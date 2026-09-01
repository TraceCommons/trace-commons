// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The attestation drill: the one thing that makes the four verified pieces
//! mean something together.
//!
//! Each piece on its own proves less than it looks like it does.
//!
//! - A verified quote proves *a* TDX enclave running *a* measured image
//!   exists. It says nothing about the endpoint that answered us.
//! - A nonce inside that quote proves the quote is fresh rather than replayed.
//! - Pinned measurements prove the image is the one we reviewed.
//! - A verified receipt proves *some* key signed over a specific request and
//!   response.
//!
//! The join is the last step, and it is the one that is easy to leave out:
//! **the key that signed the receipt must be the key the quote attests.**
//! Without it a valid attestation and a valid receipt verify independently
//! and prove nothing together -- an endpoint can proxy someone else's genuine
//! attestation report while signing receipts with a key of its own, and every
//! individual check still passes. That substitution is the attack this drill
//! exists to catch, and
//! [`NearAttestationDrillStep::ReceiptSignerIsAttestedKey`] is where it is
//! caught.
//!
//! Two rules the drill holds that a verifier deliberately does not:
//!
//! - **TCB status must be `UpToDate`.** [`super::quote::verify_quote`]
//!   returns the status without enforcing it, which is right: a verifier
//!   returns data and policy belongs to the caller. This caller's policy is
//!   that anything else is a failure, named in evidence. There is no
//!   configurable allow-list, because an allow-list is the lever someone
//!   pulls to make a red drill green.
//! - **Nothing pinned is a failure, not a skip.**
//!   [`super::measurements::check_measurements_opt`] already returns
//!   `Refused`; the drill treats it as a failed step.
//!
//! **Evidence is hash-only.** Measurement registers, the TCB status and the
//! nonce are all public values and appear in full. The API key, the
//! completion text, the receipt and the signing address never do -- the
//! address appears only as a digest, and only so that two runs can be
//! compared.
//!
//! **Step 5 spends money**, so the drill refuses to reach it if any earlier
//! step failed. A completion against an endpoint we have not established is
//! the enclave we think it is buys nothing.

use serde::Serialize;
use sha2::{Digest, Sha256};

use super::client::{AttestationClient, AttestationClientError, AttestationStep};
use super::measurements::{
    ExpectedMeasurements, MeasurementField, MeasurementVerdict, check_measurements_opt,
    json_claim_anomalies,
};
use super::quote::{VerifiedQuote, verify_quote};
use super::receipt::{ReceiptError, verify_receipt};

/// The TCB verdict the drill accepts. Anything else fails.
pub const REQUIRED_TCB_STATUS: &str = "UpToDate";

/// The prompt the paid completion sends. Kept to one word deliberately.
const DRILL_PROMPT: &str = "ping";

/// Tokens the paid completion may generate. Kept to one deliberately.
const DRILL_MAX_TOKENS: u32 = 1;

/// A step of the drill. The order is the order they run in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NearAttestationDrillStep {
    /// The endpoint served an attestation report for our nonce.
    ReportFetched,
    /// The quote in that report verified against Intel collateral.
    QuoteVerified,
    /// Intel's TCB verdict for the platform is [`REQUIRED_TCB_STATUS`].
    TcbUpToDate,
    /// Our nonce is in the verified quote's report data, at the offset NEAR
    /// AI documents.
    NonceBoundInQuote,
    /// `report_data[20..32]` are zero, i.e. the report was served in the
    /// default mode where `[0..20]` is the raw signing address rather than
    /// the `include_tls_fingerprint` mode where all 32 bytes are a hash.
    SignerBindingDefaultMode,
    /// Every pinned measurement register matched.
    MeasurementsPinned,
    /// One minimal completion succeeded. **Costs money.**
    CompletionPerformed,
    /// Its receipt verified against the exact request bytes and the response
    /// text.
    ReceiptVerified,
    /// The receipt's recovered signer is the address the quote attests.
    ReceiptSignerIsAttestedKey,
}

impl NearAttestationDrillStep {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReportFetched => "report_fetched",
            Self::QuoteVerified => "quote_verified",
            Self::TcbUpToDate => "tcb_up_to_date",
            Self::NonceBoundInQuote => "nonce_bound_in_quote",
            Self::SignerBindingDefaultMode => "signer_binding_default_mode",
            Self::MeasurementsPinned => "measurements_pinned",
            Self::CompletionPerformed => "completion_performed",
            Self::ReceiptVerified => "receipt_verified",
            Self::ReceiptSignerIsAttestedKey => "receipt_signer_is_attested_key",
        }
    }

    /// Every step, in run order. Used to render a full result even when the
    /// drill stopped early.
    pub const ALL: [NearAttestationDrillStep; 9] = [
        Self::ReportFetched,
        Self::QuoteVerified,
        Self::TcbUpToDate,
        Self::NonceBoundInQuote,
        Self::SignerBindingDefaultMode,
        Self::MeasurementsPinned,
        Self::CompletionPerformed,
        Self::ReceiptVerified,
        Self::ReceiptSignerIsAttestedKey,
    ];
}

/// How a step ended.
///
/// `NotRun` is deliberately not `Passed`. A drill that stopped before the
/// paid completion has not proved anything about the completion, and a
/// three-state result is what stops that reading as a green tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NearAttestationStepStatus {
    Passed,
    Failed,
    NotRun,
}

/// One step's outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NearAttestationStepResult {
    pub step: NearAttestationDrillStep,
    pub status: NearAttestationStepStatus,
    /// A stable label naming the failure, never a message. `None` unless the
    /// step failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// The missing control's name, when the failure is a missing control.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub missing_control: Option<String>,
}

/// What the measurement check saw.
///
/// Register values are public image identifiers, so they appear in full: an
/// operator holding a mismatch's two halves can go straight to the image,
/// which is the point of reporting it at all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NearAttestationMeasurementEvidence {
    /// `pinned`, `mismatch`, or `refused`.
    pub verdict: String,
    pub checked_fields: Vec<String>,
    pub mismatched_fields: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub missing_control: Option<String>,
}

/// A register where the report's unsigned JSON disagrees with the quote.
///
/// Reporting only. The trustworthy value is always the quote's, and
/// [`NearAttestationDrillStep::MeasurementsPinned`] already compares against
/// it; a disagreement here means the endpoint describes itself inaccurately
/// in a way the quote exposes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NearAttestationJsonClaimAnomaly {
    pub field: String,
    pub claimed: String,
    pub verified: String,
}

/// The drill's full result, and the thing that gets hashed into evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NearAttestationDrillOutcome {
    /// The nonce we chose. Public by construction -- we generated it and put
    /// it in a query string.
    pub nonce: String,
    /// True only when every step passed.
    pub passed: bool,
    pub steps: Vec<NearAttestationStepResult>,
    /// Intel's TCB verdict, once the quote verified.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tcb_status: Option<String>,
    /// Intel advisory ids attached to that verdict.
    pub advisory_ids: Vec<String>,
    /// Verified MRTD, once the quote verified.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mrtd: Option<String>,
    /// Verified RTMR0..3, once the quote verified.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rtmr: Option<[String; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub measurements: Option<NearAttestationMeasurementEvidence>,
    /// Where the report's unsigned JSON disagrees with the quote. Reporting
    /// only.
    pub json_claim_anomalies: Vec<NearAttestationJsonClaimAnomaly>,
    /// Whether the report's unsigned `signing_address` field agrees with the
    /// address the quote attests. Reporting only, for the same reason.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report_json_signing_address_agrees: Option<bool>,
    /// Digest of the address the quote attests. Never the address.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attested_signer_ref: Option<String>,
    /// Digest of the address recovered from the receipt. Never the address.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receipt_signer_ref: Option<String>,
    /// Digest of the completion id. Never the id, which is a handle to the
    /// completion's content on the provider side.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chat_id_ref: Option<String>,
    /// Whether the paid completion was performed. False whenever an earlier
    /// step failed.
    pub completion_charged: bool,
}

impl NearAttestationDrillOutcome {
    /// The steps that did not pass, named. Empty exactly when `passed`.
    pub fn blocking_steps(&self) -> Vec<String> {
        self.steps
            .iter()
            .filter(|result| result.status != NearAttestationStepStatus::Passed)
            .map(|result| match &result.reason {
                Some(reason) => format!("{}:{reason}", result.step.as_str()),
                None => result.step.as_str().to_string(),
            })
            .collect()
    }
}

/// A `sha256:`-prefixed digest, for values that must never appear in full.
fn secret_ref(value: &str) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(value.as_bytes())))
}

/// The exact bytes the drill POSTs as its completion.
///
/// Exposed because the caller must keep these bytes: the receipt binds
/// `SHA256(request_body_as_sent)`, and re-serializing from a parsed form
/// changes the digest.
pub fn drill_completion_request_body(model: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "model": model,
        "messages": [{ "role": "user", "content": DRILL_PROMPT }],
        "max_tokens": DRILL_MAX_TOKENS,
        "temperature": 0,
        "stream": false,
    }))
    .expect("a fixed JSON object always serializes")
}

/// A fresh 32-byte nonce, lowercase hex.
pub fn generate_drill_nonce() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Accumulates step results so a drill that stops early still renders every
/// step, with the ones it never reached marked `NotRun`.
struct StepLog {
    results: Vec<NearAttestationStepResult>,
}

impl StepLog {
    fn new() -> Self {
        Self {
            results: Vec::new(),
        }
    }

    fn pass(&mut self, step: NearAttestationDrillStep) {
        self.results.push(NearAttestationStepResult {
            step,
            status: NearAttestationStepStatus::Passed,
            reason: None,
            missing_control: None,
        });
    }

    fn fail(&mut self, step: NearAttestationDrillStep, reason: impl Into<String>) {
        self.results.push(NearAttestationStepResult {
            step,
            status: NearAttestationStepStatus::Failed,
            reason: Some(reason.into()),
            missing_control: None,
        });
    }

    fn fail_missing_control(&mut self, step: NearAttestationDrillStep, control: &str) {
        self.results.push(NearAttestationStepResult {
            step,
            status: NearAttestationStepStatus::Failed,
            reason: Some(format!("missing_control:{control}")),
            missing_control: Some(control.to_string()),
        });
    }

    fn fail_client(&mut self, step: NearAttestationDrillStep, error: &AttestationClientError) {
        match error.missing_control() {
            Some(control) => self.fail_missing_control(step, control),
            None => self.fail(step, error.code()),
        }
    }

    /// Fill in every step that never ran, in canonical order.
    fn finish(mut self) -> Vec<NearAttestationStepResult> {
        let seen: Vec<NearAttestationDrillStep> =
            self.results.iter().map(|result| result.step).collect();
        for step in NearAttestationDrillStep::ALL {
            if !seen.contains(&step) {
                self.results.push(NearAttestationStepResult {
                    step,
                    status: NearAttestationStepStatus::NotRun,
                    reason: None,
                    missing_control: None,
                });
            }
        }
        self.results.sort_by_key(|result| position_of(result.step));
        self.results
    }
}

fn position_of(step: NearAttestationDrillStep) -> usize {
    NearAttestationDrillStep::ALL
        .iter()
        .position(|candidate| *candidate == step)
        .expect("ALL contains every step")
}

/// Run the drill.
///
/// `now_unix` is the clock the quote's collateral is evaluated against.
/// Production callers pass real wall-clock time; it is a parameter only so
/// tests can pin a captured fixture to the day it was captured.
pub async fn run_near_attestation_drill(
    client: &dyn AttestationClient,
    expected: Option<&ExpectedMeasurements>,
    nonce: &str,
    now_unix: u64,
) -> NearAttestationDrillOutcome {
    let mut log = StepLog::new();
    let mut outcome = NearAttestationDrillOutcome {
        nonce: nonce.to_string(),
        passed: false,
        steps: Vec::new(),
        tcb_status: None,
        advisory_ids: Vec::new(),
        mrtd: None,
        rtmr: None,
        measurements: None,
        json_claim_anomalies: Vec::new(),
        report_json_signing_address_agrees: None,
        attested_signer_ref: None,
        receipt_signer_ref: None,
        chat_id_ref: None,
        completion_charged: false,
    };

    // 1. The report.
    let report = match client.fetch_report(nonce).await {
        Ok(report) => {
            log.pass(NearAttestationDrillStep::ReportFetched);
            report
        }
        Err(error) => {
            debug_assert_eq!(error.step(), AttestationStep::Report);
            log.fail_client(NearAttestationDrillStep::ReportFetched, &error);
            return finish(outcome, log);
        }
    };

    let quote_bytes = match report.quote_bytes() {
        Ok(bytes) => bytes,
        Err(_) => {
            log.fail(NearAttestationDrillStep::QuoteVerified, "quote_not_hex");
            return finish(outcome, log);
        }
    };

    // 2. The quote, against Intel collateral. This is what makes the nonce
    //    binding and the measurements mean anything.
    let collateral = match client.fetch_collateral(&quote_bytes).await {
        Ok(collateral) => collateral,
        Err(error) => {
            log.fail_client(NearAttestationDrillStep::QuoteVerified, &error);
            return finish(outcome, log);
        }
    };
    let verified: VerifiedQuote = match verify_quote(&quote_bytes, &collateral, now_unix) {
        Ok(verified) => {
            log.pass(NearAttestationDrillStep::QuoteVerified);
            verified
        }
        Err(error) => {
            // The variant name, not the message: the message can carry a
            // detail hash but nothing here needs it.
            let label = match error {
                super::quote::QuoteVerifyError::CollateralMalformed { .. } => {
                    "collateral_malformed"
                }
                super::quote::QuoteVerifyError::VerificationFailed { .. } => "verification_failed",
                super::quote::QuoteVerifyError::NotTdx => "not_tdx",
            };
            log.fail(NearAttestationDrillStep::QuoteVerified, label);
            return finish(outcome, log);
        }
    };

    outcome.tcb_status = Some(verified.tcb_status.clone());
    outcome.advisory_ids = verified.advisory_ids.clone();
    outcome.mrtd = Some(verified.mrtd.clone());
    outcome.rtmr = Some(verified.rtmr.clone());

    // 3. Intel's TCB verdict. Policy, not verification -- see the module docs.
    match tcb_step_reason(&verified.tcb_status) {
        None => log.pass(NearAttestationDrillStep::TcbUpToDate),
        Some(reason) => log.fail(NearAttestationDrillStep::TcbUpToDate, reason),
    }

    // 4. Our nonce, inside the signed quote.
    //
    // The authoritative check is the *offset*: NEAR AI writes the requested
    // nonce at report_data[32..64]. `quote_binds_nonce` searches the whole
    // quote for the bytes, which is weaker -- three quarters of a quote is
    // not covered by its signature, so a match anywhere else would prove
    // nothing. Both are required, so a future change that verifies a
    // different quote than the one parsed shows up here.
    if verified.report_data.len() < 64 {
        log.fail(
            NearAttestationDrillStep::NonceBoundInQuote,
            "report_data_too_short",
        );
    } else if hex::encode(&verified.report_data[32..64]) != nonce.to_ascii_lowercase() {
        log.fail(
            NearAttestationDrillStep::NonceBoundInQuote,
            "nonce_not_in_report_data",
        );
    } else if !report.quote_binds_nonce(nonce).unwrap_or(false) {
        log.fail(
            NearAttestationDrillStep::NonceBoundInQuote,
            "nonce_not_in_quote_bytes",
        );
    } else {
        log.pass(NearAttestationDrillStep::NonceBoundInQuote);
    }

    // 5. The signer-binding mode.
    //
    // In the default mode report_data[0..20] is the raw signing address and
    // [20..32] are zero. With `include_tls_fingerprint=true` all 32 bytes are
    // SHA256(signing_address || spki_hash) instead. This drill fetches in
    // default mode and compares the address; if the fetch ever grows that
    // flag, the padding stops being zero and this fails loudly rather than
    // comparing an address against the first 20 bytes of a hash.
    let attested_address = match attested_signer_address(&verified.report_data) {
        Ok(address) => {
            log.pass(NearAttestationDrillStep::SignerBindingDefaultMode);
            Some(address)
        }
        Err(error) => {
            log.fail(
                NearAttestationDrillStep::SignerBindingDefaultMode,
                error.label(),
            );
            None
        }
    };
    if let Some(address) = attested_address.as_deref() {
        outcome.attested_signer_ref = Some(secret_ref(&address.to_ascii_lowercase()));
        outcome.report_json_signing_address_agrees =
            Some(report.signing_address.eq_ignore_ascii_case(address));
    }

    // 6. The pinned image measurements.
    let verdict = check_measurements_opt(expected, &verified);
    outcome.measurements = Some(measurement_evidence(&verdict));
    match &verdict {
        MeasurementVerdict::Pinned { .. } => log.pass(NearAttestationDrillStep::MeasurementsPinned),
        MeasurementVerdict::Mismatch { mismatches } => {
            let fields: Vec<&str> = mismatches.iter().map(|m| m.field.as_str()).collect();
            log.fail(
                NearAttestationDrillStep::MeasurementsPinned,
                format!("mismatch:{}", fields.join(",")),
            );
        }
        MeasurementVerdict::Refused { control } => {
            log.fail_missing_control(NearAttestationDrillStep::MeasurementsPinned, control);
        }
    }

    outcome.json_claim_anomalies =
        json_claim_anomalies(&report.unverified_json_measurements(), &verified)
            .into_iter()
            .map(|anomaly| NearAttestationJsonClaimAnomaly {
                field: anomaly.field.as_str().to_string(),
                claimed: anomaly.claimed,
                verified: anomaly.verified,
            })
            .collect();

    // Everything above is free. Everything below is not: refuse to spend
    // money proving something about an endpoint we have not established.
    if log
        .results
        .iter()
        .any(|result| result.status != NearAttestationStepStatus::Passed)
    {
        return finish(outcome, log);
    }
    let Some(attested_address) = attested_address else {
        return finish(outcome, log);
    };

    // 7. One minimal completion. These are the bytes the receipt will bind.
    let request_body = drill_completion_request_body(client.model());
    let completion = match client.complete(&request_body).await {
        Ok(completion) => {
            outcome.completion_charged = true;
            log.pass(NearAttestationDrillStep::CompletionPerformed);
            completion
        }
        Err(error) => {
            // A request that reached the provider may well have been billed
            // even though it did not return usable data. Only a transport
            // failure is confidently free, and this does not try to guess.
            log.fail_client(NearAttestationDrillStep::CompletionPerformed, &error);
            return finish(outcome, log);
        }
    };
    outcome.chat_id_ref = Some(secret_ref(&completion.chat_id));

    // 8. Its receipt, against the exact bytes sent.
    let receipt = match client.fetch_receipt(&completion.chat_id).await {
        Ok(receipt) => receipt,
        Err(error) => {
            log.fail_client(NearAttestationDrillStep::ReceiptVerified, &error);
            return finish(outcome, log);
        }
    };
    let verdict = match verify_receipt(&receipt, &request_body, &completion.response_text) {
        Ok(verdict) => {
            log.pass(NearAttestationDrillStep::ReceiptVerified);
            verdict
        }
        Err(error) => {
            log.fail(
                NearAttestationDrillStep::ReceiptVerified,
                receipt_error_label(&error),
            );
            return finish(outcome, log);
        }
    };
    outcome.receipt_signer_ref = Some(secret_ref(&verdict.signing_address.to_ascii_lowercase()));

    // 9. The join. See the module docs: without this the two halves prove
    //    nothing together.
    if verdict
        .signing_address
        .eq_ignore_ascii_case(&attested_address)
    {
        log.pass(NearAttestationDrillStep::ReceiptSignerIsAttestedKey);
    } else {
        log.fail(
            NearAttestationDrillStep::ReceiptSignerIsAttestedKey,
            RECEIPT_SIGNER_NOT_ATTESTED,
        );
    }

    finish(outcome, log)
}

fn finish(mut outcome: NearAttestationDrillOutcome, log: StepLog) -> NearAttestationDrillOutcome {
    outcome.steps = log.finish();
    outcome.passed = outcome
        .steps
        .iter()
        .all(|result| result.status == NearAttestationStepStatus::Passed);
    outcome
}

fn measurement_evidence(verdict: &MeasurementVerdict) -> NearAttestationMeasurementEvidence {
    let names = |fields: &[MeasurementField]| -> Vec<String> {
        fields.iter().map(|f| f.as_str().to_string()).collect()
    };
    match verdict {
        MeasurementVerdict::Pinned { fields } => NearAttestationMeasurementEvidence {
            verdict: "pinned".to_string(),
            checked_fields: names(fields),
            mismatched_fields: Vec::new(),
            missing_control: None,
        },
        MeasurementVerdict::Mismatch { mismatches } => NearAttestationMeasurementEvidence {
            verdict: "mismatch".to_string(),
            checked_fields: mismatches
                .iter()
                .map(|m| m.field.as_str().to_string())
                .collect(),
            mismatched_fields: names(&verdict.mismatched_fields()),
            missing_control: None,
        },
        MeasurementVerdict::Refused { control } => NearAttestationMeasurementEvidence {
            verdict: "refused".to_string(),
            checked_fields: Vec::new(),
            mismatched_fields: Vec::new(),
            missing_control: Some((*control).to_string()),
        },
    }
}

/// Why the verified quote's report data does not yield a signing address.
///
/// A layout problem and a key substitution are different failures with
/// different next actions, and they must never collapse into one label: an
/// operator told "the layout is not what we expect" should be looking at how
/// the report was fetched, and one told "the receipt signer is not the
/// attested key" should be taking the endpoint out of service. The two are
/// raised from different steps and carry different reasons for exactly that
/// reason.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SignerBindingError {
    /// `report_data` is shorter than the 64 bytes TDX defines.
    #[error("report data is {len} bytes, expected at least 64")]
    ReportDataTooShort { len: usize },
    /// Bytes 20..32 are not zero, so `[0..20]` is not a raw address.
    #[error("report data bytes 20..32 are not zero; the report was served in TLS-fingerprint mode")]
    TlsFingerprintMode,
}

impl SignerBindingError {
    /// A stable label for evidence. The condition, never the message.
    pub fn label(&self) -> &'static str {
        match self {
            Self::ReportDataTooShort { .. } => "report_data_too_short",
            Self::TlsFingerprintMode => "report_data_signer_padding_not_zero",
        }
    }
}

/// The signing address a TDX report binds, read out of verified report data.
///
/// NEAR AI serves `/v1/attestation/report` in two modes, and the difference
/// is not visible from the bytes' length:
///
/// - **Default** (what this drill fetches): `report_data[0..20]` is the raw
///   20-byte signing address, `[20..32]` are zero, `[32..64]` is the nonce.
/// - **`?include_tls_fingerprint=true`**: `report_data[0..32]` is
///   `SHA256(signing_address || spki_hash)` in full. NEAR AI's own verifier
///   README documents only this form.
///
/// So the zero padding is the discriminator, and asserting it is what keeps
/// the signer comparison honest. Without it, a future change that adds the
/// TLS flag would leave this function returning the first twenty bytes of a
/// hash, formatted as an address, which would never equal any real signer --
/// and the drill would go red at the *substitution* step, sending an
/// operator to hunt for a key attack that is not happening. Both mistakes
/// are avoided by refusing here, with [`SignerBindingError::TlsFingerprintMode`].
pub fn attested_signer_address(report_data: &[u8]) -> Result<String, SignerBindingError> {
    if report_data.len() < 64 {
        return Err(SignerBindingError::ReportDataTooShort {
            len: report_data.len(),
        });
    }
    if report_data[20..32].iter().any(|byte| *byte != 0) {
        return Err(SignerBindingError::TlsFingerprintMode);
    }
    Ok(format!("0x{}", hex::encode(&report_data[0..20])))
}

/// The reason [`NearAttestationDrillStep::ReceiptSignerIsAttestedKey`] fails.
///
/// Named as a constant so the test that keeps it distinct from every
/// [`SignerBindingError::label`] cannot drift out of sync with the drill.
pub const RECEIPT_SIGNER_NOT_ATTESTED: &str = "receipt_signer_is_not_the_attested_key";

/// The drill's TCB policy, as a function so it can be tested against the
/// statuses Intel actually emits.
///
/// `None` means the platform is accepted. Anything but [`REQUIRED_TCB_STATUS`]
/// is refused and the status is named, because "attestation failed" sends an
/// operator to the wrong place and "tcb_status:SWHardeningNeeded" sends them
/// to Intel's advisory.
///
/// There is deliberately no allow-list parameter. A configurable set of
/// acceptable statuses is the lever someone pulls at 2am to make a red drill
/// green, and the whole value of this drill is that it cannot be made green
/// except by fixing what it found.
///
/// One upstream behaviour worth knowing, measured against `dcap-qvl` 0.6.3:
/// a platform whose SVNs match *no* TCB level at all does not arrive here as
/// a downgraded status. `verify` returns `Err("No matching TCB level found")`
/// instead, so that case is refused one step earlier, at
/// [`NearAttestationDrillStep::QuoteVerified`]. This function is what catches
/// the statuses that *do* map to a level -- `SWHardeningNeeded`,
/// `ConfigurationNeeded`, `OutOfDate` and the rest.
pub fn tcb_step_reason(status: &str) -> Option<String> {
    (status != REQUIRED_TCB_STATUS).then(|| format!("tcb_status:{status}"))
}

/// A stable label for a receipt failure. The variant name, never the message.
fn receipt_error_label(error: &ReceiptError) -> &'static str {
    match error {
        ReceiptError::TextPartCount { .. } => "text_part_count",
        ReceiptError::RequestHashMalformed => "request_hash_malformed",
        ReceiptError::ResponseHashMalformed => "response_hash_malformed",
        ReceiptError::SignatureMalformed => "signature_malformed",
        ReceiptError::RecoveryIdUnsupported { .. } => "recovery_id_unsupported",
        ReceiptError::SignatureUnrecoverable => "signature_unrecoverable",
        ReceiptError::SigningAddressMalformed => "signing_address_malformed",
        ReceiptError::SignerMismatch => "signer_mismatch",
        ReceiptError::RequestHashMismatch => "request_hash_mismatch",
        ReceiptError::ResponseHashMismatch => "response_hash_mismatch",
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;
    use k256::ecdsa::SigningKey;
    use sha3::Keccak256;

    use super::*;
    use crate::near_attestation::AttestationReport;
    use crate::near_attestation::client::CompletionOutcome;
    use crate::near_attestation::quote::{Collateral, parse_collateral};
    use crate::near_attestation::receipt::ReceiptPayload;

    const REPORT: &str = include_str!("../../tests/fixtures/near_ai_attestation_report.json");
    const COLLATERAL: &str =
        include_str!("../../tests/fixtures/near_ai_attestation_collateral.json");
    const OUTDATED_REPORT: &str =
        include_str!("../../tests/fixtures/near_ai_attestation_report_outdated_tcb.json");
    const OUTDATED_COLLATERAL: &str =
        include_str!("../../tests/fixtures/dcap_qvl_outdated_tcb_collateral.json");

    /// 2026-09-01T12:00:00Z, the day the report and collateral fixtures were
    /// captured. `verify_quote` consults no clock but the one it is passed,
    /// so pinning it here means these tests fail on a code change and never
    /// on a calendar date.
    const FIXTURE_CAPTURED_AT: u64 = 1_788_264_000;

    /// The address the fixture quote attests: `report_data[0..20]`.
    const ATTESTED_ADDRESS: &str = "0xe5d0fec43b001f181a3410b96715ec54171f36da";

    /// Measurements as read out of the *verified* fixture quote -- which is
    /// exactly where the runbook tells an operator to get them, and never
    /// from the report's `info.tcb_info` JSON.
    const MRTD: &str = "b24d3b24e9e3c16012376b52362ca09856c4adecb709d5fac33addf1c47e193da075b125b6c364115771390a5461e217";
    const RTMR0: &str = "bc122d143ab768565ba5c3774ff5f03a63c89a4df7c1f5ea38d3bd173409d14f8cbdcc36d40e703cccb996a9d9687590";
    const RTMR1: &str = "c0445b704e4c48139496ae337423ddb1dcee3a673fd5fb60a53d562f127d235f11de471a7b4ee12c9027c829786757dc";
    const RTMR2: &str = "564622c7ddc55a53272cc9f0956d29b3f7e0dd18ede432720b71fd89e5b5d76cb0b99be7b7ff2a6a92b89b6b01643135";
    const RTMR3: &str = "8f993f8b7a99d5e4ea49a3413a0d6311efa6a61be3ec6cae1d13b353dd1835544084cba4b4e767c17f5c513da1857de8";

    /// A fixed key, never a generated one: a random key makes a failure
    /// unreproducible. It is emphatically *not* the attested key -- no one
    /// outside the enclave holds that -- which is why the drill's last step
    /// is the one thing these tests cannot drive to green offline.
    const IMPOSTOR_KEY_HEX: &str =
        "4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318";

    fn fixture_nonce() -> String {
        let value: serde_json::Value = serde_json::from_str(REPORT).unwrap();
        value["_fixture_nonce"].as_str().unwrap().to_string()
    }

    fn all_pins() -> ExpectedMeasurements {
        ExpectedMeasurements::from_env_value(Some(&format!(
            "mrtd={MRTD},rtmr0={RTMR0},rtmr1={RTMR1},rtmr2={RTMR2},rtmr3={RTMR3}"
        )))
        .expect("pins parse")
        .expect("pins are present")
    }

    // -- a stub endpoint -----------------------------------------------------

    /// What the stub should do when asked for a receipt.
    enum ReceiptBehaviour {
        /// Sign over the bytes actually sent and the response actually
        /// returned. This is what an honest endpoint does.
        SignWhatWasSent,
        /// Sign over different request bytes: the mistake a caller makes by
        /// re-serializing a parsed request instead of keeping what it sent.
        SignOverOtherRequestBytes(Vec<u8>),
        /// Fail the fetch.
        Fail(AttestationClientError),
    }

    struct StubEndpoint {
        model: String,
        report: Result<String, AttestationClientError>,
        collateral: String,
        response_text: String,
        receipt_key: SigningKey,
        receipt: ReceiptBehaviour,
        completion: Option<AttestationClientError>,
        sent_request: Mutex<Option<Vec<u8>>>,
        completion_calls: AtomicUsize,
    }

    impl StubEndpoint {
        /// An endpoint that serves the real captured report and collateral
        /// and answers a completion honestly.
        fn honest() -> Self {
            Self {
                model: "Qwen/Qwen3.6-35B-A3B-FP8".to_string(),
                report: Ok(REPORT.to_string()),
                collateral: COLLATERAL.to_string(),
                response_text: "pong".to_string(),
                receipt_key: SigningKey::from_slice(&hex::decode(IMPOSTOR_KEY_HEX).unwrap())
                    .unwrap(),
                receipt: ReceiptBehaviour::SignWhatWasSent,
                completion: None,
                sent_request: Mutex::new(None),
                completion_calls: AtomicUsize::new(0),
            }
        }

        fn completions_attempted(&self) -> usize {
            self.completion_calls.load(Ordering::SeqCst)
        }
    }

    /// EIP-191 `personal_sign`, re-derived here rather than reaching into
    /// `receipt`'s private helpers: a test that signs with the same code it
    /// verifies with proves less than one that does not.
    fn personal_sign(key: &SigningKey, message: &str) -> String {
        let mut hasher = Keccak256::new();
        hasher.update(b"\x19Ethereum Signed Message:\n");
        hasher.update(message.len().to_string().as_bytes());
        hasher.update(message.as_bytes());
        let digest: [u8; 32] = hasher.finalize().into();
        let (signature, recovery) = key.sign_prehash_recoverable(&digest).unwrap();
        let mut raw = signature.to_bytes().to_vec();
        raw.push(recovery.to_byte() + 27);
        format!("0x{}", hex::encode(raw))
    }

    fn eth_address(key: &SigningKey) -> String {
        let point = key.verifying_key().to_encoded_point(false);
        let digest = Keccak256::digest(&point.as_bytes()[1..]);
        format!("0x{}", hex::encode(&digest[12..]))
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        hex::encode(Sha256::digest(bytes))
    }

    #[async_trait]
    impl AttestationClient for StubEndpoint {
        fn model(&self) -> &str {
            &self.model
        }

        async fn fetch_report(
            &self,
            _nonce: &str,
        ) -> Result<AttestationReport, AttestationClientError> {
            let json = self.report.clone()?;
            Ok(AttestationReport::from_json(&json).expect("fixture report parses"))
        }

        async fn fetch_collateral(
            &self,
            _quote: &[u8],
        ) -> Result<Collateral, AttestationClientError> {
            Ok(parse_collateral(&self.collateral).expect("fixture collateral parses"))
        }

        async fn complete(
            &self,
            request_body: &[u8],
        ) -> Result<CompletionOutcome, AttestationClientError> {
            self.completion_calls.fetch_add(1, Ordering::SeqCst);
            if let Some(error) = self.completion.clone() {
                return Err(error);
            }
            *self.sent_request.lock().unwrap() = Some(request_body.to_vec());
            Ok(CompletionOutcome {
                chat_id: "chatcmpl-drill-fixture".to_string(),
                response_text: self.response_text.clone(),
            })
        }

        async fn fetch_receipt(
            &self,
            _chat_id: &str,
        ) -> Result<ReceiptPayload, AttestationClientError> {
            let sent = self
                .sent_request
                .lock()
                .unwrap()
                .clone()
                .expect("a receipt is only ever fetched after a completion");
            let signed_over = match &self.receipt {
                ReceiptBehaviour::Fail(error) => return Err(error.clone()),
                ReceiptBehaviour::SignWhatWasSent => sent,
                ReceiptBehaviour::SignOverOtherRequestBytes(other) => other.clone(),
            };
            let text = format!(
                "{}:{}",
                sha256_hex(&signed_over),
                sha256_hex(self.response_text.as_bytes())
            );
            Ok(ReceiptPayload {
                signature: personal_sign(&self.receipt_key, &text),
                signing_address: eth_address(&self.receipt_key),
                text,
            })
        }
    }

    fn step(
        outcome: &NearAttestationDrillOutcome,
        step: NearAttestationDrillStep,
    ) -> &NearAttestationStepResult {
        outcome
            .steps
            .iter()
            .find(|result| result.step == step)
            .expect("every step is reported")
    }

    async fn run(
        endpoint: &StubEndpoint,
        expected: Option<&ExpectedMeasurements>,
        nonce: &str,
    ) -> NearAttestationDrillOutcome {
        run_near_attestation_drill(endpoint, expected, nonce, FIXTURE_CAPTURED_AT).await
    }

    // -- the cases -----------------------------------------------------------

    #[tokio::test]
    async fn the_drill_refuses_when_no_measurements_are_pinned() {
        let endpoint = StubEndpoint::honest();
        let outcome = run(&endpoint, None, &fixture_nonce()).await;

        let measurements = step(&outcome, NearAttestationDrillStep::MeasurementsPinned);
        assert_eq!(measurements.status, NearAttestationStepStatus::Failed);
        assert_eq!(
            measurements.missing_control.as_deref(),
            Some(super::super::measurements::EXPECTED_MEASUREMENTS_CONTROL)
        );
        assert_eq!(
            measurements.reason.as_deref(),
            Some("missing_control:near_ai_expected_measurements")
        );
        assert!(!outcome.passed);
        // A refusal is not a skip-to-pass, and it must not cost money.
        assert_eq!(endpoint.completions_attempted(), 0);
        assert!(!outcome.completion_charged);
    }

    #[tokio::test]
    async fn the_drill_fails_when_the_report_does_not_carry_our_nonce() {
        // The replay case: the endpoint serves a genuine, correctly signed
        // report -- just not one bound to the nonce we asked for.
        let endpoint = StubEndpoint::honest();
        let ours = "f".repeat(64);
        assert_ne!(ours, fixture_nonce());
        let outcome = run(&endpoint, Some(&all_pins()), &ours).await;

        let nonce_step = step(&outcome, NearAttestationDrillStep::NonceBoundInQuote);
        assert_eq!(nonce_step.status, NearAttestationStepStatus::Failed);
        assert_eq!(
            nonce_step.reason.as_deref(),
            Some("nonce_not_in_report_data")
        );
        // Everything else about that report is genuine, so this must be the
        // only thing that failed -- a test that passed because the quote also
        // failed to verify would prove nothing about nonce binding.
        assert_eq!(
            step(&outcome, NearAttestationDrillStep::QuoteVerified).status,
            NearAttestationStepStatus::Passed
        );
        assert_eq!(
            step(&outcome, NearAttestationDrillStep::MeasurementsPinned).status,
            NearAttestationStepStatus::Passed
        );
        assert!(!outcome.passed);
        assert_eq!(endpoint.completions_attempted(), 0);
    }

    #[tokio::test]
    async fn the_drill_fails_when_the_receipt_signer_is_not_the_attested_key() {
        // The substitution case, and the reason this drill exists as one
        // thing rather than two. The report is genuine, the quote verifies,
        // the measurements match, the receipt is validly signed over exactly
        // the bytes we sent -- and it is signed by a key the hardware never
        // attested. Every step but the last passes.
        let endpoint = StubEndpoint::honest();
        assert_ne!(
            eth_address(&endpoint.receipt_key).to_ascii_lowercase(),
            ATTESTED_ADDRESS
        );
        let outcome = run(&endpoint, Some(&all_pins()), &fixture_nonce()).await;

        for earlier in [
            NearAttestationDrillStep::ReportFetched,
            NearAttestationDrillStep::QuoteVerified,
            NearAttestationDrillStep::TcbUpToDate,
            NearAttestationDrillStep::NonceBoundInQuote,
            NearAttestationDrillStep::SignerBindingDefaultMode,
            NearAttestationDrillStep::MeasurementsPinned,
            NearAttestationDrillStep::CompletionPerformed,
            NearAttestationDrillStep::ReceiptVerified,
        ] {
            assert_eq!(
                step(&outcome, earlier).status,
                NearAttestationStepStatus::Passed,
                "{} should have passed",
                earlier.as_str()
            );
        }
        let join = step(
            &outcome,
            NearAttestationDrillStep::ReceiptSignerIsAttestedKey,
        );
        assert_eq!(join.status, NearAttestationStepStatus::Failed);
        assert_eq!(
            join.reason.as_deref(),
            Some("receipt_signer_is_not_the_attested_key")
        );
        assert!(!outcome.passed);
        assert_eq!(
            outcome.blocking_steps(),
            vec!["receipt_signer_is_attested_key:receipt_signer_is_not_the_attested_key"]
        );
        // The two halves really did verify on their own: that is the point.
        assert!(outcome.attested_signer_ref.is_some());
        assert!(outcome.receipt_signer_ref.is_some());
        assert_ne!(outcome.attested_signer_ref, outcome.receipt_signer_ref);
    }

    #[tokio::test]
    async fn drill_evidence_carries_no_secret() {
        // Run the case that produces the most evidence -- a completion was
        // performed, a receipt was fetched, both signers are known.
        let endpoint = StubEndpoint::honest();
        let outcome = run(&endpoint, Some(&all_pins()), &fixture_nonce()).await;
        let text = serde_json::to_string(&outcome).unwrap();

        for forbidden in ["sk-", "Bearer", "0x"] {
            assert!(!text.contains(forbidden), "evidence leaked {forbidden}");
        }
        // Named, not merely absent: an address that appeared without its
        // `0x` prefix would slip past the loop above.
        assert!(!text.contains(&ATTESTED_ADDRESS[2..]));
        assert!(!text.contains(&eth_address(&endpoint.receipt_key)[2..]));
        assert!(!text.contains("chatcmpl-drill-fixture"));
        assert!(!text.contains(&endpoint.response_text));
        // What it does carry: the nonce we chose, the tcb status, and the
        // public measurement registers.
        assert!(text.contains(&fixture_nonce()));
        assert!(text.contains("UpToDate"));
        assert!(text.contains(MRTD));
    }

    #[tokio::test]
    async fn a_missing_api_key_is_a_named_missing_control_not_a_skip() {
        let mut endpoint = StubEndpoint::honest();
        endpoint.report = Err(AttestationClientError::MissingControl {
            step: AttestationStep::Report,
            control: super::super::client::API_KEY_CONTROL,
        });
        let outcome = run(&endpoint, Some(&all_pins()), &fixture_nonce()).await;

        let report_step = step(&outcome, NearAttestationDrillStep::ReportFetched);
        assert_eq!(report_step.status, NearAttestationStepStatus::Failed);
        assert_eq!(
            report_step.missing_control.as_deref(),
            Some("near_ai_api_key")
        );
        assert!(!outcome.passed);
        assert_eq!(endpoint.completions_attempted(), 0);
    }

    #[tokio::test]
    async fn a_step_that_never_ran_is_not_reported_as_passed() {
        // The failure mode this guards: a drill that stops early and renders
        // only the steps it reached, so the summary reads as green.
        let endpoint = StubEndpoint::honest();
        let outcome = run(&endpoint, None, &fixture_nonce()).await;

        assert_eq!(outcome.steps.len(), NearAttestationDrillStep::ALL.len());
        for later in [
            NearAttestationDrillStep::CompletionPerformed,
            NearAttestationDrillStep::ReceiptVerified,
            NearAttestationDrillStep::ReceiptSignerIsAttestedKey,
        ] {
            assert_eq!(
                step(&outcome, later).status,
                NearAttestationStepStatus::NotRun,
                "{} must not read as passed",
                later.as_str()
            );
        }
        assert!(!outcome.passed);
    }

    #[tokio::test]
    async fn a_measurement_mismatch_names_the_register() {
        let endpoint = StubEndpoint::honest();
        let wrong = ExpectedMeasurements::from_env_value(Some(&format!(
            "mrtd={MRTD},rtmr2={}",
            "a".repeat(96)
        )))
        .unwrap()
        .unwrap();
        let outcome = run(&endpoint, Some(&wrong), &fixture_nonce()).await;

        let measurements = step(&outcome, NearAttestationDrillStep::MeasurementsPinned);
        assert_eq!(measurements.status, NearAttestationStepStatus::Failed);
        assert_eq!(measurements.reason.as_deref(), Some("mismatch:rtmr2"));
        assert_eq!(
            outcome.measurements.as_ref().unwrap().mismatched_fields,
            vec!["rtmr2".to_string()]
        );
        assert_eq!(endpoint.completions_attempted(), 0);
    }

    #[tokio::test]
    async fn the_receipt_must_bind_the_exact_bytes_that_were_sent() {
        // A caller that re-serializes its request gets RequestHashMismatch,
        // which reads as tampering. This pins that the drill hands the
        // receipt check the bytes it actually put on the wire.
        let mut endpoint = StubEndpoint::honest();
        endpoint.receipt = ReceiptBehaviour::SignOverOtherRequestBytes(
            br#"{"messages":[{"content":"ping","role":"user"}],"model":"Qwen/Qwen3.6-35B-A3B-FP8"}"#
                .to_vec(),
        );
        let outcome = run(&endpoint, Some(&all_pins()), &fixture_nonce()).await;

        let receipt_step = step(&outcome, NearAttestationDrillStep::ReceiptVerified);
        assert_eq!(receipt_step.status, NearAttestationStepStatus::Failed);
        assert_eq!(
            receipt_step.reason.as_deref(),
            Some("request_hash_mismatch")
        );
        assert_eq!(
            step(
                &outcome,
                NearAttestationDrillStep::ReceiptSignerIsAttestedKey
            )
            .status,
            NearAttestationStepStatus::NotRun
        );
    }

    #[tokio::test]
    async fn a_receipt_fetch_failure_names_its_condition() {
        let mut endpoint = StubEndpoint::honest();
        endpoint.receipt = ReceiptBehaviour::Fail(AttestationClientError::HttpStatus {
            step: AttestationStep::Receipt,
            status: 401,
        });
        let outcome = run(&endpoint, Some(&all_pins()), &fixture_nonce()).await;

        let receipt_step = step(&outcome, NearAttestationDrillStep::ReceiptVerified);
        assert_eq!(receipt_step.status, NearAttestationStepStatus::Failed);
        assert_eq!(receipt_step.reason.as_deref(), Some("http_status"));
        // The completion was already paid for by then.
        assert!(outcome.completion_charged);
    }

    #[tokio::test]
    async fn a_platform_with_no_matching_tcb_level_is_refused_at_the_quote() {
        // A real quote from a platform whose SVNs match no TCB level in
        // Intel's tcb_info. `dcap-qvl` 0.6.3 does not downgrade the status
        // for this -- it refuses the quote outright -- so the refusal lands
        // on `quote_verified` rather than on `tcb_up_to_date`. Pinned here
        // because the difference matters to whoever reads the evidence.
        let mut endpoint = StubEndpoint::honest();
        endpoint.report = Ok(OUTDATED_REPORT.to_string());
        endpoint.collateral = OUTDATED_COLLATERAL.to_string();
        let value: serde_json::Value = serde_json::from_str(OUTDATED_REPORT).unwrap();
        let nonce = value["_fixture_nonce"].as_str().unwrap();

        let outcome =
            run_near_attestation_drill(&endpoint, Some(&all_pins()), nonce, 1_772_000_000).await;

        let quote_step = step(&outcome, NearAttestationDrillStep::QuoteVerified);
        assert_eq!(quote_step.status, NearAttestationStepStatus::Failed);
        assert_eq!(quote_step.reason.as_deref(), Some("verification_failed"));
        assert!(outcome.tcb_status.is_none());
        assert_eq!(endpoint.completions_attempted(), 0);
    }

    #[test]
    fn only_up_to_date_satisfies_the_tcb_policy() {
        // Every status Intel's TCB info can carry, from `dcap-qvl`'s own
        // enumeration. Only one of them passes, and the rest are named.
        assert_eq!(tcb_step_reason("UpToDate"), None);
        for status in [
            "SWHardeningNeeded",
            "ConfigurationNeeded",
            "ConfigurationAndSWHardeningNeeded",
            "OutOfDate",
            "OutOfDateConfigurationNeeded",
            "Revoked",
        ] {
            assert_eq!(
                tcb_step_reason(status),
                Some(format!("tcb_status:{status}")),
                "{status} must be refused and named"
            );
        }
    }

    #[test]
    fn the_default_report_mode_yields_the_raw_signing_address() {
        // Twenty address bytes, twelve zero bytes, then the nonce. Measured
        // against the live service, not read off documentation -- NEAR AI's
        // own README describes only the other mode.
        let mut report_data = vec![0u8; 64];
        report_data[..20].copy_from_slice(&hex::decode(&ATTESTED_ADDRESS[2..]).unwrap());
        report_data[32..].copy_from_slice(&hex::decode(fixture_nonce()).unwrap());
        assert_eq!(
            attested_signer_address(&report_data).unwrap(),
            ATTESTED_ADDRESS
        );
    }

    #[test]
    fn tls_fingerprint_mode_is_refused_rather_than_silently_truncated() {
        // The whole point of the zero-padding assertion. In this mode all 32
        // bytes are SHA256(signing_address || spki_hash); taking the first 20
        // of them and calling it an address is the failure this prevents.
        let mut report_data = vec![0u8; 64];
        report_data[..32].copy_from_slice(&Sha256::digest(b"address || spki"));
        let error = attested_signer_address(&report_data)
            .expect_err("a hashed report_data must not be read as an address");
        assert_eq!(error, SignerBindingError::TlsFingerprintMode);
        assert_eq!(error.label(), "report_data_signer_padding_not_zero");
    }

    #[test]
    fn a_single_non_zero_padding_byte_is_enough_to_refuse() {
        // Not "the padding looks hashed" but "the padding is not zero".
        // A check that only rejected an obviously-hashed prefix would let a
        // near-zero one through.
        for index in 20..32 {
            let mut report_data = vec![0u8; 64];
            report_data[index] = 1;
            assert_eq!(
                attested_signer_address(&report_data),
                Err(SignerBindingError::TlsFingerprintMode),
                "byte {index} must be checked"
            );
        }
        // ... and bytes outside 20..32 must not trip it.
        for index in [0usize, 19, 32, 63] {
            let mut report_data = vec![0u8; 64];
            report_data[index] = 1;
            assert!(
                attested_signer_address(&report_data).is_ok(),
                "byte {index} is not padding"
            );
        }
    }

    #[test]
    fn short_report_data_is_refused_distinctly_from_the_wrong_mode() {
        let error =
            attested_signer_address(&[0u8; 48]).expect_err("48 bytes is not a TDX report_data");
        assert_eq!(error, SignerBindingError::ReportDataTooShort { len: 48 });
        assert_eq!(error.label(), "report_data_too_short");
    }

    #[test]
    fn a_layout_failure_is_never_reported_as_a_key_substitution() {
        // An operator who sees "the layout is not what we expect" should be
        // looking at how the report was fetched. One who sees the
        // substitution reason should be taking the endpoint out of service.
        // Collapsing the two would send them to the wrong place.
        for error in [
            SignerBindingError::TlsFingerprintMode,
            SignerBindingError::ReportDataTooShort { len: 0 },
        ] {
            assert_ne!(error.label(), RECEIPT_SIGNER_NOT_ATTESTED);
        }
        assert_ne!(
            SignerBindingError::TlsFingerprintMode.label(),
            SignerBindingError::ReportDataTooShort { len: 0 }.label()
        );
    }

    #[test]
    fn a_real_quote_with_non_zero_padding_is_refused_by_the_seam() {
        // Not a synthetic buffer: real report_data off a real signed quote,
        // in the shape a `?include_tls_fingerprint=true` fetch would return.
        //
        // This stops at the seam rather than running the whole drill because
        // no obtainable fixture can reach the binding step -- a quote that
        // verifies has zero padding, and this one fails at `quote_verified`
        // first. The drill's own use of this function is covered from the
        // other direction: `the_drill_fails_when_the_receipt_signer_is_not_
        // the_attested_key` asserts `SignerBindingDefaultMode` *passed* for
        // the good fixture, so removing the check from the seam fails the
        // test above and removing the call from the drill fails that one.
        let quote = AttestationReport::from_json(OUTDATED_REPORT)
            .unwrap()
            .quote_bytes()
            .unwrap();
        let report_data = &quote[568..632];
        // Measured, not assumed.
        assert!(report_data[20..32].iter().any(|byte| *byte != 0));
        assert_eq!(
            attested_signer_address(report_data),
            Err(SignerBindingError::TlsFingerprintMode)
        );
    }

    #[test]
    fn the_completion_request_is_small_and_deterministic() {
        // Step 5 costs money on every run; this is what bounds the bill.
        let body = drill_completion_request_body("some/model");
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed["max_tokens"], serde_json::json!(1));
        assert_eq!(parsed["stream"], serde_json::json!(false));
        assert_eq!(parsed["messages"][0]["content"], serde_json::json!("ping"));
        assert_eq!(body, drill_completion_request_body("some/model"));
        assert!(body.len() < 200, "the drill request must stay minimal");
    }

    #[test]
    fn a_generated_nonce_is_thirty_two_bytes_of_hex_and_not_reused() {
        let first = generate_drill_nonce();
        assert_eq!(first.len(), 64);
        assert!(first.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_ne!(first, generate_drill_nonce());
    }
}
