// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Server-side verification of a witness certificate.
//!
//! Four facts have to hold before a certificate says anything useful about
//! an artifact the server holds:
//!
//! 1. the signature recovers to the witness signing address the operator
//!    pinned;
//! 2. `timestamp` is inside the pin's freshness window;
//! 3. `witness_measurement` is one the operator pinned;
//! 4. `redacted_sha256` is the digest of the bytes actually on hand.
//!
//! They run in that order, and a test pins it.
//!
//! # What the freshness window is, and is not
//!
//! A certificate names no submitter and carries no nonce. Nothing in it says
//! who may present it, so the pair (envelope bytes, certificate) is a bearer
//! token: whoever holds it can submit those bytes under any account and get
//! the bypass. Before [`WitnessFreshness`] existed that was true forever --
//! `timestamp` was signed into the preimage and then read only to render a
//! header, so a single captured pair replayed indefinitely.
//!
//! The window does not fix that. It bounds it. A certificate is still
//! replayable by anyone holding it, for as long as the window lasts. Making
//! one single-use, or binding it to a submission or a tenant, requires a
//! nonce or a submission identifier inside the signed preimage -- a protocol
//! change, on both sides, that this module cannot make alone.
//!
//! [`WitnessCertificate::verify`] checks only the first. A caller who ran it
//! and stopped there would have established that *some* enclave signed
//! *something* -- not that this certificate covers this artifact, not that
//! the enclave running the witness is one anybody vouched for, and not that
//! it was issued at any time in particular.
//!
//! So this module does not offer four checks. It offers one entry point,
//! [`verify_witness_certificate`], which takes every input the three checks
//! need in a single call, consumes the certificate, and returns a
//! [`VerifiedWitnessCertificate`] that has no other constructor. A partial
//! verification is not discouraged here, it is unspeakable: there is no way
//! to obtain the verified type without having passed all four, and the
//! field a policy would want to act on -- the residual-risk verdict --
//! is reachable only through it.
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
use trace_commons_protocol::trace_contribution::ResidualPiiRisk;

/// Missing-control name reported when the operator has pinned no witness.
pub const EXPECTED_MEASUREMENT_CONTROL: &str = "witness_expected_measurement";

/// How old a certificate may be, by default, and still be accepted.
///
/// Twenty-four hours. The honest path takes seconds -- a contributor witnesses
/// a session and submits the envelope it got back -- so this is not a bound on
/// normal use but on how long a captured pair stays replayable. It is generous
/// deliberately: a contributor who witnessed a session and then lost
/// connectivity should not have to send the raw session a second time, and by
/// then it has already left their machine once.
pub const DEFAULT_CERTIFICATE_MAX_AGE_SECONDS: i64 = 24 * 60 * 60;

/// How far ahead of the server's clock a certificate may be stamped.
///
/// Five minutes. The witness stamps from its own clock, and two machines that
/// are both behaving still disagree by seconds; refusing at zero skew would
/// turn ordinary NTP drift into a refusal an operator cannot diagnose from the
/// message. It is small because a wide forward tolerance is a wide replay
/// window in disguise -- a certificate stamped `now + tolerance` is accepted
/// for `max_age + tolerance`.
pub const DEFAULT_CERTIFICATE_FORWARD_TOLERANCE_SECONDS: i64 = 5 * 60;

/// Why a freshness window could not be built.
#[derive(Clone, PartialEq, Eq, thiserror::Error)]
pub enum WitnessFreshnessError {
    /// The window is zero or negative, which would refuse every certificate
    /// including an honest one issued this instant.
    #[error("the witness certificate max age must be a positive number of seconds")]
    MaxAgeNotPositive,
    /// The forward tolerance is negative, which would refuse a certificate for
    /// being stamped at exactly the server's own clock.
    #[error("the witness certificate forward tolerance must not be negative")]
    ForwardToleranceNegative,
}

impl std::fmt::Debug for WitnessFreshnessError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, formatter)
    }
}

/// How long a certificate stays acceptable.
///
/// A certificate carries no nonce and names no submitter, so the pair
/// (envelope bytes, certificate) is a bearer token: anyone who observes one
/// can present it again, under any account, for as long as it verifies. Until
/// the signed preimage binds a submission there is no way to make that
/// single-use, and this window is the only thing that bounds it at all.
///
/// It is part of [`WitnessPin`] rather than a separate argument to
/// [`verify_witness_certificate`] so that there is no way to verify a
/// certificate without one. An `Option` here, or a second parameter a caller
/// could pass `None` to, would make "no freshness check" expressible -- and
/// the state this fixes is precisely that it was not expressible any other
/// way.
///
/// Values are operator configuration. `Debug` is derived for the same reason
/// [`WitnessPin`]'s is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WitnessFreshness {
    max_age_seconds: i64,
    forward_tolerance_seconds: i64,
}

impl Default for WitnessFreshness {
    fn default() -> Self {
        Self {
            max_age_seconds: DEFAULT_CERTIFICATE_MAX_AGE_SECONDS,
            forward_tolerance_seconds: DEFAULT_CERTIFICATE_FORWARD_TOLERANCE_SECONDS,
        }
    }
}

impl WitnessFreshness {
    /// Build a window. The forward tolerance keeps its default.
    pub fn new(max_age_seconds: i64) -> Result<Self, WitnessFreshnessError> {
        Self::with_forward_tolerance(
            max_age_seconds,
            DEFAULT_CERTIFICATE_FORWARD_TOLERANCE_SECONDS,
        )
    }

    /// Build a window, naming both halves.
    pub fn with_forward_tolerance(
        max_age_seconds: i64,
        forward_tolerance_seconds: i64,
    ) -> Result<Self, WitnessFreshnessError> {
        if max_age_seconds <= 0 {
            return Err(WitnessFreshnessError::MaxAgeNotPositive);
        }
        if forward_tolerance_seconds < 0 {
            return Err(WitnessFreshnessError::ForwardToleranceNegative);
        }
        Ok(Self {
            max_age_seconds,
            forward_tolerance_seconds,
        })
    }

    /// The configured maximum age, in seconds.
    pub fn max_age_seconds(&self) -> i64 {
        self.max_age_seconds
    }

    /// The configured forward tolerance, in seconds.
    pub fn forward_tolerance_seconds(&self) -> i64 {
        self.forward_tolerance_seconds
    }

    /// Judge a claimed timestamp against `now`, both as Unix seconds.
    ///
    /// `checked_sub` rather than `-`: a certificate may claim `i64::MIN`, and
    /// an overflow in a security check is not a diagnostic, it is a panic on
    /// attacker-chosen input inside a request handler. An age that does not
    /// compute is refused as expired, which is the fail-closed direction.
    fn check(&self, claimed: i64, now: i64) -> Result<(), WitnessVerificationError> {
        let Some(age) = now.checked_sub(claimed) else {
            return Err(WitnessVerificationError::CertificateExpired {
                age_seconds: self.max_age_seconds,
            });
        };
        if age > self.max_age_seconds {
            return Err(WitnessVerificationError::CertificateExpired { age_seconds: age });
        }
        if age < -self.forward_tolerance_seconds {
            return Err(WitnessVerificationError::CertificateFutureDated { skew_seconds: -age });
        }
        Ok(())
    }
}

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
///
/// # Why the derived `Debug` is safe here, as an exception
///
/// This repo does not log signing addresses, and rendering one is normally
/// wrong. This is the exception, deliberately: a witness signing address is
/// a *public verification key's* address, published in the enclave's own
/// attestation report, and it is operator config rather than a credential.
/// Nothing is authorised by holding it. The rule exists for addresses whose
/// exposure links a payer or reveals a key an operator controls, and neither
/// applies.
///
/// Stated because an unexplained exception gets "fixed" in the wrong
/// direction: someone will either strip this `Debug` and lose the one
/// rendering that helps diagnose a pin, or read it as licence to render
/// addresses elsewhere. Neither follows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WitnessPin {
    signing_address: String,
    measurements: BTreeSet<String>,
    freshness: WitnessFreshness,
}

impl WitnessPin {
    /// Validate and build a pin.
    ///
    /// Measurements are compared exactly, byte for byte. They are opaque
    /// identifiers a witness reports rather than values with two circulating
    /// spellings, so a case-folding comparison could only conflate two
    /// distinct pins; a case difference against an honest witness fails
    /// closed and is diagnosable from the reported value.
    ///
    /// Surrounding whitespace is trimmed off each entry before it is pinned.
    /// It is the one difference an operator cannot see in their own config,
    /// and a pinned `" abc "` would silently match nothing.
    pub fn new(
        signing_address: &str,
        measurements: impl IntoIterator<Item = String>,
    ) -> Result<Self, WitnessPinError> {
        if decode_address(signing_address).is_none() {
            return Err(WitnessPinError::SigningAddressMalformed);
        }
        let mut pinned = BTreeSet::new();
        for measurement in measurements {
            let measurement = measurement.trim();
            if measurement.is_empty() {
                return Err(WitnessPinError::MeasurementBlank);
            }
            pinned.insert(measurement.to_string());
        }
        if pinned.is_empty() {
            return Err(WitnessPinError::NoMeasurements);
        }
        Ok(WitnessPin {
            signing_address: signing_address.to_string(),
            measurements: pinned,
            freshness: WitnessFreshness::default(),
        })
    }

    /// Replace the default freshness window.
    ///
    /// Additive rather than a third parameter on [`Self::new`], so that a pin
    /// always HAS a window and an operator who configures nothing gets the
    /// default rather than no check.
    pub fn with_freshness(mut self, freshness: WitnessFreshness) -> Self {
        self.freshness = freshness;
        self
    }

    /// The freshness window this pin applies.
    pub fn freshness(&self) -> WitnessFreshness {
        self.freshness
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
    /// The certificate is genuine and older than the configured window.
    ///
    /// Carries the age in seconds, not the timestamp. The age is what an
    /// operator acts on -- widen the window, or investigate a replay -- and it
    /// is safe to render for the same reason the reported measurement is: the
    /// signature is checked first, so this value derives from one the PINNED
    /// witness stamped, not one a sender chose.
    #[error("the witness certificate is {age_seconds}s old, past the configured window")]
    CertificateExpired { age_seconds: i64 },
    /// The certificate is genuine and stamped further into the future than the
    /// forward tolerance allows.
    ///
    /// Almost always a clock, not an attack -- which is why it is its own
    /// variant. Told a certificate is expired, an operator inspects the
    /// contributor; told it is future-dated by a large skew, they inspect the
    /// witness's clock, which is the actual fix.
    #[error("the witness certificate is stamped {skew_seconds}s in the future")]
    CertificateFutureDated { skew_seconds: i64 },
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
/// downstream that wants the residual-risk verdict must take this type, and a
/// bare [`WitnessCertificate`] will not do.
///
/// `Debug` delegates to [`WitnessCertificate`]'s, which renders every field.
#[derive(Clone, PartialEq, Eq)]
pub struct VerifiedWitnessCertificate {
    certificate: WitnessCertificate,
}

impl std::fmt::Debug for VerifiedWitnessCertificate {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Delegates to the certificate's own hand-written Debug. No field
        // on a certificate identifies a contributor or an upstream
        // conversation any more -- see that type's Logging note.
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

    /// The residual-PII verdict the witness reached over the artifact.
    ///
    /// Lands with its caller, as this module's standing rule below requires:
    /// `corpus_status_with_pii_backstop_hold` in the ingest binary reads it,
    /// and nothing else may.
    ///
    /// **This is a pass over the ORIGINATING redaction pass, not a clean
    /// bill.** A `Low` here says a known program in a pinned enclave reached
    /// `Low` over these bytes. It does not say the artifact is clean, and no
    /// caller may render it as though it did -- see the standing note below
    /// for the credential case that survives it.
    pub fn residual_risk_verdict(&self) -> ResidualPiiRisk {
        self.certificate.residual_risk_verdict()
    }

    /// The redaction-policy alias the witness reported.
    ///
    /// **An alias, never an authority.** `redaction_pipeline_version()`
    /// concatenates hardcoded constants selected by backend family, so every
    /// self-hosted deployment reports the same string regardless of which
    /// checkpoint loaded. A caller checks it against
    /// [`WitnessBypassConfig::policy_version_allowed`](super::config::WitnessBypassConfig::policy_version_allowed)
    /// and trusts [`Self::witness_measurement`]. Lands with that caller.
    pub fn redaction_policy_version(&self) -> &str {
        self.certificate.claimed_redaction_policy_version()
    }

    /// The measurement the witness reported, which verification has proven is
    /// one the operator pinned. A caller reporting the strength of the check
    /// wants this.
    pub fn witness_measurement(&self) -> &str {
        self.certificate.claimed_witness_measurement()
    }
}

// There is deliberately no accessor for `timestamp`, and the rule that kept
// the two above from existing still stands for it: adding a getter per field
// would hand back exactly the unverified-read surface that making the
// certificate's fields private just closed. The moment something legitimately
// needs one is the moment to add it -- with a caller in the same commit.
//
// `residual_risk_verdict` and `redaction_policy_version` got theirs under
// that rule, in the commit that landed the PII-backstop bypass. Their one
// caller is `corpus_status_with_pii_backstop_hold` in the ingest binary.
//
// WHOEVER TOUCHES THAT BYPASS: read this first. A verified certificate does
// NOT license skipping the backstop wholesale, and the reason is not
// conservatism.
//
// The witness runs the ORIGINATING redaction ordering -- deterministic pass,
// then prose classifier -- because that is what its raw input requires, and
// matching it is what makes the certificate describe the pass ingest performs.
// The server backstop runs a different one, and its trailing deterministic
// sweep is not redundancy: the classifier is trained on prose PII, not
// credential formats, so it writes a credential straight back into a field it
// was handed. That sweep is what catches it, and per the pilot's own notes
// that case is the whole of the quarantine backlog.
//
// So a `Low` verdict here is a PASS OVER THE ORIGINATING PASS. A credential
// the classifier itself emitted survives it and is still on the artifact. A
// bypass may therefore skip the backstop's CLASSIFIER stage on a verified
// certificate; it must still run the trailing sweep, or it re-opens exactly
// the hole the sweep exists to close.
//
// HOW THE LANDED BYPASS SATISFIES THAT, so nobody re-derives it wrongly: the
// bypass needs no exemption from the sweep, because the sweep has already
// run. `rescrub_trace_envelope` -- the deterministic pass over
// `redacted_content` and `structured_payload`, plus `residual_envelope_scan`
// -- runs synchronously in the submit handler BEFORE the hold is decided, and
// a credential it finds raises the risk so the trace never reaches the
// `Accepted` status the bypass is gated on. The only thing skipping the hold
// removes is the async backstop's classifier stage. That ordering is the
// entire safety argument, and the ingest binary pins it with a source-order
// test. If the hold decision is ever moved above the rescrub, this stops
// being true and the feature becomes a wholesale bypass.
//
// `deploy/witness/README.md` states the same limit for operators, and
// `witness_service::mod`'s doc states it on the issuing side. It is repeated
// here because this is the file the bypass gets written in.
//
// `timestamp` is bound by the signature and now bounded by a freshness window
// as well -- see `WitnessFreshness`. That window is what stops the same
// (bytes, certificate) pair from being replayed forever; it is NOT a binding
// to a submission, a tenant, or an account, and a certificate remains
// replayable by anyone holding it FOR AS LONG AS THE WINDOW LASTS. Binding one
// to a submitter needs a nonce or a submission identifier inside the signed
// preimage, which is a protocol change this module cannot make on its own.

/// Verify a witness certificate against the artifact the server holds.
///
/// All four checks or none: the signature against the pinned address, the
/// claimed timestamp against the pin's freshness window, the reported
/// measurement against the pinned set, and the certificate's digest against
/// `redacted_bytes` -- in that order, which a test pins. There is no way to
/// run a subset, and the successful return value cannot be produced any other
/// way.
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
    verify_witness_certificate_at(
        certificate,
        signature_hex,
        pin,
        redacted_bytes,
        chrono::Utc::now().timestamp(),
    )
}

/// [`verify_witness_certificate`] with the current time supplied.
///
/// `pub(crate)` and not the entry point: a caller that could choose `now`
/// could choose the certificate's own timestamp and turn the freshness check
/// into a tautology. Reading the clock is part of the check, so the public
/// function reads it, and this exists only so a test can drive a certificate
/// across the window boundary without sleeping through it.
pub(crate) fn verify_witness_certificate_at(
    certificate: WitnessCertificate,
    signature_hex: &str,
    pin: Option<&WitnessPin>,
    redacted_bytes: &[u8],
    now: i64,
) -> Result<VerifiedWitnessCertificate, WitnessVerificationError> {
    let Some(pin) = pin else {
        return Err(WitnessVerificationError::Unpinned {
            control: EXPECTED_MEASUREMENT_CONTROL,
        });
    };

    certificate
        .verify(signature_hex, &pin.signing_address)
        .map_err(WitnessVerificationError::Signature)?;

    // Second, and only now: the signature has been checked, so the timestamp
    // is one the pinned witness stamped rather than one a sender chose. A
    // freshness check ahead of the signature would be judging an unsigned
    // integer, and would put an attacker-chosen value on an operator surface.
    pin.freshness.check(certificate.claimed_timestamp(), now)?;

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
            residual_risk_verdict: ResidualPiiRisk::Medium,
            redaction_policy_version: "policy-v3".to_string(),
            witness_measurement: PINNED_MEASUREMENT.to_string(),
            // Stamped now, because every test below except the freshness ones
            // is about something else and a fixed past instant would make all
            // of them fail on the window instead. The freshness tests choose
            // their own `now` through `verify_witness_certificate_at`.
            timestamp: chrono::Utc::now().timestamp(),
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

    /// A certificate stamped at `at`, covering [`ARTIFACT`].
    fn certificate_stamped(at: i64) -> WitnessCertificate {
        WitnessCertificate::from_parts(
            digest_of(ARTIFACT),
            CertificateDetails {
                timestamp: at,
                ..details()
            },
        )
    }

    /// The signed timestamp was never read. Nothing checked it, so the same
    /// (bytes, certificate) pair verified forever -- a bearer token with no
    /// expiry, presentable by anyone who observed one.
    #[test]
    fn a_certificate_older_than_the_window_is_refused() {
        let (k, pin) = witness();
        let now = 1_800_000_000;
        let cert = certificate_stamped(now - DEFAULT_CERTIFICATE_MAX_AGE_SECONDS - 1);
        let signature = sign(&k, &cert);

        let err = verify_witness_certificate_at(cert, &signature, Some(&pin), ARTIFACT, now)
            .expect_err("a certificate past the window must refuse");
        assert_eq!(
            err,
            WitnessVerificationError::CertificateExpired {
                age_seconds: DEFAULT_CERTIFICATE_MAX_AGE_SECONDS + 1
            },
            "{err}"
        );
    }

    /// The PUBLIC entry point refuses a certificate stamped long ago.
    ///
    /// Every other freshness test drives `verify_witness_certificate_at`,
    /// which proves the predicate but not that production applies it. The
    /// entry point reads the clock itself, and that read is the part a
    /// regression removes: replacing `Utc::now()` with the certificate's own
    /// timestamp makes every age zero and disables the window completely.
    /// Nothing caught that, because the rest of this suite stamps `now` and
    /// passes either way. This test does not -- 2020 is more than 24 hours
    /// before any clock this will ever run on.
    #[test]
    fn the_public_entry_point_refuses_an_ancient_certificate() {
        let (k, pin) = witness();
        let cert = certificate_stamped(1_600_000_000);
        let signature = sign(&k, &cert);

        let err = verify_witness_certificate(cert, &signature, Some(&pin), ARTIFACT)
            .expect_err("a certificate from 2020 must refuse against the real clock");
        assert!(
            matches!(err, WitnessVerificationError::CertificateExpired { .. }),
            "the entry point is not applying the freshness window: {err}"
        );
    }

    /// And the same entry point refuses one stamped past the tolerance ahead.
    ///
    /// Relative to the real clock rather than an absolute future instant, so
    /// it cannot quietly stop being in the future.
    #[test]
    fn the_public_entry_point_refuses_a_future_dated_certificate() {
        let (k, pin) = witness();
        let cert = certificate_stamped(chrono::Utc::now().timestamp() + 86_400);
        let signature = sign(&k, &cert);

        let err = verify_witness_certificate(cert, &signature, Some(&pin), ARTIFACT)
            .expect_err("a certificate stamped tomorrow must refuse against the real clock");
        assert!(
            matches!(err, WitnessVerificationError::CertificateFutureDated { .. }),
            "the entry point is not applying the forward tolerance: {err}"
        );
    }

    /// The boundary itself passes. A window that refused at exactly its own
    /// width would be one second narrower than it is documented to be.
    #[test]
    fn a_certificate_at_exactly_the_window_still_verifies() {
        let (k, pin) = witness();
        let now = 1_800_000_000;
        let cert = certificate_stamped(now - DEFAULT_CERTIFICATE_MAX_AGE_SECONDS);
        let signature = sign(&k, &cert);

        verify_witness_certificate_at(cert, &signature, Some(&pin), ARTIFACT, now)
            .expect("the last second of the window is inside it");
    }

    /// Ordinary clock skew is tolerated; a certificate from next week is not.
    #[test]
    fn a_certificate_inside_the_forward_tolerance_verifies_and_one_beyond_it_does_not() {
        let (k, pin) = witness();
        let now = 1_800_000_000;

        let near = certificate_stamped(now + DEFAULT_CERTIFICATE_FORWARD_TOLERANCE_SECONDS);
        let signature = sign(&k, &near);
        verify_witness_certificate_at(near, &signature, Some(&pin), ARTIFACT, now)
            .expect("skew inside the tolerance is not a refusal");

        let far = certificate_stamped(now + DEFAULT_CERTIFICATE_FORWARD_TOLERANCE_SECONDS + 1);
        let signature = sign(&k, &far);
        let err = verify_witness_certificate_at(far, &signature, Some(&pin), ARTIFACT, now)
            .expect_err("a future-dated certificate must refuse");
        assert_eq!(
            err,
            WitnessVerificationError::CertificateFutureDated {
                skew_seconds: DEFAULT_CERTIFICATE_FORWARD_TOLERANCE_SECONDS + 1
            },
            "{err}"
        );
    }

    /// Future-dating is its own refusal, not "expired".
    ///
    /// The two send an operator to different places: expired means look at
    /// the submission, future-dated means look at the witness's clock.
    #[test]
    fn a_future_dated_certificate_is_not_reported_as_expired() {
        let (k, pin) = witness();
        let now = 1_800_000_000;
        let cert = certificate_stamped(now + 86_400);
        let signature = sign(&k, &cert);

        let err = verify_witness_certificate_at(cert, &signature, Some(&pin), ARTIFACT, now)
            .expect_err("a certificate from tomorrow must refuse");
        assert!(
            matches!(err, WitnessVerificationError::CertificateFutureDated { .. }),
            "{err}"
        );
    }

    /// `i64::MIN` is a timestamp a sender can type. Subtracting it overflows,
    /// and an overflow inside a request handler is a panic, not a refusal.
    #[test]
    fn a_timestamp_that_overflows_the_age_computation_refuses_rather_than_panicking() {
        let (k, pin) = witness();
        let cert = certificate_stamped(i64::MIN);
        let signature = sign(&k, &cert);

        let err = verify_witness_certificate_at(cert, &signature, Some(&pin), ARTIFACT, 1)
            .expect_err("an unrepresentable age must refuse");
        assert!(
            matches!(err, WitnessVerificationError::CertificateExpired { .. }),
            "{err}"
        );
    }

    /// The window is checked AFTER the signature.
    ///
    /// Order matters for what reaches an operator surface: the age rendered
    /// in `CertificateExpired` is only trustworthy because it comes from a
    /// certificate the pinned witness signed. A stale certificate signed by
    /// somebody else must report the signature, not the age.
    #[test]
    fn a_stale_certificate_signed_by_a_stranger_reports_the_signature() {
        let (_, pin) = witness();
        let stranger = key("not the witness");
        let now = 1_800_000_000;
        let cert = certificate_stamped(now - DEFAULT_CERTIFICATE_MAX_AGE_SECONDS - 1000);
        let signature = sign(&stranger, &cert);

        let err = verify_witness_certificate_at(cert, &signature, Some(&pin), ARTIFACT, now)
            .expect_err("a stranger's signature must refuse");
        assert!(
            matches!(err, WitnessVerificationError::Signature(_)),
            "the freshness check ran before the signature: {err}"
        );
    }

    /// And before the measurement, so an operator widening a pin is not sent
    /// chasing an enclave over a certificate that was stale anyway.
    #[test]
    fn a_stale_certificate_from_an_unpinned_enclave_reports_the_age() {
        let (k, pin) = witness();
        let now = 1_800_000_000;
        let cert = WitnessCertificate::from_parts(
            digest_of(ARTIFACT),
            CertificateDetails {
                witness_measurement: "d2d2d2d2".to_string(),
                timestamp: now - DEFAULT_CERTIFICATE_MAX_AGE_SECONDS - 1,
                ..details()
            },
        );
        let signature = sign(&k, &cert);

        let err = verify_witness_certificate_at(cert, &signature, Some(&pin), ARTIFACT, now)
            .expect_err("a stale certificate must refuse");
        assert!(
            matches!(err, WitnessVerificationError::CertificateExpired { .. }),
            "{err}"
        );
    }

    /// An operator can narrow the window, and the narrowed one is what
    /// applies.
    #[test]
    fn a_configured_window_replaces_the_default() {
        let (k, _) = witness();
        let pin = WitnessPin::new(&address_of_key(&k), [PINNED_MEASUREMENT.to_string()])
            .expect("pin is well formed")
            .with_freshness(WitnessFreshness::new(60).expect("a minute is a window"));
        let now = 1_800_000_000;

        let inside = certificate_stamped(now - 59);
        let signature = sign(&k, &inside);
        verify_witness_certificate_at(inside, &signature, Some(&pin), ARTIFACT, now)
            .expect("inside the narrowed window");

        // And a certificate the DEFAULT window would have admitted does not
        // get in through the narrowed one.
        let outside = certificate_stamped(now - 3_600);
        let signature = sign(&k, &outside);
        let err = verify_witness_certificate_at(outside, &signature, Some(&pin), ARTIFACT, now)
            .expect_err("outside the narrowed window");
        assert_eq!(
            err,
            WitnessVerificationError::CertificateExpired { age_seconds: 3_600 },
            "{err}"
        );
    }

    /// A pin that names no window still has one.
    #[test]
    fn a_pin_built_without_a_window_carries_the_default() {
        let (_, pin) = witness();
        assert_eq!(
            pin.freshness().max_age_seconds(),
            DEFAULT_CERTIFICATE_MAX_AGE_SECONDS
        );
        assert_eq!(
            pin.freshness().forward_tolerance_seconds(),
            DEFAULT_CERTIFICATE_FORWARD_TOLERANCE_SECONDS
        );
    }

    /// A window that admits nothing is a configuration error, not a very
    /// strict policy.
    #[test]
    fn a_window_that_is_not_positive_refuses_to_build() {
        for seconds in [0, -1, i64::MIN] {
            let err = WitnessFreshness::new(seconds).expect_err("must refuse");
            assert_eq!(err, WitnessFreshnessError::MaxAgeNotPositive, "{err}");
        }
        let err = WitnessFreshness::with_forward_tolerance(60, -1).expect_err("must refuse");
        assert_eq!(
            err,
            WitnessFreshnessError::ForwardToleranceNegative,
            "{err}"
        );
    }

    /// The refusals render an age and a skew, and nothing else.
    #[test]
    fn the_freshness_refusals_carry_no_content() {
        for err in [
            WitnessVerificationError::CertificateExpired { age_seconds: 99 },
            WitnessVerificationError::CertificateFutureDated { skew_seconds: 99 },
        ] {
            let rendered = format!("{err} {err:?}");
            assert!(rendered.contains("99"), "{rendered}");
            assert!(
                !rendered.contains(&digest_of(ARTIFACT)) && !rendered.contains(PINNED_MEASUREMENT),
                "a freshness refusal rendered certificate content: {rendered}"
            );
        }
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
    fn a_pin_trims_the_measurement_it_stores() {
        // Padding is invisible in an operator's own config file, and a
        // pinned " abc " matches no honest witness.
        let k = key("witness enclave signing key");
        let pin = WitnessPin::new(&address_of_key(&k), [format!("  {PINNED_MEASUREMENT}  ")])
            .expect("pin is well formed");
        let cert = certificate();
        let signature = sign(&k, &cert);

        verify_witness_certificate(cert, &signature, Some(&pin), ARTIFACT)
            .expect("the trimmed value is what got pinned");
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
            // Nothing that hands back contributor content. The artifact
            // digest is the handle on it that this module's hash-only
            // discipline keeps out of error text, and the policy alias is a
            // certificate field no refusal has any reason to echo.
            for leak in ["policy-v3", &digest_of(ARTIFACT)] {
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
    fn the_verified_wrapper_renders_the_certificate_it_wraps() {
        let (k, pin) = witness();
        let cert = certificate();
        let signature = sign(&k, &cert);
        let verified =
            verify_witness_certificate(cert, &signature, Some(&pin), ARTIFACT).expect("verifies");

        // There is no Display; Debug is the path a `tracing` call would take.
        let debug = format!("{verified:?}");
        assert!(debug.contains(PINNED_MEASUREMENT));
        assert!(debug.contains(&digest_of(ARTIFACT)));
        assert!(debug.contains("Medium"));
    }
}

/// The verifier against a certificate a real enclave actually issued.
///
/// Every other fixture in this file was written beside the code that checks
/// it, so all of them agree by construction: if the signing preimage, the
/// address recovery or the digest were wrong in the same way on both sides,
/// they would still pass. This one was captured, not authored -- from CVM
/// `8b8e6543-9743-41fc-ac05-a6b414888d5e` on dstack-pha-prod9, whose signing
/// key the Phala KMS derived and which this project has never held. It is the
/// only case here that can fail because our end is wrong.
///
/// What it therefore pins, and nothing weaker: that the preimage this crate
/// builds is byte-for-byte the one the witness binary signed, and that the
/// address recovered from a real secp256k1 signature is the one dstack
/// reported for the app. A drift in field order, in the domain string, or in
/// how the digest is spelled breaks this test and no other.
///
/// The verdict is `Medium` and the policy version is the deterministic alias
/// because that is what the enclave returned; a certificate is not required
/// to be admissible to be authentic, and the checks that would refuse this one
/// for a fast-path bypass live elsewhere. Do not "fix" those fields.
#[cfg(test)]
mod live_capture {
    use super::*;
    use crate::redaction_witness::certificate::CertificateDetails;

    // Captured 2026-09-04 from POST /v1/witness. Kept verbatim, including the
    // trailing newline, because the digest is over exactly these bytes.
    const ARTIFACT: &[u8] =
        b"user: deploy the thing\nassistant: using AWS_SECRET_ACCESS_KEY=[REDACTED] and my home dir <PRIVATE_LOCAL_PATH_1>\n";

    const REDACTED_SHA256: &str =
        "d78e9cfa5e7b3c58f44511084041127b05552c38665166de3d7e5f8d59c137f0";

    // The address the KMS derived for app f1654b0beac2ac2afae4235ee3d907096cd8f3de.
    // Nothing in this repository can produce a signature that recovers to it.
    const SIGNING_ADDRESS: &str = "0x655a17fcf6d0b9069e1b1dd07a7f5535d0c76798";

    const MEASUREMENT: &str = "mrtd:f06dfda6dce1cf904d4e2bab1dc370634cf95cefa2ceb2de2eee127c9382698090d7a4a13e14c536ec6c9c3c8fa87077+mrconfigid:01c2511a8b98937b819d4bd40bdbc65d38c766cb853649657ff9151ab4117befbd000000000000000000000000000000";

    const SIGNATURE_HEX: &str = "0x18ba77ef989ef61039f5c1d41f93916a1b4e211de4ad5fe88de499c8aa67e34156134a547392d783f78a0b4c9ec47ef320ecf791c1e9047a7a8771fe6e76c2a61c";

    const ISSUED_AT: i64 = 1788530732;

    fn captured() -> WitnessCertificate {
        WitnessCertificate::from_wire(
            REDACTED_SHA256.to_string(),
            CertificateDetails {
                residual_risk_verdict: ResidualPiiRisk::Medium,
                redaction_policy_version: "ironclaw-deterministic-secret-path-v3".to_string(),
                witness_measurement: MEASUREMENT.to_string(),
                timestamp: ISSUED_AT,
            },
        )
    }

    fn pin() -> WitnessPin {
        WitnessPin::new(SIGNING_ADDRESS, [MEASUREMENT.to_string()])
            .expect("the captured address and measurement are well-formed")
    }

    /// The clock is fixed at issue time rather than read, so this test does
    /// not start failing a day after the capture. Freshness has its own tests
    /// against synthetic certificates; what this one is for is the signature.
    fn just_after_issue() -> i64 {
        ISSUED_AT + 1
    }

    #[test]
    fn a_certificate_from_the_live_enclave_verifies() {
        let verified = verify_witness_certificate_at(
            captured(),
            SIGNATURE_HEX,
            Some(&pin()),
            ARTIFACT,
            just_after_issue(),
        )
        .expect("the captured certificate verifies against the address dstack reported");

        assert_eq!(verified.redacted_sha256(), REDACTED_SHA256);
        assert_eq!(verified.residual_risk_verdict(), ResidualPiiRisk::Medium);
        assert_eq!(verified.witness_measurement(), MEASUREMENT);
    }

    /// The digest in the captured certificate is over the captured artifact.
    ///
    /// Separate from the test above because that one would still pass if
    /// `verify_witness_certificate_at` compared the digest against itself
    /// rather than against the bytes it was handed.
    #[test]
    fn the_captured_digest_is_over_the_captured_artifact() {
        assert_eq!(hex::encode(Sha256::digest(ARTIFACT)), REDACTED_SHA256);
    }

    /// A single flipped bit in the artifact is refused.
    ///
    /// Guards against the digest comparison being dropped: with it gone, the
    /// test above still passes and this one does not.
    #[test]
    fn a_modified_artifact_is_refused() {
        let mut tampered = ARTIFACT.to_vec();
        *tampered.last_mut().expect("the artifact is not empty") = b'!';

        assert!(matches!(
            verify_witness_certificate_at(
                captured(),
                SIGNATURE_HEX,
                Some(&pin()),
                &tampered,
                just_after_issue(),
            ),
            Err(WitnessVerificationError::ArtifactMismatch)
        ));
    }

    /// The signature recovers to that address and not to a neighbouring one.
    ///
    /// Without this, a recovery that returned some fixed wrong address would
    /// pass every test above, since the pin would simply be that address.
    #[test]
    fn the_signature_does_not_verify_against_a_different_address() {
        let other = "0x655a17fcf6d0b9069e1b1dd07a7f5535d0c76799";
        assert_ne!(other, SIGNING_ADDRESS, "the decoy must differ");

        let pin = WitnessPin::new(other, [MEASUREMENT.to_string()])
            .expect("the decoy address is well-formed");

        assert!(matches!(
            verify_witness_certificate_at(
                captured(),
                SIGNATURE_HEX,
                Some(&pin),
                ARTIFACT,
                just_after_issue(),
            ),
            Err(WitnessVerificationError::Signature(_))
        ));
    }

    /// Any change to a signed field breaks the signature.
    ///
    /// The verdict is the field a forger would most want to move, and it is
    /// the one the fast-path decision reads. `Low` is the admissible value.
    #[test]
    fn upgrading_the_verdict_breaks_the_signature() {
        let forged = WitnessCertificate::from_wire(
            REDACTED_SHA256.to_string(),
            CertificateDetails {
                residual_risk_verdict: ResidualPiiRisk::Low,
                redaction_policy_version: "ironclaw-deterministic-secret-path-v3".to_string(),
                witness_measurement: MEASUREMENT.to_string(),
                timestamp: ISSUED_AT,
            },
        );

        assert!(matches!(
            verify_witness_certificate_at(
                forged,
                SIGNATURE_HEX,
                Some(&pin()),
                ARTIFACT,
                just_after_issue(),
            ),
            Err(WitnessVerificationError::Signature(_))
        ));
    }
}
