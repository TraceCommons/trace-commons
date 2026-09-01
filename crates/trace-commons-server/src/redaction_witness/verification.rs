// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Server-side verification of a witness certificate.
//!
//! Three facts have to hold before a certificate says anything useful about
//! an artifact the server holds:
//!
//! 1. the signature recovers to the witness signing address the operator
//!    pinned;
//! 2. `witness_measurement` is one the operator pinned;
//! 3. `redacted_sha256` is the digest of the bytes actually on hand.
//!
//! They run in that order, and a test pins it.
//!
//! [`WitnessCertificate::verify`] checks only the first. A caller who ran it
//! and stopped there would have established that *some* enclave signed
//! *something* -- not that this certificate covers this artifact, and not
//! that the enclave running the witness is one anybody vouched for.
//!
//! So this module does not offer three checks. It offers one entry point,
//! [`verify_witness_certificate`], which takes every input the three checks
//! need in a single call, consumes the certificate, and returns a
//! [`VerifiedWitnessCertificate`] that has no other constructor. A partial
//! verification is not discouraged here, it is unspeakable: there is no way
//! to obtain the verified type without having passed all three, and the
//! fields a policy would want to act on -- the token counts, the chat id --
//! are reachable only through it.
//!
//! # Fail closed on the measurement
//!
//! `pin: None` is a refusal naming a missing control, never a pass. This is
//! the shape [`crate::near_attestation::measurements::check_measurements_opt`]
//! established: an operator who has pinned nothing gets told which control is
//! missing, because the alternative is a green tick that means nothing.
//!
//! The pin is a single [`WitnessPin`] holding both the signing address and
//! the measurement set, and it validates on construction. That is deliberate:
//! an address without a measurement set is exactly the half-configured state
//! that produces a confident, worthless verification, and bundling them means
//! it cannot be expressed. It also means a malformed address is a
//! configuration error raised where configuration is loaded, rather than a
//! per-certificate verification failure that looks like a bad submission.
//!
//! # Logging
//!
//! Nothing here logs, and [`WitnessVerificationError`] is safe under both
//! formatters. The one error that carries a value carries a measurement --
//! a public image identifier, and one that has already been proven to come
//! from the pinned signer, because the signature is checked first.

use std::collections::BTreeSet;

use sha2::{Digest, Sha256};

use super::certificate::{CertificateError, WitnessCertificate};
use crate::near_attestation::receipt::decode_address;

/// Missing-control name reported when the operator has pinned no witness.
pub const EXPECTED_MEASUREMENT_CONTROL: &str = "witness_expected_measurement";

/// Why a witness pin could not be loaded.
///
/// Every variant is a refusal to construct. There is no partially valid pin:
/// a `WitnessPin` that exists is one that names a well-formed signing address
/// and at least one measurement.
#[derive(Clone, PartialEq, Eq, thiserror::Error)]
pub enum WitnessPinError {
    /// The signing address is not a `0x`-prefixed 20-byte hex address.
    #[error("the pinned witness signing address is not a 20-byte hex address")]
    SigningAddressMalformed,
    /// The pin named no measurements. An empty set would admit every enclave
    /// while reading, in config, as a control that is present.
    #[error("the witness pin named no expected measurements")]
    NoMeasurements,
    /// A named measurement is blank. A blank pin cannot match any honest
    /// certificate and would silently weaken the set it sits in.
    #[error("the witness pin contains a blank expected measurement")]
    MeasurementBlank,
}

impl std::fmt::Debug for WitnessPinError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, formatter)
    }
}

/// What the operator has decided to trust: one witness signing address, and
/// the measurements a witness enclave is allowed to report.
///
/// Both halves are here because neither is worth anything alone. A signing
/// address with nothing pinned admits any image that holds the key; a
/// measurement set with no address admits any signer.
///
/// Values are operator configuration -- an Ethereum-style address and public
/// image identifiers -- so `Debug` is derived. No contributor content reaches
/// this type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WitnessPin {
    signing_address: String,
    measurements: BTreeSet<String>,
}

impl WitnessPin {
    /// Validate and build a pin.
    ///
    /// Measurements are compared exactly, byte for byte. They are opaque
    /// identifiers a witness reports rather than values with two circulating
    /// spellings, so a case-folding comparison could only conflate two
    /// distinct pins; a case difference against an honest witness fails
    /// closed and is diagnosable from the reported value.
    pub fn new(
        signing_address: &str,
        measurements: impl IntoIterator<Item = String>,
    ) -> Result<Self, WitnessPinError> {
        if decode_address(signing_address).is_none() {
            return Err(WitnessPinError::SigningAddressMalformed);
        }
        let mut pinned = BTreeSet::new();
        for measurement in measurements {
            if measurement.trim().is_empty() {
                return Err(WitnessPinError::MeasurementBlank);
            }
            pinned.insert(measurement);
        }
        if pinned.is_empty() {
            return Err(WitnessPinError::NoMeasurements);
        }
        Ok(WitnessPin {
            signing_address: signing_address.to_string(),
            measurements: pinned,
        })
    }

    /// How many distinct measurements this pin admits. A caller reporting the
    /// strength of the check wants this; nothing else does.
    pub fn pinned_measurement_count(&self) -> usize {
        self.measurements.len()
    }
}

/// Why a certificate did not verify against an artifact.
///
/// Four variants, because an operator does four different things about them.
/// A signature failure means the certificate is not ours. An artifact
/// mismatch means the bytes we hold are not the bytes that were certified. A
/// measurement that is not pinned means an enclave nobody vouched for signed
/// it. Nothing pinned at all means the operator's *own configuration* is
/// missing -- and reporting that as any of the other three sends them to
/// inspect a contributor instead of their config.
///
/// [`WitnessVerificationError::Unpinned`] and
/// [`WitnessVerificationError::MeasurementNotPinned`] stay separate for the
/// same reason: "you have pinned nothing" and "this enclave is not in your
/// set" are different states of the operator's config with different fixes.
///
/// `Debug` delegates to `Display`, as everywhere else in this module, because
/// `tracing::warn!(?err)` is how an error ordinarily reaches a log here.
#[derive(Clone, PartialEq, Eq, thiserror::Error)]
pub enum WitnessVerificationError {
    /// No pin is configured, so nothing about the witness could be checked.
    /// A refusal, never a skip.
    #[error("witness verification refused: missing control {control}")]
    Unpinned { control: &'static str },
    /// The signature is malformed, or recovers to somebody who is not the
    /// pinned witness.
    #[error("the witness certificate signature did not verify: {0}")]
    Signature(#[source] CertificateError),
    /// The certificate's `redacted_sha256` is not the digest of the bytes the
    /// server holds. The certificate is genuine and covers a different
    /// artifact.
    ///
    /// Carries neither digest. The caller refuses either way, and the held
    /// artifact's digest is a handle on contributor content that the module's
    /// hash-only discipline keeps out of error text.
    #[error("the witness certificate does not cover the artifact on hand")]
    ArtifactMismatch,
    /// The certificate is genuine, and reports a measurement the operator has
    /// not pinned.
    ///
    /// Carries the reported measurement, which is a public image identifier
    /// and the one value an operator needs to decide whether to widen the pin
    /// or investigate. It is safe to render because the signature has already
    /// been checked against the pinned address: an unsigned certificate never
    /// reaches this variant.
    #[error("the witness reported measurement {reported}, which is not pinned")]
    MeasurementNotPinned { reported: String },
}

impl std::fmt::Debug for WitnessVerificationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, formatter)
    }
}

/// A certificate that passed all three checks against a specific artifact.
///
/// There is no public constructor and no public field. The only way to hold
/// one is to have called [`verify_witness_certificate`] and had it succeed,
/// which is what makes a half-done verification impossible to express: code
/// downstream that wants the token counts or the chat id must take this type,
/// and a bare [`WitnessCertificate`] will not do.
///
/// `Debug` renders the two digests only, as on [`WitnessCertificate`].
#[derive(Clone, PartialEq, Eq)]
pub struct VerifiedWitnessCertificate {
    certificate: WitnessCertificate,
}

impl std::fmt::Debug for VerifiedWitnessCertificate {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Delegates to the certificate's own hand-written Debug, which
        // withholds `chat_id`, `model` and the token counts. Verification
        // does not make those safe to log.
        formatter
            .debug_struct("VerifiedWitnessCertificate")
            .field("certificate", &self.certificate)
            .finish()
    }
}

impl VerifiedWitnessCertificate {
    /// Lowercase hex SHA-256 of the artifact this certificate covers, which
    /// verification has proven is the artifact the caller passed.
    pub fn redacted_sha256(&self) -> &str {
        self.certificate.claimed_redacted_sha256()
    }

    /// The measurement the witness reported, which verification has proven is
    /// one the operator pinned. A caller reporting the strength of the check
    /// wants this.
    pub fn witness_measurement(&self) -> &str {
        self.certificate.claimed_witness_measurement()
    }
}

// There are deliberately no accessors for `chat_id`, `model`, the token
// counts or the timestamp. Nothing consumes them yet, and adding a getter per
// field would hand back exactly the unverified-read surface that making the
// certificate's fields private just closed. They are contributor-linked, and
// the moment something legitimately needs one is the moment to add it -- with
// a caller in the same commit.
//
// `timestamp` in particular is bound by the signature and *not* checked for
// freshness: a certificate has no nonce and is replayable against the same
// artifact by anyone holding it, so any freshness window is a policy decision
// above this module.

/// Verify a witness certificate against the artifact the server holds.
///
/// All three checks or none: the signature against the pinned address, the
/// certificate's digest against `redacted_bytes`, and the reported
/// measurement against the pinned set. There is no way to run a subset, and
/// the successful return value cannot be produced any other way.
///
/// The order is fail-closed first. `pin: None` refuses before anything is
/// examined, and the signature is checked before any certificate field is
/// read into an error, so no unsigned attacker-chosen string can reach an
/// operator surface through a refusal.
///
/// A successful return does **not** mean the artifact is clean. The
/// certificate attests that the artifact derives from raw bytes by redaction
/// alone; sufficiency of that redaction is the policy's job and the PII
/// backstop's.
pub fn verify_witness_certificate(
    certificate: WitnessCertificate,
    signature_hex: &str,
    pin: Option<&WitnessPin>,
    redacted_bytes: &[u8],
) -> Result<VerifiedWitnessCertificate, WitnessVerificationError> {
    let Some(pin) = pin else {
        return Err(WitnessVerificationError::Unpinned {
            control: EXPECTED_MEASUREMENT_CONTROL,
        });
    };

    certificate
        .verify(signature_hex, &pin.signing_address)
        .map_err(WitnessVerificationError::Signature)?;

    if !pin
        .measurements
        .contains(certificate.claimed_witness_measurement())
    {
        return Err(WitnessVerificationError::MeasurementNotPinned {
            reported: certificate.claimed_witness_measurement().to_string(),
        });
    }

    // Hex case is not a difference in digest, and both spellings circulate.
    // The comparison is otherwise exact: a shorter certificate digest never
    // matches by prefix.
    let held = hex::encode(Sha256::digest(redacted_bytes));
    if !certificate
        .claimed_redacted_sha256()
        .eq_ignore_ascii_case(&held)
    {
        return Err(WitnessVerificationError::ArtifactMismatch);
    }

    Ok(VerifiedWitnessCertificate { certificate })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::redaction_witness::certificate::CertificateDetails;
    use k256::ecdsa::SigningKey;
    use sha3::Keccak256;

    /// The artifact bytes every test verifies against, unless it is about a
    /// mismatch.
    const ARTIFACT: &[u8] = b"turn 1: hello\nturn 2: my card is [REDACTED:private_card]\n";

    /// The measurement the pin admits.
    const PINNED_MEASUREMENT: &str =
        "c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1";

    fn digest_of(bytes: &[u8]) -> String {
        hex::encode(Sha256::digest(bytes))
    }

    /// A certificate that covers [`ARTIFACT`] and reports the pinned
    /// measurement, with a distinctive value in every other field so that a
    /// formatter test cannot pass because two fields happened to be equal.
    fn details() -> CertificateDetails {
        CertificateDetails {
            chat_id: "chatcmpl-secret-session".to_string(),
            prompt_tokens: 1_204,
            completion_tokens: 337,
            model: "qwen3.6-27b-fp8".to_string(),
            timestamp: 1_788_000_000,
            redaction_policy_version: "policy-v3".to_string(),
            witness_measurement: PINNED_MEASUREMENT.to_string(),
        }
    }

    fn certificate() -> WitnessCertificate {
        WitnessCertificate::from_parts(digest_of(ARTIFACT), details())
    }

    /// A certificate covering [`ARTIFACT`] but reporting `measurement`.
    fn certificate_reporting(measurement: &str) -> WitnessCertificate {
        WitnessCertificate::from_parts(
            digest_of(ARTIFACT),
            CertificateDetails {
                witness_measurement: measurement.to_string(),
                ..details()
            },
        )
    }

    /// A certificate claiming `digest` and reporting the pinned measurement.
    fn certificate_claiming(digest: String) -> WitnessCertificate {
        WitnessCertificate::from_parts(digest, details())
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

    /// Sign as the witness enclave would: EIP-191 over the canonical signing
    /// bytes, 65-byte hex with a 27/28 recovery byte.
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

    /// The witness key, and a pin naming its address and the one measurement.
    fn witness() -> (SigningKey, WitnessPin) {
        let k = key("witness enclave signing key");
        let pin = WitnessPin::new(&address_of_key(&k), [PINNED_MEASUREMENT.to_string()])
            .expect("pin is well formed");
        (k, pin)
    }

    #[test]
    fn a_complete_verification_returns_the_certificate() {
        let (k, pin) = witness();
        let cert = certificate();
        let signature = sign(&k, &cert);

        let verified =
            verify_witness_certificate(cert, &signature, Some(&pin), ARTIFACT).expect("verifies");

        assert_eq!(verified.redacted_sha256(), digest_of(ARTIFACT));
        assert_eq!(verified.witness_measurement(), PINNED_MEASUREMENT);
    }

    #[test]
    fn no_pinned_measurement_refuses_rather_than_passing() {
        let (k, _) = witness();
        let cert = certificate();
        let signature = sign(&k, &cert);

        // Everything else about this call is perfect: the signature is the
        // witness's own, over a certificate that covers exactly these bytes.
        // Absence of a pin is still a refusal.
        let error = verify_witness_certificate(cert, &signature, None, ARTIFACT)
            .expect_err("an unpinned operator must not get a pass");
        assert_eq!(
            error,
            WitnessVerificationError::Unpinned {
                control: "witness_expected_measurement",
            }
        );
    }

    #[test]
    fn a_certificate_from_an_unpinned_enclave_is_refused() {
        let (k, pin) = witness();
        // Signed by the real witness key: this is a genuine certificate from
        // an enclave running an image nobody vouched for, not a tampered one.
        let cert = certificate_reporting(&"d".repeat(64));
        let signature = sign(&k, &cert);

        let error = verify_witness_certificate(cert, &signature, Some(&pin), ARTIFACT)
            .expect_err("an unpinned enclave must be refused");
        assert_eq!(
            error,
            WitnessVerificationError::MeasurementNotPinned {
                reported: "d".repeat(64),
            }
        );
    }

    #[test]
    fn a_certificate_whose_hash_does_not_match_the_held_bytes_is_refused() {
        let (k, pin) = witness();
        let cert = certificate();
        let signature = sign(&k, &cert);

        let error = verify_witness_certificate(
            cert,
            &signature,
            Some(&pin),
            b"turn 1: hello\nturn 2: my card is 4111 1111 1111 1111\n",
        )
        .expect_err("a certificate for other bytes must be refused");
        assert_eq!(error, WitnessVerificationError::ArtifactMismatch);
    }

    #[test]
    fn a_certificate_digest_that_is_a_prefix_of_the_held_digest_is_refused() {
        // Truncating the *artifact* changes the digest entirely, so it cannot
        // tell an equality comparison apart from a prefix one. Only a
        // truncated digest in the certificate can, and without this test a
        // `starts_with` comparison passes the whole suite.
        let (k, pin) = witness();
        let cert = certificate_claiming(digest_of(ARTIFACT)[..32].to_string());
        let signature = sign(&k, &cert);

        let error = verify_witness_certificate(cert, &signature, Some(&pin), ARTIFACT)
            .expect_err("a shorter digest must not match by prefix");
        assert_eq!(error, WitnessVerificationError::ArtifactMismatch);
    }

    #[test]
    fn a_truncated_artifact_is_refused() {
        let (k, pin) = witness();
        let cert = certificate();
        let signature = sign(&k, &cert);

        let error = verify_witness_certificate(
            cert,
            &signature,
            Some(&pin),
            &ARTIFACT[..ARTIFACT.len() - 1],
        )
        .expect_err("a truncated artifact must be refused");
        assert_eq!(error, WitnessVerificationError::ArtifactMismatch);
    }

    #[test]
    fn an_uppercase_digest_in_the_certificate_still_matches_the_held_bytes() {
        let (k, pin) = witness();
        let cert = certificate_claiming(digest_of(ARTIFACT).to_ascii_uppercase());
        let signature = sign(&k, &cert);

        verify_witness_certificate(cert, &signature, Some(&pin), ARTIFACT)
            .expect("hex case is not a difference in digest");
    }

    #[test]
    fn a_certificate_signed_by_another_key_is_a_signature_failure() {
        let (_, pin) = witness();
        let cert = certificate();
        let signature = sign(&key("some other enclave"), &cert);

        let error = verify_witness_certificate(cert, &signature, Some(&pin), ARTIFACT)
            .expect_err("a foreign signer must be refused");
        assert_eq!(
            error,
            WitnessVerificationError::Signature(CertificateError::SignerMismatch)
        );
    }

    #[test]
    fn a_malformed_signature_is_named_as_malformed_not_as_a_mismatch() {
        let (_, pin) = witness();

        let error = verify_witness_certificate(certificate(), "0xnothex", Some(&pin), ARTIFACT)
            .expect_err("a malformed signature must be refused");
        assert_eq!(
            error,
            WitnessVerificationError::Signature(CertificateError::SignatureMalformed)
        );
    }

    #[test]
    fn swapping_in_a_pinned_measurement_after_signing_fails_the_signature() {
        let (k, pin) = witness();
        // The attacker holds a genuine certificate from an unpinned enclave
        // and edits the measurement to one that is pinned. The signature no
        // longer covers the certificate, and that is what must be reported --
        // not a measurement failure, which would send an operator to their
        // pin list rather than to the fact that somebody is editing signed
        // structures.
        let signature = sign(&k, &certificate_reporting(&"d".repeat(64)));
        let cert = certificate();
        let error = verify_witness_certificate(cert, &signature, Some(&pin), ARTIFACT)
            .expect_err("an edited certificate must be refused");
        assert_eq!(
            error,
            WitnessVerificationError::Signature(CertificateError::SignerMismatch)
        );
    }

    #[test]
    fn the_missing_control_is_reported_before_any_other_failure() {
        // Unpinned, wrong signer, and the wrong bytes all at once. The
        // operator's own missing configuration is what they need to hear
        // first: with no pin, the other two verdicts were not computed
        // against anything trustworthy.
        let cert = certificate();
        let signature = sign(&key("some other enclave"), &cert);

        let error = verify_witness_certificate(cert, &signature, None, b"other bytes")
            .expect_err("an unpinned operator must not get a pass");
        assert_eq!(
            error,
            WitnessVerificationError::Unpinned {
                control: EXPECTED_MEASUREMENT_CONTROL,
            }
        );
    }

    #[test]
    fn a_bad_signature_is_reported_before_the_measurement_it_claims() {
        let (_, pin) = witness();
        let cert = certificate_reporting(&"d".repeat(64));
        let signature = sign(&key("some other enclave"), &cert);

        // A certificate nobody vouched for must not get its self-reported
        // measurement rendered into an operator surface. Signature first is
        // what keeps `MeasurementNotPinned`'s payload attacker-free.
        let error = verify_witness_certificate(cert, &signature, Some(&pin), ARTIFACT)
            .expect_err("a foreign signer must be refused");
        assert_eq!(
            error,
            WitnessVerificationError::Signature(CertificateError::SignerMismatch)
        );
    }

    #[test]
    fn measurements_are_compared_exactly_and_do_not_case_fold() {
        let (k, pin) = witness();
        let cert = certificate_reporting(&PINNED_MEASUREMENT.to_ascii_uppercase());
        let signature = sign(&k, &cert);

        let error = verify_witness_certificate(cert, &signature, Some(&pin), ARTIFACT)
            .expect_err("a measurement pin is compared exactly");
        assert_eq!(
            error,
            WitnessVerificationError::MeasurementNotPinned {
                reported: PINNED_MEASUREMENT.to_ascii_uppercase(),
            }
        );
    }

    #[test]
    fn a_pin_with_several_measurements_admits_each_of_them() {
        let k = key("witness enclave signing key");
        let pin = WitnessPin::new(
            &address_of_key(&k),
            ["e".repeat(64), PINNED_MEASUREMENT.to_string()],
        )
        .expect("pin is well formed");
        assert_eq!(pin.pinned_measurement_count(), 2);

        for measurement in ["e".repeat(64), PINNED_MEASUREMENT.to_string()] {
            let cert = certificate_reporting(&measurement);
            let signature = sign(&k, &cert);
            let verified = verify_witness_certificate(cert, &signature, Some(&pin), ARTIFACT)
                .expect("a pinned measurement verifies");
            assert_eq!(verified.witness_measurement(), measurement);
        }
    }

    #[test]
    fn a_pin_with_no_measurements_cannot_be_constructed() {
        let address = address_of_key(&key("witness enclave signing key"));
        assert_eq!(
            WitnessPin::new(&address, []).expect_err("an empty pin is not a pin"),
            WitnessPinError::NoMeasurements
        );
    }

    #[test]
    fn a_pin_with_a_blank_measurement_cannot_be_constructed() {
        let address = address_of_key(&key("witness enclave signing key"));
        assert_eq!(
            WitnessPin::new(&address, ["   ".to_string()])
                .expect_err("a blank measurement is not a pin"),
            WitnessPinError::MeasurementBlank
        );
    }

    #[test]
    fn a_malformed_signing_address_is_a_configuration_error_not_a_refusal() {
        assert_eq!(
            WitnessPin::new("0xnope", [PINNED_MEASUREMENT.to_string()])
                .expect_err("a malformed address is not a pin"),
            WitnessPinError::SigningAddressMalformed
        );
    }

    #[test]
    fn both_error_formatters_render_the_same_safe_text() {
        let errors = [
            (
                WitnessVerificationError::Unpinned {
                    control: EXPECTED_MEASUREMENT_CONTROL,
                },
                EXPECTED_MEASUREMENT_CONTROL,
            ),
            (
                WitnessVerificationError::Signature(CertificateError::SignerMismatch),
                "signed by a different signer",
            ),
            (
                WitnessVerificationError::ArtifactMismatch,
                "does not cover the artifact on hand",
            ),
            (
                WitnessVerificationError::MeasurementNotPinned {
                    reported: PINNED_MEASUREMENT.to_string(),
                },
                PINNED_MEASUREMENT,
            ),
        ];
        for (error, expected) in errors {
            let display = format!("{error}");
            let debug = format!("{error:?}");
            assert_eq!(display, debug, "Debug renders more than Display");
            // Positive control. Without it every leak assertion below holds
            // vacuously for a blank formatter -- and most of them hold
            // vacuously anyway, since no variant carries a token count or a
            // certificate. This is the assertion that observes something.
            assert!(
                display.contains(expected),
                "expected {expected} in the rendered error, got {display}"
            );
            // Nothing that identifies a contributor or their session.
            for leak in [
                "chatcmpl-secret-session",
                "qwen3.6-27b-fp8",
                "1204",
                "337",
                &digest_of(ARTIFACT),
            ] {
                assert!(
                    !display.contains(leak),
                    "error text renders {leak}: {display}"
                );
            }
        }

        for (error, expected) in [
            (
                WitnessPinError::SigningAddressMalformed,
                "not a 20-byte hex address",
            ),
            (WitnessPinError::NoMeasurements, "named no expected"),
            (
                WitnessPinError::MeasurementBlank,
                "blank expected measurement",
            ),
        ] {
            let display = format!("{error}");
            assert_eq!(display, format!("{error:?}"));
            assert!(display.contains(expected), "got {display}");
        }
    }

    #[test]
    fn neither_formatter_renders_verified_certificate_identifiers() {
        let (k, pin) = witness();
        let cert = certificate();
        let signature = sign(&k, &cert);
        let verified =
            verify_witness_certificate(cert, &signature, Some(&pin), ARTIFACT).expect("verifies");

        // There is no Display; Debug is the path a `tracing` call would take.
        let debug = format!("{verified:?}");
        for leak in ["chatcmpl-secret-session", "qwen3.6-27b-fp8", "1204", "337"] {
            assert!(!debug.contains(leak), "Debug renders {leak}: {debug}");
        }
        assert!(debug.contains(PINNED_MEASUREMENT));
    }
}
