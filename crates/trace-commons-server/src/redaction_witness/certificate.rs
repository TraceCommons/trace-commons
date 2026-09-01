// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The witness certificate: what the witness signs once the correspondence
//! check passes, and how the server verifies it.
//!
//! The certificate is over the **redacted** artifact. That is the whole point
//! of it -- the server holds the redacted bytes and never the raw ones, so a
//! statement it can check must be a statement about what it holds. It says
//! that a witness with a given measurement saw raw bytes and this artifact,
//! and that the artifact derives from those bytes by redaction alone.
//!
//! It does **not** say the redaction was sufficient. A submission that redacts
//! nothing corresponds perfectly to its raw text and gets a valid certificate.
//! Sufficiency is the redaction policy's job and the PII backstop's, and no
//! surface may read a valid certificate as "verified clean".
//!
//! # Canonical bytes are length-prefixed, never JSON
//!
//! [`WitnessCertificate::signing_bytes`] is the only encoder, and it is a
//! length-prefixed encoding in the shape of
//! `instance_enroll_attestation_signing_bytes` in `trace-commons-protocol`.
//! Serializing this struct to JSON and hashing that would be wrong twice
//! over: `serde_json`'s map ordering is not guaranteed, and a dependency
//! enabling `serde_json/preserve_order` in this workspace on 2026-09-01
//! shifted every digest in it that was taken over untyped JSON. There is
//! deliberately no `Serialize` on this type, so that no JSON form of it can
//! drift into being treated as the signing preimage.
//!
//! Length prefixes are what make the encoding injective. Concatenating fields
//! directly would let content shift across a field boundary without changing
//! the bytes -- `("ab", "c")` and `("a", "bc")` would sign identically -- and
//! that collision is invisible to every round-trip and every signature test
//! that does not construct it on purpose. `signing_bytes_are_unambiguous`
//! constructs it.
//!
//! # Signatures
//!
//! secp256k1/ECDSA under EIP-191, recovered to an Ethereum-style address, the
//! same scheme and the same code path as a NEAR AI inference receipt
//! ([`crate::near_attestation::receipt`]). dstack gives an enclave a per-app
//! secp256k1 identity and publishes its address in the attestation report, so
//! verifying a certificate is the same operation as verifying a receipt, over
//! a different message.
//!
//! # Logging
//!
//! Nothing here logs. A certificate carries `chat_id`, which identifies an
//! upstream conversation, so `Debug` is hand-written to withhold it; only the
//! two digests render.

use crate::near_attestation::receipt::{ReceiptError, decode_address, recover_eip191_signer};

/// Domain separation. A signature over these bytes cannot be replayed as a
/// signature over any other length-prefixed structure in this workspace, and
/// a future `.v2` layout cannot be confused with this one.
const SIGNING_DOMAIN: &[u8] = b"trace_commons.redaction_witness_certificate.v1\n";

/// What a witness signs on a successful correspondence check.
///
/// Every field is bound by the signature. The server verifies the signature
/// against the witness's attested signing address, then checks
/// `redacted_sha256` against the bytes it actually holds; a certificate is
/// therefore useless on any other artifact.
///
/// Construct with public fields -- this is a record, not an invariant-bearing
/// type. The invariant that matters lives in [`Self::signing_bytes`].
#[derive(Clone, PartialEq, Eq)]
pub struct WitnessCertificate {
    /// Lowercase hex SHA-256 of the redacted artifact, as carried by
    /// `CorrespondenceProof`.
    pub redacted_sha256: String,
    /// The upstream inference conversation this trace came from.
    pub chat_id: String,
    /// Tokens the upstream inference billed.
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    /// The upstream model slug.
    pub model: String,
    /// Unix seconds at issue.
    pub timestamp: i64,
    /// The redaction policy the client applied. A version, not a verdict:
    /// the witness checks mechanics, never policy.
    pub redaction_policy_version: String,
    /// The witness enclave's measurement, as the witness reports it. The
    /// operator's pinned value is a *parameter* elsewhere, never a field
    /// here -- this is what the certificate claims, and pinning is the
    /// server's separate check against it.
    pub witness_measurement: String,
}

impl std::fmt::Debug for WitnessCertificate {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Only the two digests. `chat_id` identifies an upstream
        // conversation, and `model` and the token counts describe one
        // contributor's usage; a derived `Debug` would put all of them into
        // any `tracing` call that used `?cert`.
        formatter
            .debug_struct("WitnessCertificate")
            .field("redacted_sha256", &self.redacted_sha256)
            .field("witness_measurement", &self.witness_measurement)
            .finish_non_exhaustive()
    }
}

/// Why a certificate did not verify.
///
/// Three variants, because an operator does three different things about
/// them. There is deliberately no separate variant for "tampered field": a
/// certificate whose fields were altered after signing and a certificate
/// signed by someone else are the same observation -- the recovered signer is
/// not the witness -- and ECDSA recovery cannot tell them apart even in
/// principle. Inventing two names for one observation would claim a
/// distinction the check cannot make.
///
/// `Debug` delegates to `Display`, as in `correspondence`. No variant carries
/// a field today; the hand-written impl is what keeps that true if one ever
/// does, since `tracing::warn!(?err)` is how an error reaches a log here.
#[derive(Clone, PartialEq, Eq, thiserror::Error)]
pub enum CertificateError {
    /// The signature is not 65 bytes of hex with a usable recovery byte, or
    /// no public key recovers from it. All of these mean the same thing to a
    /// caller -- these bytes are not a signature -- and the same thing to an
    /// operator, which is that the witness response was malformed.
    #[error("the certificate signature is malformed or unrecoverable")]
    SignatureMalformed,
    /// The signature recovers cleanly, to somebody who is not the witness.
    /// Distinct from the above because it is the interesting one: a
    /// well-formed certificate from an unexpected signer means either a
    /// tampered submission or a witness key the operator has not pinned.
    #[error("the certificate was signed by a different signer than the expected witness")]
    SignerMismatch,
    /// The expected witness address is not a `0x`-prefixed 20-byte hex
    /// address. This is operator configuration, not a bad submission, and
    /// refusing every certificate would be the wrong diagnosis.
    #[error("the expected witness signing address is not a 20-byte hex address")]
    WitnessAddressMalformed,
}

impl std::fmt::Debug for CertificateError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, formatter)
    }
}

impl WitnessCertificate {
    /// The canonical bytes a witness signs and a verifier reconstructs.
    ///
    /// Each string field is `u64`-little-endian length followed by its UTF-8
    /// bytes; each integer is fixed-width little-endian. The field order is
    /// fixed and the field count is fixed, so the encoding is injective:
    /// distinct certificates cannot produce identical bytes, and no content
    /// can shift across a field boundary unnoticed.
    ///
    /// This is the single source of truth for both sides. If the witness and
    /// the verifier ever encode differently, every honest certificate fails,
    /// so there must never be a second encoder.
    pub fn signing_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(SIGNING_DOMAIN);
        for field in [
            self.redacted_sha256.as_str(),
            self.chat_id.as_str(),
            self.model.as_str(),
            self.redaction_policy_version.as_str(),
            self.witness_measurement.as_str(),
        ] {
            out.extend_from_slice(&(field.len() as u64).to_le_bytes());
            out.extend_from_slice(field.as_bytes());
        }
        out.extend_from_slice(&self.prompt_tokens.to_le_bytes());
        out.extend_from_slice(&self.completion_tokens.to_le_bytes());
        out.extend_from_slice(&self.timestamp.to_le_bytes());
        out
    }

    /// Check that `signature_hex` is this certificate, signed by
    /// `witness_signing_address`.
    ///
    /// `witness_signing_address` is the address from the witness enclave's
    /// own attestation report -- what the operator has decided to trust.
    /// Comparison is case-insensitive, because the address is hex and both
    /// EIP-55 checksummed and all-lowercase forms are in circulation.
    ///
    /// This checks the signature and nothing else. It does not compare
    /// `redacted_sha256` against any artifact and does not pin
    /// `witness_measurement`; both are the caller's separate checks, and
    /// folding them in here would let a caller believe one call had done all
    /// three.
    pub fn verify(
        &self,
        signature_hex: &str,
        witness_signing_address: &str,
    ) -> Result<(), CertificateError> {
        let expected = decode_address(witness_signing_address)
            .ok_or(CertificateError::WitnessAddressMalformed)?;
        let recovered = recover_eip191_signer(&self.signing_bytes(), signature_hex).map_err(
            |error| match error {
                ReceiptError::SignatureMalformed
                | ReceiptError::RecoveryIdUnsupported { .. }
                | ReceiptError::SignatureUnrecoverable => CertificateError::SignatureMalformed,
                // recover_eip191_signer returns no other variant; treating an
                // unexpected one as a refusal is the fail-closed reading.
                _ => CertificateError::SignatureMalformed,
            },
        )?;
        if recovered != expected {
            return Err(CertificateError::SignerMismatch);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use k256::ecdsa::SigningKey;
    use sha3::{Digest, Keccak256};

    /// A certificate with plausible, distinct values in every field, so that
    /// a field-binding test cannot pass because two fields happened to be
    /// equal.
    fn certificate() -> WitnessCertificate {
        WitnessCertificate {
            redacted_sha256: "a".repeat(64),
            chat_id: "chatcmpl-7f3a".to_string(),
            prompt_tokens: 1_204,
            completion_tokens: 337,
            model: "qwen3.6-27b-fp8".to_string(),
            timestamp: 1_788_000_000,
            redaction_policy_version: "policy-v3".to_string(),
            witness_measurement: "b".repeat(64),
        }
    }

    /// The five string fields, in signing order, as a mutable view. Used by
    /// the collision test so it covers every adjacent pair rather than the
    /// one pair someone happened to pick.
    fn string_fields(cert: &mut WitnessCertificate) -> [&mut String; 5] {
        [
            &mut cert.redacted_sha256,
            &mut cert.chat_id,
            &mut cert.model,
            &mut cert.redaction_policy_version,
            &mut cert.witness_measurement,
        ]
    }

    fn key(seed: &str) -> SigningKey {
        let bytes = Keccak256::digest(seed.as_bytes());
        SigningKey::from_slice(&bytes).expect("seed is a valid scalar")
    }

    fn address_of_key(k: &SigningKey) -> String {
        let point = k.verifying_key().to_encoded_point(false);
        let digest = Keccak256::digest(&point.as_bytes()[1..]);
        format!("0x{}", hex::encode(&digest[12..]))
    }

    /// Sign a certificate the way the witness enclave would: EIP-191 over the
    /// canonical signing bytes, 65-byte hex with a 27/28 recovery byte.
    fn sign(k: &SigningKey, cert: &WitnessCertificate) -> String {
        let message = cert.signing_bytes();
        let mut hasher = Keccak256::new();
        hasher.update(b"\x19Ethereum Signed Message:\n");
        hasher.update(message.len().to_string().as_bytes());
        hasher.update(&message);
        let digest: [u8; 32] = hasher.finalize().into();
        let (signature, recovery_id) = k.sign_prehash_recoverable(&digest).unwrap();
        let mut raw = signature.to_bytes().to_vec();
        raw.push(recovery_id.to_byte() + 27);
        format!("0x{}", hex::encode(raw))
    }

    #[test]
    fn signing_bytes_are_unambiguous_across_every_adjacent_string_pair() {
        // The classic length-prefix failure: without prefixes, moving a
        // character from the start of one field to the end of the previous
        // one leaves the concatenation unchanged. ("ab", "c") and ("a", "bc")
        // sign identically, and nothing else in this suite would notice --
        // every round-trip and every signature test passes either way.
        //
        // Five string fields, so four adjacent pairs. Checking one pair would
        // test that pair; checking all four tests the encoder.
        for pair in 0..4 {
            let mut left = certificate();
            {
                let fields = string_fields(&mut left);
                *fields[pair] = "ab".to_string();
                *fields[pair + 1] = "c".to_string();
            }
            let mut right = certificate();
            {
                let fields = string_fields(&mut right);
                *fields[pair] = "a".to_string();
                *fields[pair + 1] = "bc".to_string();
            }
            assert_ne!(
                left.signing_bytes(),
                right.signing_bytes(),
                "fields {pair} and {} collide under a field-boundary shift",
                pair + 1
            );
        }
    }

    #[test]
    fn signing_bytes_are_domain_separated_and_stable() {
        let bytes = certificate().signing_bytes();
        assert!(bytes.starts_with(SIGNING_DOMAIN));
        // Same input, same bytes: the encoder has no map iteration in it.
        assert_eq!(bytes, certificate().signing_bytes());
    }

    #[test]
    fn an_empty_string_field_is_still_length_prefixed() {
        // A zero-length field must still consume its prefix. Otherwise an
        // empty field contributes nothing at all, and a certificate whose
        // content sits in the *other* field of the pair encodes identically.
        // The adjacent-shift loop above never exercises an empty field, so
        // this case is only covered here.
        for pair in 0..4 {
            let mut left = certificate();
            {
                let fields = string_fields(&mut left);
                *fields[pair] = String::new();
                *fields[pair + 1] = "x".to_string();
            }
            let mut right = certificate();
            {
                let fields = string_fields(&mut right);
                *fields[pair] = "x".to_string();
                *fields[pair + 1] = String::new();
            }
            assert_ne!(
                left.signing_bytes(),
                right.signing_bytes(),
                "an empty field {pair} is indistinguishable from an empty field {}",
                pair + 1
            );
        }
    }

    #[test]
    fn a_valid_certificate_verifies() {
        let k = key("witness");
        let cert = certificate();
        let signature = sign(&k, &cert);
        assert_eq!(cert.verify(&signature, &address_of_key(&k)), Ok(()));
    }

    #[test]
    fn the_expected_address_is_compared_case_insensitively() {
        // Checksummed and all-lowercase addresses are both in circulation;
        // an operator pasting the checksummed form from an attestation
        // report must not silently refuse every certificate.
        let k = key("witness");
        let cert = certificate();
        let signature = sign(&k, &cert);
        let shouty = address_of_key(&k).to_uppercase().replace("0X", "0x");
        assert_eq!(cert.verify(&signature, &shouty), Ok(()));
    }

    #[test]
    fn a_certificate_for_a_different_artifact_does_not_verify() {
        // The property the whole design rests on: the certificate is useless
        // on any artifact but the one it was issued over.
        let k = key("witness");
        let signed = certificate();
        let signature = sign(&k, &signed);

        let mut other = certificate();
        other.redacted_sha256 = "c".repeat(64);
        assert_eq!(
            other.verify(&signature, &address_of_key(&k)),
            Err(CertificateError::SignerMismatch)
        );
    }

    #[test]
    fn a_tampered_token_count_does_not_verify() {
        let k = key("witness");
        let signature = sign(&k, &certificate());
        let mut inflated = certificate();
        inflated.prompt_tokens += 1;
        assert_eq!(
            inflated.verify(&signature, &address_of_key(&k)),
            Err(CertificateError::SignerMismatch)
        );
    }

    #[test]
    fn every_field_is_bound_by_the_signature() {
        // One mutation per field. A field left out of signing_bytes is free
        // for a contributor to rewrite after the fact, and the only test that
        // catches it is one that mutates that specific field.
        let k = key("witness");
        let address = address_of_key(&k);
        let signature = sign(&k, &certificate());

        let mutations: Vec<(&str, Box<dyn Fn(&mut WitnessCertificate)>)> = vec![
            (
                "redacted_sha256",
                Box::new(|c: &mut WitnessCertificate| c.redacted_sha256 = "d".repeat(64)),
            ),
            (
                "chat_id",
                Box::new(|c: &mut WitnessCertificate| c.chat_id = "chatcmpl-other".to_string()),
            ),
            (
                "prompt_tokens",
                Box::new(|c: &mut WitnessCertificate| c.prompt_tokens = 1),
            ),
            (
                "completion_tokens",
                Box::new(|c: &mut WitnessCertificate| c.completion_tokens = 1),
            ),
            (
                "model",
                Box::new(|c: &mut WitnessCertificate| c.model = "some-other-model".to_string()),
            ),
            (
                "timestamp",
                Box::new(|c: &mut WitnessCertificate| c.timestamp += 1),
            ),
            (
                "redaction_policy_version",
                Box::new(|c: &mut WitnessCertificate| {
                    c.redaction_policy_version = "policy-v4".to_string()
                }),
            ),
            (
                "witness_measurement",
                Box::new(|c: &mut WitnessCertificate| c.witness_measurement = "e".repeat(64)),
            ),
        ];

        for (field, mutate) in mutations {
            let mut tampered = certificate();
            mutate(&mut tampered);
            assert_ne!(tampered, certificate(), "{field} mutation was a no-op");
            assert_eq!(
                tampered.verify(&signature, &address),
                Err(CertificateError::SignerMismatch),
                "{field} is not bound by the signature"
            );
        }
    }

    #[test]
    fn a_signature_by_a_different_key_is_refused() {
        let witness = key("witness");
        let impostor = key("impostor");
        assert_ne!(address_of_key(&witness), address_of_key(&impostor));

        let cert = certificate();
        let signature = sign(&impostor, &cert);
        assert_eq!(
            cert.verify(&signature, &address_of_key(&witness)),
            Err(CertificateError::SignerMismatch)
        );
    }

    #[test]
    fn a_signature_that_is_not_a_signature_is_refused() {
        let address = address_of_key(&key("witness"));
        let cert = certificate();
        for bad in [
            "0x",
            "not-hex",
            // 64 bytes: a well-formed ECDSA signature with no recovery byte.
            &format!("0x{}", "11".repeat(64)),
            // 66 bytes.
            &format!("0x{}", "11".repeat(66)),
            // 65 bytes whose recovery byte is neither 0/1 nor 27/28.
            &format!("0x{}{}", "11".repeat(64), "07"),
        ] {
            assert_eq!(
                cert.verify(bad, &address),
                Err(CertificateError::SignatureMalformed),
                "{bad} was not refused as malformed"
            );
        }
    }

    #[test]
    fn a_malformed_expected_address_is_its_own_refusal() {
        // Not SignatureMalformed and not SignerMismatch: this is operator
        // configuration, and reporting it as a bad submission would send an
        // operator looking at the contributor instead of at their own pin.
        let k = key("witness");
        let cert = certificate();
        let signature = sign(&k, &cert);
        for bad in [
            "",
            "0x",
            "deadbeef",
            // 40 characters, so the length check passes and only the hex
            // decode refuses it.
            &format!("0x{}", "zz".repeat(20)),
            &format!("0x{}", "ab".repeat(19)),
        ] {
            assert_eq!(
                cert.verify(&signature, bad),
                Err(CertificateError::WitnessAddressMalformed),
                "{bad} was not refused as a malformed address"
            );
        }
    }

    #[test]
    fn neither_formatter_renders_certificate_identifiers() {
        // `?cert` and `%cert` must be equally safe: a derived Debug prints
        // every field, and `?` is how a value ordinarily reaches a log here.
        let cert = certificate();
        let rendered = format!("{cert:?}");
        for secret in [cert.chat_id.as_str(), cert.model.as_str()] {
            assert!(
                !rendered.contains(secret),
                "Debug rendered {secret}: {rendered}"
            );
        }
        assert!(!rendered.contains("1204"));
        assert!(!rendered.contains("337"));
        assert!(rendered.contains(&cert.redacted_sha256));
        assert!(rendered.contains(&cert.witness_measurement));
    }

    #[test]
    fn both_error_formatters_render_the_same_safe_text() {
        for error in [
            CertificateError::SignatureMalformed,
            CertificateError::SignerMismatch,
            CertificateError::WitnessAddressMalformed,
        ] {
            let display = format!("{error}");
            let debug = format!("{error:?}");
            assert_eq!(display, debug);
            assert!(!display.is_empty());
            // No variant name leaks a hex blob or an address today, and the
            // hand-written Debug is what keeps that true if one gains a field.
            assert!(!display.contains("0x"));
        }
    }
}
