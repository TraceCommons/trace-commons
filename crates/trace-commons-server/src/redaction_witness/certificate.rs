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
//! # Why there are no inference fields, and why they are not coming back
//!
//! An earlier shape of this certificate carried `chat_id`, `prompt_tokens`,
//! `completion_tokens` and `model` -- what the upstream inference cost and
//! which model served it. They are gone, deliberately, and this note exists
//! so that they are not re-added by someone reading the gap as an oversight.
//!
//! **No trace population in this repo can fill them honestly.** A CLI
//! transcript is a local agent session with no upstream receipt at all.
//! IronWire's ledger records a routing decision, not an inference receipt.
//! The witness in this design never sees the inference: it is handed a raw
//! transcript and performs the redaction. So every one of those fields could
//! only ever have been passed through from whatever the caller typed, signed
//! by an enclave, and thereby made to *look* attested. A field that no honest
//! path can fill is an invitation to fill it dishonestly.
//!
//! **They are not returning as optional fields here.** IronWire now records a
//! provider response id (`nearai/ironwire#19`), so a future IronWire-sourced
//! trace could genuinely carry a receipt, and the temptation will be to bolt
//! four `Option`s onto [`CertificateDetails`]. That would be wrong three ways.
//! An optional field in a length-prefixed signed encoding still has to be
//! encoded when absent, or the encoding stops being injective -- so "optional"
//! buys nothing on the wire and costs an absent/present discriminant that every
//! verifier must get identically right. Two field sets under one domain string
//! is exactly the confusion `SIGNING_DOMAIN` exists to prevent. And a
//! certificate that sometimes attests to inference and sometimes does not is
//! one a consumer must branch on, which is where "verified" quietly becomes
//! "verified something".
//!
//! If a receipt-bearing population does arrive, the shape to build is a
//! separate certificate profile with its own domain string -- a
//! `redaction_witness_certificate.v2`, or a distinct receipt-binding
//! structure -- chosen deliberately, by a witness that actually holds the
//! receipt it binds. Not fields grafted onto this one.
//!
//! # Logging
//!
//! Nothing here logs. With the inference fields gone, no field on this type
//! identifies a contributor or an upstream conversation: a digest, a coarse
//! three-valued risk verdict, a policy alias, an image measurement and a
//! timestamp. `Debug` therefore renders all of them. It stays hand-written
//! rather than derived so that adding an identifying field is a visible
//! decision here rather than a silent widening of every `?cert` in a log.

use trace_commons_protocol::trace_contribution::ResidualPiiRisk;

use super::correspondence::CorrespondenceProof;
use crate::near_attestation::receipt::{ReceiptError, decode_address, recover_eip191_signer};

/// The verdict's fixed-width tag in [`WitnessCertificate::signing_bytes`].
///
/// These values are assigned permanently. Changing one silently invalidates
/// every certificate ever issued -- and, worse, would let a Medium certificate
/// re-verify as Low if two assignments ever swapped. The match is exhaustive
/// rather than a cast, so a new [`ResidualPiiRisk`] variant is a compile error
/// here and gets a deliberate tag rather than inheriting a discriminant.
fn residual_risk_tag(verdict: ResidualPiiRisk) -> u8 {
    match verdict {
        ResidualPiiRisk::Low => 1,
        ResidualPiiRisk::Medium => 2,
        ResidualPiiRisk::High => 3,
    }
}

/// Domain separation. A signature over these bytes cannot be replayed as a
/// signature over any other length-prefixed structure in this workspace, and
/// a future `.v2` layout cannot be confused with this one.
const SIGNING_DOMAIN: &[u8] = b"trace_commons.redaction_witness_certificate.v1\n";

/// What the witness reports alongside the artifact digest.
///
/// Every one of these is witness self-report. Public fields are right for
/// this type: there is nothing here a witness could not have typed, and
/// pretending otherwise would be decoration.
///
/// The one field that is *not* here is `redacted_sha256`, which comes only
/// from a [`CorrespondenceProof`]. That asymmetry is the point of the type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertificateDetails {
    /// The residual-PII verdict the witness reached over the redacted
    /// artifact, using `residual_risk` from `trace-commons-protocol` -- the
    /// same function ingest runs. This is the field the design turns on: it
    /// is what lets the server trust a trace instead of re-running its own
    /// PII backstop, and it is worth exactly as much as the measurement that
    /// says which program computed it.
    ///
    /// Typed rather than a free string on purpose. A `String` here would let
    /// a witness sign "clean", or "low ", or any spelling a server's
    /// comparison happened not to expect; the closed set cannot.
    pub residual_risk_verdict: ResidualPiiRisk,
    /// The redaction policy the witness applied. **An alias, never an
    /// authority**: `redaction_pipeline_version()` concatenates hardcoded
    /// constants selected by backend family, so every self-hosted deployment
    /// reports the same string regardless of which checkpoint loaded or which
    /// detector regexes shipped. The server checks this against an allowlist
    /// and trusts `witness_measurement`.
    pub redaction_policy_version: String,
    /// The witness enclave's measurement, as the witness reports it. The
    /// operator's pinned value is a *parameter* elsewhere, never a field
    /// here -- this is what the certificate claims, and pinning is the
    /// server's separate check against it.
    pub witness_measurement: String,
    /// Unix seconds at issue.
    pub timestamp: i64,
}

/// What a witness signs on a successful correspondence check.
///
/// Every field is bound by the signature. The server verifies the signature
/// against the witness's attested signing address, then checks
/// `redacted_sha256` against the bytes it actually holds; a certificate is
/// therefore useless on any other artifact.
///
/// # Construction
///
/// The fields are private and there is exactly one production constructor,
/// [`Self::from_proof`], which consumes a [`CorrespondenceProof`] by value.
/// A public-field record would have made that constructor decoration: anybody
/// could write a struct literal with a digest they typed in, and the
/// correspondence check -- the strongest link in this chain -- would have sat
/// next to the weakest with nothing requiring it. Private fields are what
/// make holding a proof a *requirement* rather than a convention.
///
/// The wire type's `into_certificate` will be the second path when the
/// witness service slice lands. There is deliberately no third.
#[derive(Clone, PartialEq, Eq)]
pub struct WitnessCertificate {
    /// Lowercase hex SHA-256 of the redacted artifact, as carried by
    /// `CorrespondenceProof`.
    redacted_sha256: String,
    residual_risk_verdict: ResidualPiiRisk,
    redaction_policy_version: String,
    witness_measurement: String,
    timestamp: i64,
}

impl std::fmt::Debug for WitnessCertificate {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Every field, because no field on this type identifies a contributor
        // or an upstream conversation -- see the module's Logging note. Kept
        // hand-written rather than derived so that a field which *does*
        // identify one cannot start rendering by accident.
        formatter
            .debug_struct("WitnessCertificate")
            .field("redacted_sha256", &self.redacted_sha256)
            .field("residual_risk_verdict", &self.residual_risk_verdict)
            .field("redaction_policy_version", &self.redaction_policy_version)
            .field("witness_measurement", &self.witness_measurement)
            .field("timestamp", &self.timestamp)
            .finish()
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
    /// Build a certificate from a correspondence proof and what the witness
    /// reports.
    ///
    /// The proof is consumed by value, so `redacted_sha256` came out of
    /// [`check_correspondence`](super::correspondence::check_correspondence)
    /// rather than being typed by the caller. Everything else is witness
    /// self-report and is passed in as such.
    ///
    /// # What the proof attests, exactly
    ///
    /// That the digest is of bytes a correspondence check ran over, and that
    /// those bytes are the raw text with the caller's spans applied. Nothing
    /// more. In particular it does **not** attest that:
    ///
    /// - the redaction was non-trivial -- `check_correspondence(x, x, &[])`
    ///   succeeds, an empty span list being legal, so anyone holding the
    ///   artifact can mint a proof over it in one line;
    /// - the raw text was ever seen by anything but the caller. That the
    ///   caller is an attested enclave holding bytes the contributor sent is
    ///   a deployment property, not a type-system one;
    /// - this certificate is the only one for that check. `CorrespondenceProof`
    ///   is not `Clone`, so one proof yields one certificate, but a caller can
    ///   re-run the check freely.
    ///
    /// The private fields stop a *typed* digest. They do not make the digest
    /// evidence of a witnessed redaction, and a service binding certificates
    /// to receipts must not treat them as such.
    ///
    /// The digest is over the exact bytes `check_correspondence` compared. A
    /// server verifying this certificate hashes the artifact bytes it holds,
    /// so the witness must be handed the artifact byte for byte: any
    /// re-encoding, wrapper, or added trailing newline between the two
    /// produces a different digest and fails closed at
    /// [`verify_witness_certificate`](super::verification::verify_witness_certificate).
    pub fn from_proof(proof: CorrespondenceProof, details: CertificateDetails) -> Self {
        WitnessCertificate {
            redacted_sha256: proof.redacted_sha256().to_string(),
            residual_risk_verdict: details.residual_risk_verdict,
            redaction_policy_version: details.redaction_policy_version,
            witness_measurement: details.witness_measurement,
            timestamp: details.timestamp,
        }
    }

    /// Rebuild a certificate that arrived over the wire, from fields a
    /// decoder parsed one by one.
    ///
    /// **This is the second construction path the type doc promises, and it
    /// is the last one.** It exists because the receiving side has no proof
    /// and cannot have one: a server holding an artifact and a claimed digest
    /// has, by construction, only what the sender said. Anything else it
    /// could do here would be theatre.
    ///
    /// # Why a typed digest is harmless here, unlike in [`Self::from_proof`]
    ///
    /// Private fields exist so that nobody on the *issuing* side can mint a
    /// certificate over a digest they typed instead of one a correspondence
    /// check produced. That argument does not transfer to the receiving side,
    /// where the certificate is untrusted input by definition. A certificate
    /// built here is worth exactly nothing until
    /// [`verify_witness_certificate`](super::verification::verify_witness_certificate)
    /// has recovered the pinned signer from a signature over
    /// [`Self::signing_bytes`] -- which covers every field, digest included --
    /// pinned the measurement, and matched the digest against the bytes the
    /// server actually holds. Typing a field here only produces a certificate
    /// that fails all three.
    ///
    /// The caller is `redaction_witness::request::witness_headers`, and it
    /// must stay the only one: a second decoder is a second wire format, and
    /// two wire formats is how an honest certificate starts failing.
    pub fn from_wire(redacted_sha256: String, details: CertificateDetails) -> Self {
        WitnessCertificate {
            redacted_sha256,
            residual_risk_verdict: details.residual_risk_verdict,
            redaction_policy_version: details.redaction_policy_version,
            witness_measurement: details.witness_measurement,
            timestamp: details.timestamp,
        }
    }

    /// Build a certificate with an arbitrary digest, for tests that need a
    /// certificate covering bytes no proof was taken over.
    ///
    /// Deliberately not `pub`: a public test helper is a second construction
    /// path, and would reopen exactly what private fields close.
    #[cfg(test)]
    pub(crate) fn from_parts(redacted_sha256: String, details: CertificateDetails) -> Self {
        WitnessCertificate {
            redacted_sha256,
            residual_risk_verdict: details.residual_risk_verdict,
            redaction_policy_version: details.redaction_policy_version,
            witness_measurement: details.witness_measurement,
            timestamp: details.timestamp,
        }
    }

    /// The digest the certificate claims, for the module's own artifact
    /// check and for the witness service's response encoder.
    ///
    /// `pub(crate)`, not public: outside this crate the digest is only
    /// reachable through a verified certificate. The name stays "claimed"
    /// even on the issuing side, because that is what it is to everyone who
    /// receives it -- and a second accessor spelling the same field without
    /// the word would be the first step in forgetting.
    pub(crate) fn claimed_redacted_sha256(&self) -> &str {
        &self.redacted_sha256
    }

    /// The verdict the certificate binds.
    ///
    /// `pub(crate)`, and it lands with its caller as this file's standing rule
    /// requires: `WitnessResponse::residual_risk_verdict` reads it so the
    /// witness service has one source for the field rather than a copy beside
    /// the certificate. It stays crate-visible because outside this crate a
    /// verdict is only meaningful through a `VerifiedWitnessCertificate` --
    /// an unverified certificate's verdict is a claim by an unnamed program,
    /// and the accessor for the verified form belongs with the PII-backstop
    /// bypass that consumes it.
    pub(crate) fn residual_risk_verdict(&self) -> ResidualPiiRisk {
        self.residual_risk_verdict
    }

    /// The measurement the certificate claims, for the module's own pin
    /// check and for the witness service's response encoder. `pub(crate)`
    /// for the same reason as the digest above.
    pub(crate) fn claimed_witness_measurement(&self) -> &str {
        &self.witness_measurement
    }

    /// The policy alias the certificate carries.
    ///
    /// Lands with its caller, as this file's standing rule requires: the
    /// witness service's response encoder is the only reader. **An alias,
    /// never an authority** -- see the field's own documentation. A server
    /// checks it against an allowlist and trusts the measurement.
    pub(crate) fn claimed_redaction_policy_version(&self) -> &str {
        &self.redaction_policy_version
    }

    /// The timestamp the certificate carries. Witness self-report, from the
    /// witness's own clock; it orders nothing and proves nothing on its own.
    pub(crate) fn claimed_timestamp(&self) -> i64 {
        self.timestamp
    }

    /// The canonical bytes a witness signs and a verifier reconstructs.
    ///
    /// Each string field is `u64`-little-endian length followed by its UTF-8
    /// bytes; each fixed-width field follows as little-endian. The field order
    /// is fixed and the field count is fixed, so the encoding is injective:
    /// distinct certificates cannot produce identical bytes, and no content
    /// can shift across a field boundary unnoticed.
    ///
    /// The three free-form strings are encoded contiguously, ahead of the
    /// fixed-width fields, so that every adjacent length-prefixed pair is a
    /// pair whose contents an attacker could actually choose. That is what
    /// `signing_bytes_are_unambiguous_across_every_adjacent_string_pair`
    /// exercises; interleaving a closed-set field between two of them would
    /// leave the test varying a boundary it cannot move content across, which
    /// is a collision test with no collision to find.
    ///
    /// This is the single source of truth for both sides. If the witness and
    /// the verifier ever encode differently, every honest certificate fails,
    /// so there must never be a second encoder.
    pub fn signing_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(SIGNING_DOMAIN);
        for field in [
            self.redacted_sha256.as_str(),
            self.redaction_policy_version.as_str(),
            self.witness_measurement.as_str(),
        ] {
            out.extend_from_slice(&(field.len() as u64).to_le_bytes());
            out.extend_from_slice(field.as_bytes());
        }
        out.push(residual_risk_tag(self.residual_risk_verdict));
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
    /// `witness_measurement`, which is why it is `pub(super)`: a public
    /// function that performs one of three required checks and returns
    /// `Ok(())` is the "some enclave signed something" shape this module
    /// exists to make unavailable, and a doc comment saying so is not a
    /// guard. [`verify_witness_certificate`](super::verification::verify_witness_certificate)
    /// is its only caller.
    pub(super) fn verify(
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
            residual_risk_verdict: ResidualPiiRisk::Medium,
            redaction_policy_version: "policy-v3".to_string(),
            witness_measurement: "b".repeat(64),
            timestamp: 1_788_000_000,
        }
    }

    /// The three free-form string fields, **in signing order and contiguous
    /// in it**, as a mutable view. Used by the collision tests so they cover
    /// every adjacent pair rather than the one pair someone happened to pick.
    ///
    /// If a field is ever added to `signing_bytes` between two of these, this
    /// helper stops describing adjacency and the collision loops below stop
    /// testing anything -- see the note on `signing_bytes`.
    fn string_fields(cert: &mut WitnessCertificate) -> [&mut String; 3] {
        [
            &mut cert.redacted_sha256,
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
    fn from_proof_takes_its_digest_from_the_proof_and_not_from_the_caller() {
        // The only production constructor. Its point is that the digest is
        // not an argument: it comes out of check_correspondence, so a
        // certificate cannot claim a digest nobody proved.
        let redacted = "call [REDACTED:private_name] back";
        let proof = crate::redaction_witness::correspondence::check_correspondence(
            "call alice back",
            redacted,
            &[super::super::correspondence::RedactionSpan {
                start: 5,
                end: 10,
                replacement: "[REDACTED:private_name]".to_string(),
            }],
        )
        .expect("the artifact corresponds");
        let expected = proof.redacted_sha256().to_string();

        let cert = WitnessCertificate::from_proof(
            proof,
            CertificateDetails {
                residual_risk_verdict: ResidualPiiRisk::Medium,
                redaction_policy_version: "policy-v3".to_string(),
                witness_measurement: "b".repeat(64),
                timestamp: 1_788_000_000,
            },
        );

        assert_eq!(cert.redacted_sha256, expected);
        assert_eq!(
            cert.redacted_sha256,
            hex::encode(sha2::Sha256::digest(redacted.as_bytes())),
            "the digest is over the redacted bytes exactly"
        );
    }

    #[test]
    fn a_certificate_binds_the_residual_risk_verdict() {
        // The field the whole design turns on. If it is not in the signing
        // bytes, a contributor holding a genuine certificate over a High
        // artifact can present it as Low and the server's own backstop
        // bypass reads it. Every ordered pair of distinct verdicts, so the
        // test cannot pass because one particular pair happened to differ.
        for (left_verdict, right_verdict) in [
            (ResidualPiiRisk::Low, ResidualPiiRisk::Medium),
            (ResidualPiiRisk::Low, ResidualPiiRisk::High),
            (ResidualPiiRisk::Medium, ResidualPiiRisk::High),
        ] {
            let mut left = certificate();
            left.residual_risk_verdict = left_verdict;
            let mut right = certificate();
            right.residual_risk_verdict = right_verdict;
            assert_ne!(
                left.signing_bytes(),
                right.signing_bytes(),
                "{left_verdict:?} and {right_verdict:?} sign identically"
            );
        }
    }

    #[test]
    fn a_tampered_residual_risk_verdict_does_not_verify() {
        // The binding test above proves the bytes differ. This one proves
        // the signature notices, which is the property an operator relies
        // on: downgrading the verdict on a signed certificate must be
        // indistinguishable from forging one.
        let k = key("witness");
        let signature = sign(&k, &certificate());
        let mut downgraded = certificate();
        downgraded.residual_risk_verdict = ResidualPiiRisk::Low;
        assert_ne!(downgraded, certificate(), "the mutation was a no-op");
        assert_eq!(
            downgraded.verify(&signature, &address_of_key(&k)),
            Err(CertificateError::SignerMismatch)
        );
    }

    #[test]
    fn signing_bytes_are_unambiguous_across_every_adjacent_string_pair() {
        // The classic length-prefix failure: without prefixes, moving a
        // character from the start of one field to the end of the previous
        // one leaves the concatenation unchanged. ("ab", "c") and ("a", "bc")
        // sign identically, and nothing else in this suite would notice --
        // every round-trip and every signature test passes either way.
        //
        // Three free-form string fields, contiguous in the encoding, so two
        // adjacent pairs: (redacted_sha256, redaction_policy_version) and
        // (redaction_policy_version, witness_measurement). Checking one pair
        // would test that pair; checking both tests the encoder.
        for pair in 0..2 {
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
        for pair in 0..2 {
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
                "residual_risk_verdict",
                Box::new(|c: &mut WitnessCertificate| {
                    c.residual_risk_verdict = ResidualPiiRisk::Low
                }),
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
    fn debug_renders_every_field_and_names_none_that_identifies_a_contributor() {
        // The inference fields this type used to carry -- chat_id, model and
        // the token counts -- were the reason Debug withheld anything. With
        // them gone, every remaining field is safe to log, and the assertion
        // worth making is the opposite one: that the hand-written impl still
        // renders all of them, so `?cert` in a diagnostic is actually useful.
        let cert = certificate();
        let rendered = format!("{cert:?}");
        for field in [
            cert.redacted_sha256.as_str(),
            cert.redaction_policy_version.as_str(),
            cert.witness_measurement.as_str(),
            "Medium",
            "1788000000",
        ] {
            assert!(
                rendered.contains(field),
                "Debug omitted {field}: {rendered}"
            );
        }
        // No `..` -- a field added to the struct and forgotten here would
        // otherwise render as an ellipsis and read as deliberate.
        assert!(
            !rendered.contains(".."),
            "Debug is non-exhaustive: {rendered}"
        );
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
