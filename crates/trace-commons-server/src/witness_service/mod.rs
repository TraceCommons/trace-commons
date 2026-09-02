// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The redaction witness service, as a function.
//!
//! [`witness`] is the whole service: it takes a contributor's raw transcript,
//! **performs** the redaction, computes the residual-PII verdict with the same
//! function ingest uses, and signs a [`WitnessCertificate`] over the artifact
//! it is about to return. The HTTP surface is a thin adapter over this, so the
//! service is testable without a socket.
//!
//! # It performs the redaction; it does not check one
//!
//! An earlier design had the witness verify a *client-supplied* span list.
//! That cannot work for the real pipeline: the redactor holds a model-based
//! prose classifier whose calls do not reproduce, so a witness that recomputed
//! and compared would refuse **honest** submissions. The witness therefore
//! produces the redaction, and correspondence holds by construction.
//!
//! [`check_correspondence`] is still on the path, and its job here is narrow
//! and worth stating exactly: it is the only producer of a
//! [`CorrespondenceProof`], and [`WitnessCertificate::from_proof`] is the only
//! production constructor of a certificate, so the digest a certificate claims
//! can only have come from bytes a check ran over. The witness runs it over
//! the artifact it is returning, with an empty span list, which binds the
//! certificate's digest to *those exact bytes* rather than to a string some
//! later refactor computed separately. It proves nothing about the raw text --
//! it cannot, and this module never claims it does.
//!
//! Two facts make a span-bearing check unavailable rather than merely
//! unnecessary, and they are recorded so the "improvement" is not attempted:
//! `DeterministicTraceRedactor` returns redacted text and a report, never a
//! span list; and its placeholders (`<PRIVATE_EMAIL_1>`) are not the
//! `[REDACTED]` / `[REDACTED:label]` grammar [`apply_spans`] enforces, so a
//! span list describing a real redaction by this pipeline would be refused as
//! [`CorrespondenceError::MalformedReplacement`].
//!
//! [`apply_spans`]: crate::redaction_witness::correspondence::apply_spans
//! [`CorrespondenceError::MalformedReplacement`]: crate::redaction_witness::correspondence::CorrespondenceError::MalformedReplacement
//!
//! # What the certificate does not say
//!
//! It attests **mechanics and a verdict**: that a program with a given
//! measurement produced this artifact and reached this residual-risk verdict
//! over it. It does not say the artifact is clean, and no name, comment,
//! error or field in this module may be read that way. A `Low` verdict from
//! an unpinned measurement is worth nothing at all.
//!
//! The verdict is a **pass** verdict. It is what
//! [`residual_risk_basis`] says about the redaction pass's own report, which
//! is the same thing `DeterministicTraceRedactor::redact_trace` writes into
//! `privacy.residual_pii_risk`. It is *not* the post-residual-scan verdict
//! `rescrub_trace_envelope` resolves on the server, which may raise it: a
//! detection-only sweep over the finished artifact can find a survivor this
//! pass cannot see. A server that trusts this verdict is trusting a pass
//! verdict, and the residual-scan half of its own decision is what it is
//! choosing to skip.
//!
//! # The witness holds nothing
//!
//! Raw bytes live in memory for one request. Nothing here writes, logs or
//! returns them, and no error carries content: every variant of
//! [`WitnessError`] is a bare label, and `Debug` delegates to `Display` so
//! that `tracing::warn!(?err)` cannot render what `%err` is guarded against.
//! [`WitnessRequest`] and [`WitnessResponse`] have hand-written `Debug`
//! impls that withhold every content-bearing field, for the same reason.

pub mod enclave;
pub mod http;
pub mod surface;

use async_trait::async_trait;
use std::sync::Arc;

use trace_commons_protocol::trace_contribution::{
    ConsentMetadata, DETERMINISTIC_REDACTION_PIPELINE_VERSION, DeterministicTraceRedactor,
    PrivacyFilterAdapter, PrivacyFilterBackendTag, RedactionReport, ResidualPiiRisk,
    ResidualRiskCondition, redaction_pipeline_version, residual_risk_basis,
};

use crate::redaction_witness::certificate::{CertificateDetails, WitnessCertificate};
use crate::redaction_witness::correspondence::check_correspondence;

/// What the contributor sends: the raw transcript and the consent flags that
/// declare what it carries.
///
/// The flags are part of the verdict's input -- `residual_risk_basis` floors
/// an envelope at Medium when a content flag is set -- so they must be
/// witnessed alongside the text rather than attached afterwards.
#[derive(Clone)]
pub struct WitnessRequest {
    /// The raw transcript. Never logged, never echoed, never persisted.
    pub raw_transcript: String,
    /// The contributor's declared consent flags, as they will be declared on
    /// the envelope.
    pub consent: ConsentMetadata,
}

impl std::fmt::Debug for WitnessRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Hand-written, and it withholds the transcript. A derived Debug on a
        // struct holding raw contributor text is one `?request` away from
        // putting a whole session in a log line.
        formatter
            .debug_struct("WitnessRequest")
            .field("raw_transcript", &"<withheld>")
            .field("consent", &"<withheld>")
            .finish()
    }
}

/// What the witness returns: the artifact, the certificate over it, and the
/// signature.
///
/// The artifact is the redacted text the contributor uploads through the
/// existing path. If they alter it the digest no longer matches and the
/// server refuses.
#[derive(Clone)]
pub struct WitnessResponse {
    /// The redacted artifact, byte for byte as the certificate's digest was
    /// taken over it. Any re-encoding, wrapper or added trailing newline
    /// between here and the server fails closed at
    /// `verify_witness_certificate`.
    pub redacted_artifact: String,
    /// The certificate. Its digest is over `redacted_artifact`.
    pub certificate: WitnessCertificate,
    /// EIP-191 signature over `certificate.signing_bytes()`, as 65 bytes of
    /// `0x`-prefixed hex.
    pub signature_hex: String,
}

impl WitnessResponse {
    /// The verdict the certificate binds.
    ///
    /// Read off the certificate rather than stored beside it. A second copy
    /// in this struct would be a second source of truth for the field the
    /// whole design turns on, and the two would eventually disagree.
    pub fn residual_risk_verdict(&self) -> ResidualPiiRisk {
        self.certificate.residual_risk_verdict()
    }
}

impl std::fmt::Debug for WitnessResponse {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The artifact is redacted, not clean -- it is contributor content
        // that a pass happened to find nothing more in. It is withheld here
        // for the same reason the raw transcript is. The signature is
        // withheld because the repo's logging convention says so.
        formatter
            .debug_struct("WitnessResponse")
            .field("redacted_artifact", &"<withheld>")
            .field("certificate", &self.certificate)
            .field("signature_hex", &"<withheld>")
            .finish()
    }
}

/// Why the witness refused.
///
/// Every variant is a bare label. Nothing here carries a reason string, a
/// count, an offset or a length: these errors reach operational surfaces, and
/// on this path every quantity derived from the input describes contributor
/// content. Callers branch on the variant.
///
/// There is no variant that means "certified with reservations". A witness
/// that cannot complete a step refuses; there is no partial certificate.
#[derive(Clone, PartialEq, Eq, thiserror::Error)]
pub enum WitnessError {
    /// The redaction did not complete. The configured prose-PII classifier
    /// was unavailable or errored, or the deterministic pass refused the
    /// input outright.
    ///
    /// This is the fail-closed arm the whole module exists around: a witness
    /// that certified what it had at the point of failure would be signing a
    /// partial redaction, which is worse than signing nothing.
    #[error("the witness could not complete the redaction")]
    RedactionFailed,
    /// The witness could not bind a digest to the artifact it produced.
    #[error("the witness could not bind a digest to the artifact it produced")]
    ArtifactBindingFailed,
    /// The enclave could not report its own measurement. Without it the
    /// certificate would name no program, and an operator's pin would have
    /// nothing to match.
    #[error("the enclave could not report its measurement")]
    MeasurementUnavailable,
    /// The signer refused or was unavailable.
    #[error("the witness could not sign the certificate")]
    SigningUnavailable,
}

impl std::fmt::Debug for WitnessError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, formatter)
    }
}

/// A seam could not do its job.
///
/// One type for all three seams, carrying nothing. Which refusal an operator
/// sees is decided in [`witness`] by the call site, not by the implementation
/// -- so a redactor cannot report itself as a signing failure, and no seam can
/// invent a variant that reads as a softer refusal than it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("unavailable")]
pub struct SeamUnavailable;

/// What a redaction pass produced.
///
/// The report travels with the text because the verdict is derived from it.
/// Returning only the text and letting the caller re-derive a report would be
/// a second implementation of the thing this module refuses to duplicate.
pub struct RedactedTranscript {
    /// The redacted text.
    pub redacted: String,
    /// The pass's own report, fed verbatim to [`residual_risk_basis`].
    pub report: RedactionReport,
    /// The policy alias this pass reports. **An alias, never an authority** --
    /// see [`CertificateDetails::redaction_policy_version`]. The measurement
    /// is the real policy identity.
    pub policy_version: String,
}

/// The redaction seam.
///
/// A trait object, following the same pattern as the gate seam in this repo:
/// the witness holds behaviour, never a concrete pipeline, so a test can
/// substitute one and a deployment can change the classifier backend without
/// touching this module.
///
/// It is fallible on purpose. [`DeterministicRedaction`] below never returns
/// `Err`, because the deterministic secret path cannot fail on well-formed
/// UTF-8; the prose-PII stage can and does -- `apply_privacy_filter_to_text`
/// propagates a self-hosted or NEAR AI backend failure rather than degrading
/// to deterministic-only -- and that failure must reach [`witness`] as a
/// refusal rather than as quietly narrower coverage.
#[async_trait]
pub trait TranscriptRedactor: Send + Sync {
    /// Redact `raw`. `Err` means the pass did not complete; there is no
    /// partial success.
    async fn redact(&self, raw: &str) -> Result<RedactedTranscript, SeamUnavailable>;
}

/// The deterministic secret path, and only that.
///
/// **This is not the full pipeline ingest runs**, and a certificate from a
/// witness wired this way says so: its `redaction_policy_version` is the
/// deterministic alias, and a server that requires the prose classifier must
/// refuse it. [`FullPipelineRedaction`] is the one that runs both stages.
///
/// This one is kept, rather than deleted in favour of it, because it is the
/// only implementation with no network dependency: it is what a test uses,
/// and what a deployment with no classifier backend reachable from inside the
/// enclave can honestly run.
pub struct DeterministicRedaction {
    redactor: DeterministicTraceRedactor,
}

impl DeterministicRedaction {
    /// Build the deterministic pass with the path prefixes it should treat as
    /// known. No env is read: a witness must not have its redaction policy
    /// changed by the environment it happens to boot into.
    pub fn new(known_path_prefixes: Vec<String>) -> Self {
        Self {
            redactor: DeterministicTraceRedactor::deterministic_only(known_path_prefixes),
        }
    }
}

#[async_trait]
impl TranscriptRedactor for DeterministicRedaction {
    async fn redact(&self, raw: &str) -> Result<RedactedTranscript, SeamUnavailable> {
        let (redacted, report) = self.redactor.redact_text(raw);
        Ok(RedactedTranscript {
            redacted,
            report,
            // Exactly what `redaction_pipeline_version(PrivacyFilterBackendTag::None)`
            // returns in the protocol crate, from the same public constant, so
            // the alias a server allowlists cannot drift from the one ingest
            // writes for the same pipeline.
            policy_version: DETERMINISTIC_REDACTION_PIPELINE_VERSION.to_string(),
        })
    }
}

/// Both stages: the deterministic secret pass, then the prose-PII classifier
/// over its output.
///
/// This is what closes the gap [`DeterministicRedaction`] leaves. It calls
/// `DeterministicTraceRedactor::redact_text_through_prose_filter`, the same
/// two stages in the same order that `TraceRedactor::redact_trace` applies to
/// every event ingest receives -- they share one private helper in the
/// protocol crate, so the pipeline a certificate attests cannot drift from the
/// one the server runs.
///
/// # This talks to the classifier over the network
///
/// [`TranscriptRedactor::redact`] is `async` for exactly this reason. Under
/// the `near-ai` backend the classifier is another host; under `self-hosted`
/// it is a loopback process. An enclave that cannot reach its configured
/// backend cannot run this redactor, and must not silently fall back to
/// [`DeterministicRedaction`] -- a backend failure surfaces as
/// [`SeamUnavailable`], which [`witness`] turns into a refusal.
///
/// # What the verdict does and does not cover
///
/// The report this returns is the originating pass's. The server's PII
/// backstop (`rescrub_envelope_prose_pii_with`) runs a further deterministic
/// sweep over the classifier's *output*, because the classifier is trained on
/// prose PII and can echo a credential back verbatim. That sweep has no
/// counterpart here, so a verdict derived from this report speaks for the
/// originating redaction, not for the backstop.
pub struct FullPipelineRedaction {
    redactor: DeterministicTraceRedactor,
    policy_version: String,
}

impl FullPipelineRedaction {
    /// Build the full pipeline from an explicitly supplied classifier
    /// adapter.
    ///
    /// The adapter is a parameter rather than something read from the
    /// environment here, for the same reason [`DeterministicRedaction::new`]
    /// reads no env: a witness must not have its redaction policy decided by
    /// the environment it happens to boot into. The binary resolves the
    /// backend once, at startup, and a witness that could not resolve one
    /// does not start.
    pub fn new(
        known_path_prefixes: Vec<String>,
        adapter: Arc<dyn PrivacyFilterAdapter>,
        backend: PrivacyFilterBackendTag,
    ) -> Self {
        Self {
            redactor: DeterministicTraceRedactor::deterministic_only(known_path_prefixes)
                .with_privacy_filter(adapter, backend),
            // From the protocol crate's own function, so the alias a server
            // allowlists is the one ingest writes for this same backend.
            policy_version: redaction_pipeline_version(backend),
        }
    }
}

#[async_trait]
impl TranscriptRedactor for FullPipelineRedaction {
    async fn redact(&self, raw: &str) -> Result<RedactedTranscript, SeamUnavailable> {
        // Fail-closed: a `near-ai` or `self-hosted` backend failure comes back
        // as `Err` from the protocol crate and becomes a refusal here. It is
        // never downgraded to the deterministic result, which would be a
        // certificate claiming coverage the pass did not have.
        let result = self
            .redactor
            .redact_text_through_prose_filter(raw)
            .await
            .map_err(|_| SeamUnavailable)?;
        Ok(RedactedTranscript {
            redacted: result.redacted,
            report: result.report,
            policy_version: self.policy_version.clone(),
        })
    }
}

/// The signing seam.
///
/// Synchronous: a dstack implementation fetches the raw private scalar once at
/// construction (`GetKey`, not the agent's `Sign`, which returns a 64-byte
/// `r || s` with no recovery byte that `recover_eip191_signer` cannot use) and
/// signs in-process thereafter. Nothing on this path should reach a socket per
/// signature.
pub trait Signer: Send + Sync {
    /// Sign `message` under EIP-191, returning 65 bytes of `0x`-prefixed hex
    /// with a 27/28 recovery byte -- the form
    /// `WitnessCertificate::verify` recovers from.
    fn sign_eip191(&self, message: &[u8]) -> Result<String, SeamUnavailable>;
}

/// The enclave seam: what the witness can say about the machine it runs on.
///
/// Declared here rather than with its dstack implementation because
/// [`witness`] consumes it as a trait object and would not compile otherwise.
#[async_trait]
pub trait Enclave: Send + Sync {
    /// The address that will sign the certificate this witness issues.
    ///
    /// Here rather than only on [`Signer`] because it is half of what a
    /// nonce-bound quote binds: a contributor compares this against the
    /// address `verify_witness_certificate` recovered, and a quote that named
    /// some other address would attest to a machine that did not sign what it
    /// is holding. Infallible -- a witness that could not name its own signer
    /// failed to start.
    fn signing_address(&self) -> &str;

    /// The measurement a contributor pins before sending anything, and the
    /// server pins before trusting a verdict. Fallible: a witness that cannot
    /// name itself must refuse, not certify anonymously.
    async fn measurement(&self) -> Result<String, SeamUnavailable>;

    /// A nonce-bound attestation quote over `report_data`.
    ///
    /// Not used by [`witness`] -- it is the other half of the surface, served
    /// so a client can verify the measurement *before* sending raw bytes.
    /// dstack accepts up to 64 bytes and zero-pads on the right; an
    /// implementation rejects anything longer rather than truncating.
    async fn attestation_quote(&self, report_data: &[u8]) -> Result<Vec<u8>, SeamUnavailable>;

    /// A quote bound to `nonce` **and** to this witness's signing address.
    ///
    /// Provided rather than required, and the one a surface should call: an
    /// HTTP handler that composed report data itself could compose it wrong,
    /// and the failure -- a quote that does not carry the caller's nonce -- is
    /// a replay that looks exactly like a success.
    async fn nonce_bound_quote(&self, nonce: &[u8]) -> Result<Vec<u8>, SeamUnavailable> {
        let report_data = enclave::witness_report_data(self.signing_address(), nonce)
            .map_err(|_| SeamUnavailable)?;
        self.attestation_quote(&report_data).await
    }
}

/// The residual-risk tier one condition implies.
///
/// The mapping is a `match` rather than [`ResidualRiskCondition::forces_high`]
/// plus an `else`, so that a new condition in the protocol crate is a compile
/// error here and gets a deliberate tier instead of silently inheriting
/// Medium. `condition_tiers_agree_with_the_protocol_crate` pins it against
/// `forces_high` so the two cannot drift apart.
fn condition_tier(condition: ResidualRiskCondition) -> ResidualPiiRisk {
    match condition {
        ResidualRiskCondition::KeyFinding
        | ResidualRiskCondition::CoverageIncomplete
        | ResidualRiskCondition::ResidualSurvivor
        | ResidualRiskCondition::ResidualScanUnavailable => ResidualPiiRisk::High,
        ResidualRiskCondition::FoundAndRemoved | ResidualRiskCondition::ConsentContentFlag => {
            ResidualPiiRisk::Medium
        }
    }
}

/// The verdict a basis implies: the highest tier any condition in it carries,
/// and `Low` when nothing held.
///
/// `ResidualPiiRisk` derives `Ord` in declaration order `Low < Medium < High`,
/// so `max` is the escalating fold and not a coincidence of spelling.
fn verdict_from_basis(basis: &[ResidualRiskCondition]) -> ResidualPiiRisk {
    basis
        .iter()
        .copied()
        .map(condition_tier)
        .fold(ResidualPiiRisk::Low, std::cmp::max)
}

/// Redact, judge, and certify.
///
/// The order of operations is the design:
///
/// 1. **Redact.** The witness performs the redaction; it never checks one.
/// 2. **Judge.** The verdict comes from [`residual_risk_basis`] in the
///    permissive crate -- the same function ingest runs. Not a
///    reimplementation: the certificate is worth something precisely because
///    a known program reached the verdict the server *would have*, and two
///    implementations drift.
/// 3. **Bind.** [`check_correspondence`] over the artifact being returned is
///    the only way to obtain the [`CorrespondenceProof`] that
///    [`WitnessCertificate::from_proof`] requires, so the digest is of those
///    bytes and no others.
/// 4. **Certify.** Name the enclave, stamp the time, sign.
///
/// Every step that cannot complete refuses with a named [`WitnessError`].
/// There is no arm that certifies what it has so far.
///
/// The redactor is a parameter alongside the signer and the enclave. It has
/// to be: `RedactionFailed` is the fail-closed arm this function exists
/// around, and a redactor constructed inside here could not be made to fail,
/// so the guard would be untestable and the deployment's classifier backend
/// would be decided by whatever env the process booted into.
///
/// [`CorrespondenceProof`]: crate::redaction_witness::correspondence::CorrespondenceProof
pub async fn witness(
    request: WitnessRequest,
    redactor: &dyn TranscriptRedactor,
    signer: &dyn Signer,
    enclave: &dyn Enclave,
) -> Result<WitnessResponse, WitnessError> {
    let RedactedTranscript {
        redacted,
        report,
        policy_version,
    } = redactor
        .redact(&request.raw_transcript)
        .await
        .map_err(|SeamUnavailable| WitnessError::RedactionFailed)?;

    // `None`: this witness runs no detection-only residual scan, so it has no
    // residual findings to report. That is not the same as a scan that came
    // back clean, and the module doc says what a server is choosing to skip
    // when it trusts a pass verdict.
    let basis = residual_risk_basis(&request.consent, &report, None);
    let residual_risk_verdict = verdict_from_basis(&basis);

    // Binds the digest to the bytes returned below and nothing else. The
    // empty span list is not a shortcut around a check -- the witness
    // produced this redaction, so there is no client claim to check -- and
    // the module doc records why a span-bearing check is unavailable here.
    let proof = check_correspondence(&redacted, &redacted, &[])
        .map_err(|_| WitnessError::ArtifactBindingFailed)?;

    let certificate = WitnessCertificate::from_proof(
        proof,
        CertificateDetails {
            residual_risk_verdict,
            redaction_policy_version: policy_version,
            witness_measurement: enclave
                .measurement()
                .await
                .map_err(|SeamUnavailable| WitnessError::MeasurementUnavailable)?,
            timestamp: chrono::Utc::now().timestamp(),
        },
    );

    let signature_hex = signer
        .sign_eip191(&certificate.signing_bytes())
        .map_err(|SeamUnavailable| WitnessError::SigningUnavailable)?;

    Ok(WitnessResponse {
        redacted_artifact: redacted,
        certificate,
        signature_hex,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::redaction_witness::verification::{WitnessPin, verify_witness_certificate};
    use k256::ecdsa::SigningKey;
    use sha2::{Digest, Sha256};
    use sha3::Keccak256;
    use trace_commons_protocol::trace_contribution::ConsentScope;

    /// A secret the deterministic pass is guaranteed to find: it matches the
    /// `aws_access_key` pattern exactly. Distinctive enough that searching a
    /// serialized response for it cannot match by accident.
    // Split so the twenty-character form never appears verbatim in the
    // source. The value is synthetic -- a keyboard walk, not a
    // credential -- but GitHub push protection matches the shape, and it
    // is right to: a scanner that trusted our word about which
    // AKIA-prefixed strings are fake would be useless. Our own detector
    // requires the prefix, so the fixture cannot avoid it; splitting the
    // literal is the honest way to keep both checks working.
    const SECRET: &str = concat!("AKIA", "QQWERTYUIOPASDFG");

    /// A token that is not secret-shaped and must therefore SURVIVE the
    /// redaction. The positive control for the leak test: without it, a
    /// redactor that returned the empty string would pass every "the secret
    /// is absent" assertion in this file.
    const SURVIVOR: &str = "zzq-control-token-zzq";

    /// A marker fed to a request that is going to be refused. Nothing may
    /// echo it -- not the error, not its `Debug`.
    const REFUSED_MARKER: &str = "zzq-refused-marker-zzq";

    /// The measurement the honest enclave reports and the pin admits.
    const MEASUREMENT: &str = "c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2";

    fn consent(message_text_included: bool) -> ConsentMetadata {
        ConsentMetadata {
            policy_version: "consent-v1".to_string(),
            scopes: vec![ConsentScope::DebuggingEvaluation],
            message_text_included,
            tool_payloads_included: false,
            correction_included: false,
            routing_metadata_included: false,
            revocable: true,
        }
    }

    fn request(raw: &str, message_text_included: bool) -> WitnessRequest {
        WitnessRequest {
            raw_transcript: raw.to_string(),
            consent: consent(message_text_included),
        }
    }

    /// The production redaction seam, with no known path prefixes so that
    /// `local_path` -- present in 93% of real sessions and deliberately
    /// non-severity-bearing -- cannot be what a verdict assertion is
    /// observing.
    fn redactor() -> DeterministicRedaction {
        DeterministicRedaction::new(Vec::new())
    }

    struct TestSigner(SigningKey);

    impl TestSigner {
        fn new(seed: &str) -> Self {
            let bytes = Keccak256::digest(seed.as_bytes());
            Self(SigningKey::from_slice(&bytes).expect("seed is a valid scalar"))
        }

        fn address(&self) -> String {
            let point = self.0.verifying_key().to_encoded_point(false);
            let digest = Keccak256::digest(&point.as_bytes()[1..]);
            format!("0x{}", hex::encode(&digest[12..]))
        }
    }

    impl Signer for TestSigner {
        fn sign_eip191(&self, message: &[u8]) -> Result<String, SeamUnavailable> {
            let mut hasher = Keccak256::new();
            hasher.update(b"\x19Ethereum Signed Message:\n");
            hasher.update(message.len().to_string().as_bytes());
            hasher.update(message);
            let digest: [u8; 32] = hasher.finalize().into();
            let (signature, recovery_id) = self
                .0
                .sign_prehash_recoverable(&digest)
                .expect("the digest is 32 bytes");
            let mut raw = signature.to_bytes().to_vec();
            raw.push(recovery_id.to_byte() + 27);
            Ok(format!("0x{}", hex::encode(raw)))
        }
    }

    struct RefusingSigner;

    impl Signer for RefusingSigner {
        fn sign_eip191(&self, _message: &[u8]) -> Result<String, SeamUnavailable> {
            Err(SeamUnavailable)
        }
    }

    /// The address the enclave doubles report. Production unites the signing
    /// and enclave seams in one `DstackEnclave`; the doubles split them, so
    /// this is a constant rather than the `TestSigner`'s address.
    const ENCLAVE_ADDRESS: &str = "0x1111111111111111111111111111111111111111";

    struct TestEnclave;

    #[async_trait]
    impl Enclave for TestEnclave {
        fn signing_address(&self) -> &str {
            ENCLAVE_ADDRESS
        }

        async fn measurement(&self) -> Result<String, SeamUnavailable> {
            Ok(MEASUREMENT.to_string())
        }

        async fn attestation_quote(&self, _report_data: &[u8]) -> Result<Vec<u8>, SeamUnavailable> {
            Ok(vec![0xab; 8])
        }
    }

    struct SilentEnclave;

    #[async_trait]
    impl Enclave for SilentEnclave {
        fn signing_address(&self) -> &str {
            ENCLAVE_ADDRESS
        }

        async fn measurement(&self) -> Result<String, SeamUnavailable> {
            Err(SeamUnavailable)
        }

        async fn attestation_quote(&self, _report_data: &[u8]) -> Result<Vec<u8>, SeamUnavailable> {
            Err(SeamUnavailable)
        }
    }

    /// A stand-in prose classifier. It removes a person's name, which the
    /// deterministic regex suite does not touch, so its effect on the output
    /// is distinguishable from the deterministic stage's.
    struct NameRemovingFilter;

    #[async_trait]
    impl trace_commons_protocol::trace_contribution::PrivacyFilterAdapter for NameRemovingFilter {
        async fn redact_text(
            &self,
            text: &str,
        ) -> Result<
            Option<trace_commons_protocol::trace_contribution::SafePrivacyFilterRedaction>,
            trace_commons_protocol::trace_contribution::TraceContributionError,
        > {
            Ok(Some(
                trace_commons_protocol::trace_contribution::SafePrivacyFilterRedaction {
                    redacted_text: text.replace(CLASSIFIER_ONLY_PII, "<PROSE_NAME>"),
                    summary: Default::default(),
                    report: RedactionReport::default(),
                },
            ))
        }
    }

    /// A classifier backend that is down.
    struct UnavailableFilter;

    #[async_trait]
    impl trace_commons_protocol::trace_contribution::PrivacyFilterAdapter for UnavailableFilter {
        async fn redact_text(
            &self,
            _text: &str,
        ) -> Result<
            Option<trace_commons_protocol::trace_contribution::SafePrivacyFilterRedaction>,
            trace_commons_protocol::trace_contribution::TraceContributionError,
        > {
            Err(
                trace_commons_protocol::trace_contribution::TraceContributionError::RedactionFailed {
                    reason: "classifier down".to_string(),
                },
            )
        }
    }

    /// Prose PII the deterministic pass leaves alone.
    const CLASSIFIER_ONLY_PII: &str = "Alice Brannigan";

    /// The full-pipeline redactor runs BOTH stages. Asserted on the two
    /// findings separately, because a redactor that ran only one of them
    /// would still satisfy an assertion about the other.
    #[tokio::test]
    async fn the_full_pipeline_redactor_runs_the_classifier_and_the_deterministic_pass() {
        let redactor = FullPipelineRedaction::new(
            Vec::new(),
            Arc::new(NameRemovingFilter),
            PrivacyFilterBackendTag::SelfHosted,
        );
        let raw = format!("{CLASSIFIER_ONLY_PII} deployed {SECRET} and kept {SURVIVOR}");

        let result = redactor.redact(&raw).await.expect("both stages succeed");

        assert!(
            !result.redacted.contains(SECRET),
            "deterministic stage did not run: {}",
            result.redacted
        );
        assert!(
            !result.redacted.contains(CLASSIFIER_ONLY_PII),
            "classifier stage did not run: {}",
            result.redacted
        );
        assert!(
            result.redacted.contains(SURVIVOR),
            "a redactor that emptied the text would pass the two assertions \
             above; it must not: {}",
            result.redacted
        );
        assert!(
            result.report.blocked_secret_detected,
            "the report must carry what the pass found: {:?}",
            result.report
        );
    }

    /// The policy alias names the backend that actually ran, and comes from
    /// the protocol crate's own function rather than a string assembled here.
    #[tokio::test]
    async fn the_full_pipeline_policy_alias_names_the_configured_backend() {
        for backend in [
            PrivacyFilterBackendTag::SelfHosted,
            PrivacyFilterBackendTag::NearAi,
            PrivacyFilterBackendTag::Sidecar,
        ] {
            let redactor =
                FullPipelineRedaction::new(Vec::new(), Arc::new(NameRemovingFilter), backend);
            let result = redactor.redact("nothing here").await.expect("succeeds");
            assert_eq!(
                result.policy_version,
                redaction_pipeline_version(backend),
                "alias must come from the protocol crate for {backend:?}"
            );
            assert_ne!(
                result.policy_version, DETERMINISTIC_REDACTION_PIPELINE_VERSION,
                "a full-pipeline witness must not report the deterministic \
                 alias for {backend:?}"
            );
        }
    }

    /// Fail-closed: a classifier backend that is down makes the witness
    /// refuse. It must NEVER hand back the deterministic-only result, which
    /// would be a certificate claiming coverage the pass did not have.
    #[tokio::test]
    async fn a_down_classifier_refuses_rather_than_degrading() {
        for backend in [
            PrivacyFilterBackendTag::SelfHosted,
            PrivacyFilterBackendTag::NearAi,
        ] {
            let redactor =
                FullPipelineRedaction::new(Vec::new(), Arc::new(UnavailableFilter), backend);
            let outcome = redactor.redact(&format!("deployed {SECRET}")).await;
            assert!(
                outcome.is_err(),
                "{backend:?}: a down classifier must refuse, not degrade to \
                 the deterministic pass"
            );
        }
    }

    struct RefusingRedactor;

    #[async_trait]
    impl TranscriptRedactor for RefusingRedactor {
        async fn redact(&self, _raw: &str) -> Result<RedactedTranscript, SeamUnavailable> {
            Err(SeamUnavailable)
        }
    }

    /// A redactor that reports a coverage gap. The only way to reach a High
    /// verdict here: `coverage_incomplete` is set when a configured filter
    /// was unavailable or skipped content, and the deterministic pass over
    /// well-formed UTF-8 never sets it.
    struct CoverageGapRedactor;

    #[async_trait]
    impl TranscriptRedactor for CoverageGapRedactor {
        async fn redact(&self, raw: &str) -> Result<RedactedTranscript, SeamUnavailable> {
            let report = RedactionReport {
                coverage_incomplete: true,
                ..RedactionReport::default()
            };
            Ok(RedactedTranscript {
                redacted: raw.to_string(),
                report,
                policy_version: "coverage-gap-test-policy".to_string(),
            })
        }
    }

    fn pin(signer: &TestSigner) -> WitnessPin {
        WitnessPin::new(&signer.address(), [MEASUREMENT.to_string()])
            .expect("the address and measurement are well formed")
    }

    #[tokio::test]
    async fn a_witnessed_artifact_and_its_certificate_agree() {
        let signer = TestSigner::new("witness-agreement");
        let response = witness(
            request(&format!("deploying {SURVIVOR} with {SECRET}"), false),
            &redactor(),
            &signer,
            &TestEnclave,
        )
        .await
        .expect("the witness certifies");

        // Ground truth from outside: the digest is checked against the bytes
        // the response actually returns, not against anything the witness
        // computed internally. A certificate over a DIFFERENT artifact is the
        // failure mode the whole design exists to prevent, and
        // `verify_witness_certificate` is the server's own check, so this
        // exercises the real consumer rather than a restatement of it.
        let verified = verify_witness_certificate(
            response.certificate.clone(),
            &response.signature_hex,
            Some(&pin(&signer)),
            response.redacted_artifact.as_bytes(),
        )
        .expect("the certificate verifies against the artifact it was returned with");

        assert_eq!(
            verified.redacted_sha256(),
            hex::encode(Sha256::digest(response.redacted_artifact.as_bytes())),
            "the certificate's digest must be of the returned bytes"
        );
        assert_eq!(verified.witness_measurement(), MEASUREMENT);
    }

    #[tokio::test]
    async fn the_verdict_matches_what_the_shared_function_says() {
        // Three cases through the production redactor, chosen so a constant
        // verdict cannot satisfy them: an empty basis, the found-and-removed
        // floor, and the consent-flag floor. The expected basis is asserted
        // against `residual_risk_basis` run directly on the same input --
        // ground truth from the permissive crate, outside the code under
        // test -- and the expected verdict is a literal, so neither the
        // shared function nor this module's mapping can drift unnoticed.
        let cases: [(&str, bool, Vec<ResidualRiskCondition>, ResidualPiiRisk); 3] = [
            (
                "the build finished and the tests passed",
                false,
                Vec::new(),
                ResidualPiiRisk::Low,
            ),
            (
                // Split for the reason given at SECRET above.
                concat!("exported AWS_ACCESS_KEY_ID=", "AKIA", "QQWERTYUIOPASDFG"),
                false,
                vec![ResidualRiskCondition::FoundAndRemoved],
                ResidualPiiRisk::Medium,
            ),
            (
                "the build finished and the tests passed",
                true,
                vec![ResidualRiskCondition::ConsentContentFlag],
                ResidualPiiRisk::Medium,
            ),
        ];

        // Collected and compared whole, not asserted inside the loop. An
        // in-loop `assert_eq!` short-circuits, so the first divergent case
        // masks every later one -- and a mutation that gets exactly one case
        // wrong would look identical to a mutation that gets all of them
        // wrong. Comparing the full vectors reports every case.
        let mut actual_bases = Vec::new();
        let mut expected_bases = Vec::new();
        let mut actual_verdicts = Vec::new();
        let mut expected_verdicts = Vec::new();

        for (raw, message_text_included, expected_basis, expected_verdict) in cases {
            let consent = consent(message_text_included);
            let (_, report) =
                DeterministicTraceRedactor::deterministic_only(Vec::new()).redact_text(raw);
            actual_bases.push(residual_risk_basis(&consent, &report, None));
            expected_bases.push(expected_basis);

            let response = witness(
                request(raw, message_text_included),
                &redactor(),
                &TestSigner::new("witness-verdict"),
                &TestEnclave,
            )
            .await
            .expect("the witness certifies");
            actual_verdicts.push(response.residual_risk_verdict());
            expected_verdicts.push(expected_verdict);
        }

        // The High tier, unreachable through the deterministic pass because
        // it never sets a coverage gap. Without this case a mapping that
        // never returns High would satisfy every assertion above.
        let response = witness(
            request("the build finished and the tests passed", false),
            &CoverageGapRedactor,
            &TestSigner::new("witness-verdict"),
            &TestEnclave,
        )
        .await
        .expect("the witness certifies");
        actual_verdicts.push(response.residual_risk_verdict());
        expected_verdicts.push(ResidualPiiRisk::High);

        assert_eq!(
            actual_bases, expected_bases,
            "the shared function's basis changed for at least one case"
        );
        assert_eq!(
            actual_verdicts, expected_verdicts,
            "the witness verdict diverged from the shared function for at least one case"
        );
    }

    #[test]
    fn condition_tiers_agree_with_the_protocol_crate() {
        // Pins this module's `match` against `forces_high` in the permissive
        // crate. Iterating `ALL` rather than a list written here is what
        // makes a newly added condition reach this test at all.
        for condition in ResidualRiskCondition::ALL.iter().copied() {
            let expected = if condition.forces_high() {
                ResidualPiiRisk::High
            } else {
                ResidualPiiRisk::Medium
            };
            assert_eq!(
                condition_tier(condition),
                expected,
                "tier for {condition:?} disagrees with forces_high"
            );
        }
    }

    #[tokio::test]
    async fn raw_bytes_never_appear_in_a_response_or_an_error() {
        let signer = TestSigner::new("witness-leak");
        let raw = format!("session start\n{SURVIVOR}\nexport KEY={SECRET}\nsession end");

        let response = witness(request(&raw, false), &redactor(), &signer, &TestEnclave)
            .await
            .expect("the witness certifies");

        // The positive control first: if this fails, every absence assertion
        // below is satisfied by a redactor that returned nothing, and the
        // test would have been proving that.
        assert!(
            response.redacted_artifact.contains(SURVIVOR),
            "non-secret content must survive, or the absence checks below are vacuous"
        );

        // Every surface the success path hands a caller, searched for the
        // secret itself rather than asserted about which function ran.
        // Named, and every leaking surface collected before asserting: a
        // `for` loop of `assert!` stops at the first one and would report a
        // single leak where there were four.
        let surfaces = [
            ("redacted_artifact", response.redacted_artifact.clone()),
            ("Debug of the response", format!("{response:?}")),
            (
                "Debug of the certificate",
                format!("{:?}", response.certificate),
            ),
            ("signature_hex", response.signature_hex.clone()),
        ];
        let leaking: Vec<&str> = surfaces
            .iter()
            .filter(|(_, rendered)| rendered.contains(SECRET))
            .map(|(name, _)| *name)
            .collect();
        assert!(
            leaking.is_empty(),
            "witness surfaces carried the raw secret: {leaking:?}"
        );

        // And the refusal path. A refused request returns nothing at all, so
        // the marker here is ordinary text rather than a secret: if any part
        // of the input can reach an error rendering, this catches it.
        let refused = witness(
            request(&format!("plain text {REFUSED_MARKER}"), false),
            &RefusingRedactor,
            &signer,
            &TestEnclave,
        )
        .await
        .expect_err("the witness refuses");
        let error_surfaces = [
            ("Display", format!("{refused}")),
            ("Debug", format!("{refused:?}")),
        ];
        let leaking: Vec<&str> = error_surfaces
            .iter()
            .filter(|(_, rendered)| rendered.contains(REFUSED_MARKER) || rendered.contains(SECRET))
            .map(|(name, _)| *name)
            .collect();
        assert!(
            leaking.is_empty(),
            "witness error renderings carried request content: {leaking:?}"
        );
    }

    #[tokio::test]
    async fn a_redaction_failure_refuses_rather_than_certifying_what_it_has() {
        let error = witness(
            request(&format!("export KEY={SECRET}"), false),
            &RefusingRedactor,
            &TestSigner::new("witness-refusal"),
            &TestEnclave,
        )
        .await
        .expect_err("a redaction that did not complete cannot be certified");
        assert_eq!(error, WitnessError::RedactionFailed);
    }

    #[tokio::test]
    async fn an_enclave_that_cannot_name_itself_refuses() {
        // A certificate naming no program is one an operator's pin can never
        // match, and signing it would put an unpinnable certificate into
        // circulation.
        let error = witness(
            request("the build finished", false),
            &redactor(),
            &TestSigner::new("witness-refusal"),
            &SilentEnclave,
        )
        .await
        .expect_err("a witness that cannot name itself cannot certify");
        assert_eq!(error, WitnessError::MeasurementUnavailable);
    }

    #[tokio::test]
    async fn a_signer_that_refuses_yields_no_response() {
        let error = witness(
            request("the build finished", false),
            &redactor(),
            &RefusingSigner,
            &TestEnclave,
        )
        .await
        .expect_err("an unsigned certificate is not a certificate");
        assert_eq!(error, WitnessError::SigningUnavailable);
    }
}
