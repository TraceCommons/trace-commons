//! Wire contract for the Insights judgement service.
//!
//! The service runs three pinned judgements (`task`, `event_kind`,
//! `finished`) over one session's judgement digest and returns labels signed
//! by its attested key. This module holds the shared vocabulary only: the
//! request and response shapes, the closed refusal set, the capacity rule,
//! the digest cap and the signing-bytes encoder. It grants no trust. The
//! service verifies its own claims and receipts, and the client verifies the
//! service's quote and signature against its own pins.
//!
//! Design: `docs/superpowers/specs/2026-10-20-insights-judgement-service-design.md`,
//! section 3.
//!
//! Every type parses closed: unknown fields, unknown labels and inconsistent
//! combinations are refused rather than carried. The digest is scrubbed
//! contributor text, so no `Debug` or error here ever contains it.

use std::collections::BTreeSet;

use chrono::{DateTime, Datelike, NaiveDate, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};

/// The only wire version this crate speaks. It is also carried by the
/// signing domain, so a later version signs different bytes.
pub const JUDGEMENT_WIRE_VERSION: u32 = 1;
/// The judgement digest's byte cap. A larger digest is dropped whole by the
/// daemon and refused by the service; it is never truncated.
pub const JUDGEMENT_DIGEST_MAX_BYTES: usize = 8192;
/// The total request body cap, checked before the body is parsed.
pub const JUDGEMENT_REQUEST_MAX_BYTES: usize = 16 * 1024;
/// Request header carrying the nonce the daemon verified the quote with.
pub const JUDGEMENT_NONCE_HEADER: &str = "x-trace-judgement-nonce";
/// `Retry-After` on `judgement_saturated`, as `witness_pacing`.
pub const JUDGEMENT_SATURATED_RETRY_AFTER_SECS: u32 = 30;
/// `event_kind` is judged for at most this many marked events per digest, in
/// digest order. Later marks come back `unjudged`.
pub const JUDGEMENT_EVENT_KIND_MAX: u32 = 4;
/// Domain prefix of the signing bytes.
pub const JUDGEMENT_SIGNING_DOMAIN: &[u8] = b"trace_commons.insights_judgement.v1\n";

const NONCE_LEN: usize = 32;
const SHA256_HEX_LEN: usize = 64;
const ED25519_HEX_LEN: usize = 64;
/// `r || s || v`, the EIP-191 form the witness signs with.
const SIGNATURE_BYTES: usize = 65;
const PROMPT_HASH_HEX_LEN: usize = 12;

// ---------------------------------------------------------------------------
// Refusals
// ---------------------------------------------------------------------------

/// The refusals the judgement service emits, as a closed set.
///
/// Mirrors [`crate::admission::AdmissionRefusal`]: a refusal reaches the
/// client as a status and a `{"error": "<label>"}` body. The client matches
/// the label against this set and never echoes the body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JudgementRefusal {
    /// The service is at capacity. Retry after `Retry-After`.
    Saturated,
    /// This account's monthly spend cap is reached. Body carries `reset_at`.
    AccountBudgetExhausted,
    /// The operator-wide monthly budget is reached. Body carries `reset_at`.
    OperatorBudgetExhausted,
    /// The request does not parse, or is inconsistent. Never retry this digest.
    RequestMalformed,
    /// The digest or the body is over its cap. Never retry this digest.
    DigestTooLarge,
    /// The service does not speak this wire version. Retry after an update.
    WireVersionUnsupported,
    /// The judgement claim was refused. Retry once with a fresh claim.
    CredentialRefused,
    /// This account cannot use the service (pooled tenants in v1).
    AccountIneligible,
    /// The model's receipt or catalog check failed. Not today.
    ModelUnattested,
    /// The model call timed out. Not today.
    Timeout,
}

impl JudgementRefusal {
    /// Every refusal, for tests and exhaustive mappings.
    pub const ALL: [JudgementRefusal; 10] = [
        JudgementRefusal::Saturated,
        JudgementRefusal::AccountBudgetExhausted,
        JudgementRefusal::OperatorBudgetExhausted,
        JudgementRefusal::RequestMalformed,
        JudgementRefusal::DigestTooLarge,
        JudgementRefusal::WireVersionUnsupported,
        JudgementRefusal::CredentialRefused,
        JudgementRefusal::AccountIneligible,
        JudgementRefusal::ModelUnattested,
        JudgementRefusal::Timeout,
    ];

    /// The wire label, which is what the service puts in the `error` field.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Saturated => "judgement_saturated",
            Self::AccountBudgetExhausted => "judgement_account_budget_exhausted",
            Self::OperatorBudgetExhausted => "judgement_operator_budget_exhausted",
            Self::RequestMalformed => "judgement_request_malformed",
            Self::DigestTooLarge => "judgement_digest_too_large",
            Self::WireVersionUnsupported => "judgement_wire_version_unsupported",
            Self::CredentialRefused => "judgement_credential_refused",
            Self::AccountIneligible => "judgement_account_ineligible",
            Self::ModelUnattested => "judgement_model_unattested",
            Self::Timeout => "judgement_timeout",
        }
    }

    /// The status this refusal is sent with.
    pub const fn status(self) -> u16 {
        match self {
            Self::Saturated | Self::AccountBudgetExhausted | Self::OperatorBudgetExhausted => 503,
            Self::RequestMalformed | Self::WireVersionUnsupported => 400,
            Self::DigestTooLarge => 413,
            Self::CredentialRefused => 401,
            Self::AccountIneligible => 403,
            Self::ModelUnattested => 502,
            Self::Timeout => 504,
        }
    }

    /// Whether this is a capacity refusal. Only these are retryable on a
    /// timer; a model or timeout failure is not capacity, because a daemon
    /// that retried it would spend the account's cap on a broken path.
    pub const fn is_capacity(self) -> bool {
        matches!(
            self,
            Self::Saturated | Self::AccountBudgetExhausted | Self::OperatorBudgetExhausted
        )
    }

    /// Whether the refusal body carries `reset_at`: exactly the two budget
    /// refusals, which must not be retried before it.
    pub const fn carries_reset_at(self) -> bool {
        matches!(
            self,
            Self::AccountBudgetExhausted | Self::OperatorBudgetExhausted
        )
    }

    /// Recognise a refusal from a label alone, for a label already recorded.
    /// Prefer [`JudgementRefusal::from_response`] where a response is read.
    pub fn from_label(label: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.label() == label)
    }

    /// Recognise a refusal from a response, requiring the status and the
    /// label to agree. A label on a status it is never sent with is not this
    /// refusal, so a body cannot pick how a status is handled.
    pub fn from_response(status: u16, label: &str) -> Option<Self> {
        Self::from_label(label).filter(|r| r.status() == status)
    }
}

/// True only for a `503` carrying one of the three capacity labels.
pub fn is_judgement_capacity(status: u16, error: &str) -> bool {
    JudgementRefusal::from_response(status, error).is_some_and(JudgementRefusal::is_capacity)
}

/// A refusal body: the label, plus `reset_at` (unix seconds) for exactly
/// the two budget refusals.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JudgementRefusalBody {
    refusal: JudgementRefusal,
    reset_at: Option<i64>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRefusalBody {
    error: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reset_at: Option<i64>,
}

impl JudgementRefusalBody {
    /// `reset_at` must be present exactly when the refusal carries one, and
    /// must be the first instant of a UTC month, as
    /// [`judgement_budget_reset_at`] computes it.
    ///
    /// Refusal bodies are not signed, and the client does not retry before
    /// `reset_at`. Without this rule a forged or broken value such as
    /// `i64::MAX` would block an account's judgements for good, and `-1`
    /// would have the client retry straight away against an exhausted cap.
    pub fn new(
        refusal: JudgementRefusal,
        reset_at: Option<i64>,
    ) -> Result<Self, JudgementWireError> {
        if refusal.carries_reset_at() != reset_at.is_some() {
            return Err(JudgementWireError::Malformed("reset_at"));
        }
        if let Some(reset_at) = reset_at {
            let month_start = reset_at
                .checked_sub(1)
                .and_then(judgement_budget_reset_at)
                .is_some_and(|expected| expected == reset_at);
            if !month_start {
                return Err(JudgementWireError::Malformed("reset_at"));
            }
        }
        Ok(Self { refusal, reset_at })
    }

    pub fn refusal(&self) -> JudgementRefusal {
        self.refusal
    }

    pub fn reset_at(&self) -> Option<i64> {
        self.reset_at
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(&RawRefusalBody {
            error: self.refusal.label().to_string(),
            reset_at: self.reset_at,
        })
        .expect("a refusal body is JSON serializable")
    }
}

/// Parse a refusal body against the status it arrived with. `None` for
/// anything outside the closed set: an unknown label, a label on the wrong
/// status, an unknown field, or `reset_at` where the rule says otherwise.
pub fn parse_judgement_refusal(status: u16, body: &[u8]) -> Option<JudgementRefusalBody> {
    let raw: RawRefusalBody = serde_json::from_slice(body).ok()?;
    let refusal = JudgementRefusal::from_response(status, &raw.error)?;
    JudgementRefusalBody::new(refusal, raw.reset_at).ok()
}

/// The first instant of the UTC month after `now_unix`, in unix seconds: the
/// `reset_at` of a budget refusal. `None` only outside chrono's range.
pub fn judgement_budget_reset_at(now_unix: i64) -> Option<i64> {
    let now = DateTime::<Utc>::from_timestamp(now_unix, 0)?;
    let (year, month) = if now.month() == 12 {
        (now.year().checked_add(1)?, 1)
    } else {
        (now.year(), now.month() + 1)
    };
    let start = NaiveDate::from_ymd_opt(year, month, 1)?.and_hms_opt(0, 0, 0)?;
    Some(start.and_utc().timestamp())
}

/// A malformed wire value. Carries a static field name, never input bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum JudgementWireError {
    #[error("malformed judgement wire value: {0}")]
    Malformed(&'static str),
    #[error("unsupported judgement wire version")]
    WireVersionUnsupported,
}

// ---------------------------------------------------------------------------
// Nonce
// ---------------------------------------------------------------------------

/// The 32-byte nonce the daemon verified the service's quote with, sent as
/// 64 lowercase hex in [`JUDGEMENT_NONCE_HEADER`] and signed into the response.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct JudgementNonce([u8; NONCE_LEN]);

impl std::fmt::Debug for JudgementNonce {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("JudgementNonce")
            .field(&self.to_hex())
            .finish()
    }
}

impl JudgementNonce {
    pub fn from_bytes(bytes: [u8; NONCE_LEN]) -> Self {
        Self(bytes)
    }

    /// Exactly 64 lowercase hex characters.
    pub fn from_hex(value: &str) -> Result<Self, JudgementWireError> {
        if !is_lower_hex(value, NONCE_LEN * 2) {
            return Err(JudgementWireError::Malformed("nonce"));
        }
        let mut bytes = [0u8; NONCE_LEN];
        hex::decode_to_slice(value, &mut bytes)
            .map_err(|_| JudgementWireError::Malformed("nonce"))?;
        Ok(Self(bytes))
    }

    pub fn as_bytes(&self) -> &[u8; NONCE_LEN] {
        &self.0
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }
}

fn is_lower_hex(value: &str, width: usize) -> bool {
    value.len() == width
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

// ---------------------------------------------------------------------------
// Judgements and labels
// ---------------------------------------------------------------------------

/// The three pinned judgements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JudgementKind {
    Task,
    EventKind,
    Finished,
}

impl JudgementKind {
    pub const fn as_wire(self) -> &'static str {
        match self {
            Self::Task => "task",
            Self::EventKind => "event_kind",
            Self::Finished => "finished",
        }
    }

    /// The labels a judged result of this kind may carry, in wire order.
    /// [`JudgementLabel::Unjudged`] is never among them.
    pub const fn choices(self) -> &'static [JudgementLabel] {
        match self {
            Self::Task => &[
                JudgementLabel::BugFix,
                JudgementLabel::Refactor,
                JudgementLabel::Tests,
                JudgementLabel::Docs,
                JudgementLabel::Other,
            ],
            Self::EventKind => &[
                JudgementLabel::Rejection,
                JudgementLabel::Redirect,
                JudgementLabel::Iteration,
                JudgementLabel::Unclear,
            ],
            Self::Finished => &[
                JudgementLabel::Yes,
                JudgementLabel::No,
                JudgementLabel::Unclear,
            ],
        }
    }

    const fn tag(self) -> u8 {
        match self {
            Self::Task => 1,
            Self::EventKind => 2,
            Self::Finished => 3,
        }
    }
}

/// Every label the wire can carry. Which ones a result may use depends on
/// its [`JudgementKind`]; the result constructors enforce that.
///
/// `Unjudged` marks an `event_kind` result past [`JUDGEMENT_EVENT_KIND_MAX`]:
/// no model call was made for it, so it has no confidence, judge version or
/// receipt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JudgementLabel {
    BugFix,
    Refactor,
    Tests,
    Docs,
    Other,
    Rejection,
    Redirect,
    Iteration,
    Unclear,
    Yes,
    No,
    Unjudged,
}

impl JudgementLabel {
    pub const fn as_wire(self) -> &'static str {
        match self {
            Self::BugFix => "bug_fix",
            Self::Refactor => "refactor",
            Self::Tests => "tests",
            Self::Docs => "docs",
            Self::Other => "other",
            Self::Rejection => "rejection",
            Self::Redirect => "redirect",
            Self::Iteration => "iteration",
            Self::Unclear => "unclear",
            Self::Yes => "yes",
            Self::No => "no",
            Self::Unjudged => "unjudged",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfidenceBucket {
    High,
    Medium,
    Low,
}

impl ConfidenceBucket {
    const fn tag(self) -> u8 {
        match self {
            Self::High => 1,
            Self::Medium => 2,
            Self::Low => 3,
        }
    }
}

/// Where a confidence bucket came from: the model's choice probabilities,
/// or a chat model's own report. The daemon shows it in the answer's (i).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfidenceBasis {
    Probability,
    SelfReported,
}

impl ConfidenceBasis {
    const fn tag(self) -> u8 {
        match self {
            Self::Probability => 1,
            Self::SelfReported => 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JudgementConfidence {
    pub bucket: ConfidenceBucket,
    pub basis: ConfidenceBasis,
}

/// `(prompt_version, model_id)`: what fixes a label's meaning. Either part
/// changing is a new judge version.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct JudgeVersion {
    prompt_version: String,
    model_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawJudgeVersion {
    prompt_version: String,
    model_id: String,
}

impl JudgeVersion {
    /// `prompt_version` is `<name>-v<N>+sha256:<12 lowercase hex>`, where
    /// `<name>` is lowercase ASCII letters and hyphens and `N` is a positive
    /// integer without leading zeros. `model_id` must not be empty.
    pub fn new(prompt_version: String, model_id: String) -> Result<Self, JudgementWireError> {
        if !is_prompt_version(&prompt_version) {
            return Err(JudgementWireError::Malformed("prompt_version"));
        }
        if model_id.is_empty() {
            return Err(JudgementWireError::Malformed("model_id"));
        }
        Ok(Self {
            prompt_version,
            model_id,
        })
    }

    pub fn prompt_version(&self) -> &str {
        &self.prompt_version
    }

    pub fn model_id(&self) -> &str {
        &self.model_id
    }
}

impl<'de> Deserialize<'de> for JudgeVersion {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = RawJudgeVersion::deserialize(deserializer)?;
        Self::new(raw.prompt_version, raw.model_id).map_err(serde::de::Error::custom)
    }
}

fn is_prompt_version(value: &str) -> bool {
    let Some((head, hash)) = value.split_once("+sha256:") else {
        return false;
    };
    if !is_lower_hex(hash, PROMPT_HASH_HEX_LEN) {
        return false;
    }
    let Some((name, number)) = head.rsplit_once("-v") else {
        return false;
    };
    let name_ok = name.starts_with(|c: char| c.is_ascii_lowercase())
        && !name.ends_with('-')
        && name.bytes().all(|b| b.is_ascii_lowercase() || b == b'-');
    let number_ok = !number.is_empty()
        && !number.starts_with('0')
        && number.bytes().all(|b| b.is_ascii_digit());
    name_ok && number_ok
}

/// The only receipt kind the service returns. A `gateway` receipt binds no
/// model and is refused by the service, so it has no variant here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum JudgementReceiptKind {
    #[serde(rename = "provider_tee")]
    ProviderTee,
}

impl JudgementReceiptKind {
    pub const fn as_wire(self) -> &'static str {
        match self {
            Self::ProviderTee => "provider_tee",
        }
    }

    const fn tag(self) -> u8 {
        match self {
            Self::ProviderTee => 1,
        }
    }
}

/// The NEAR AI receipt the service verified for one result's model call.
///
/// Omits the request and response hashes and the provider `chat_id`, as the
/// witness V2 certificate does: the daemon cannot use them, and they
/// correlate with provider-side records. `verified` is always `true` on the
/// wire; a receipt that did not verify is never sent.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct JudgementReceipt {
    signature_kind: JudgementReceiptKind,
    model: String,
    signer: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawReceipt {
    signature_kind: JudgementReceiptKind,
    model: String,
    signer: String,
    verified: bool,
}

impl JudgementReceipt {
    /// A `provider_tee` receipt for `model`, signed by the Ed25519 key
    /// `signer` (64 lowercase hex, no prefix).
    pub fn new(model: String, signer: String) -> Result<Self, JudgementWireError> {
        if model.is_empty() {
            return Err(JudgementWireError::Malformed("receipt.model"));
        }
        if !is_lower_hex(&signer, ED25519_HEX_LEN) {
            return Err(JudgementWireError::Malformed("receipt.signer"));
        }
        Ok(Self {
            signature_kind: JudgementReceiptKind::ProviderTee,
            model,
            signer,
        })
    }

    pub fn signature_kind(&self) -> JudgementReceiptKind {
        self.signature_kind
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn signer(&self) -> &str {
        &self.signer
    }
}

impl Serialize for JudgementReceipt {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        RawReceipt {
            signature_kind: self.signature_kind,
            model: self.model.clone(),
            signer: self.signer.clone(),
            verified: true,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for JudgementReceipt {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = RawReceipt::deserialize(deserializer)?;
        if !raw.verified {
            return Err(serde::de::Error::custom(
                "an unverified receipt is never sent",
            ));
        }
        Self::new(raw.model, raw.signer).map_err(serde::de::Error::custom)
    }
}

/// The judged part of a result: absent exactly for an unjudged event mark.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Judged {
    confidence: JudgementConfidence,
    judge_version: JudgeVersion,
    /// Absent only from a development build whose model returns no
    /// per-request receipt. A production service refuses such a call, and
    /// the client decides whether it accepts a result without one.
    receipt: Option<JudgementReceipt>,
}

/// One result. Fields are private so every value went through a
/// constructor that checked it against its judgement.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct JudgementResult {
    judgement: JudgementKind,
    event_index: Option<u32>,
    label: JudgementLabel,
    judged: Option<Judged>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawResult {
    judgement: JudgementKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    event_index: Option<u32>,
    label: JudgementLabel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    confidence: Option<JudgementConfidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    judge_version: Option<JudgeVersion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    receipt: Option<JudgementReceipt>,
}

impl JudgementResult {
    /// A judged result. The label must be one of `judgement`'s choices;
    /// `event_index` is present exactly for `event_kind` and below
    /// [`JUDGEMENT_EVENT_KIND_MAX`]; a receipt must name the judge version's
    /// model.
    pub fn judged(
        judgement: JudgementKind,
        event_index: Option<u32>,
        label: JudgementLabel,
        confidence: JudgementConfidence,
        judge_version: JudgeVersion,
        receipt: Option<JudgementReceipt>,
    ) -> Result<Self, JudgementWireError> {
        if !judgement.choices().contains(&label) {
            return Err(JudgementWireError::Malformed("label"));
        }
        match (judgement, event_index) {
            (JudgementKind::EventKind, Some(index)) if index < JUDGEMENT_EVENT_KIND_MAX => {}
            (JudgementKind::Task | JudgementKind::Finished, None) => {}
            _ => return Err(JudgementWireError::Malformed("event_index")),
        }
        if receipt
            .as_ref()
            .is_some_and(|r| r.model() != judge_version.model_id())
        {
            return Err(JudgementWireError::Malformed("receipt.model"));
        }
        Ok(Self {
            judgement,
            event_index,
            label,
            judged: Some(Judged {
                confidence,
                judge_version,
                receipt,
            }),
        })
    }

    /// An `event_kind` mark past the judged limit. Its index must be at or
    /// above [`JUDGEMENT_EVENT_KIND_MAX`]: the first marks are always judged.
    pub fn unjudged_event(event_index: u32) -> Result<Self, JudgementWireError> {
        if event_index < JUDGEMENT_EVENT_KIND_MAX {
            return Err(JudgementWireError::Malformed("event_index"));
        }
        Ok(Self {
            judgement: JudgementKind::EventKind,
            event_index: Some(event_index),
            label: JudgementLabel::Unjudged,
            judged: None,
        })
    }

    pub fn judgement(&self) -> JudgementKind {
        self.judgement
    }

    pub fn event_index(&self) -> Option<u32> {
        self.event_index
    }

    pub fn label(&self) -> JudgementLabel {
        self.label
    }

    pub fn confidence(&self) -> Option<JudgementConfidence> {
        self.judged.as_ref().map(|j| j.confidence)
    }

    pub fn judge_version(&self) -> Option<&JudgeVersion> {
        self.judged.as_ref().map(|j| &j.judge_version)
    }

    pub fn receipt(&self) -> Option<&JudgementReceipt> {
        self.judged.as_ref().and_then(|j| j.receipt.as_ref())
    }

    /// Whether this result carries a verified `provider_tee` receipt.
    pub fn is_attested(&self) -> bool {
        self.receipt().is_some()
    }

    fn to_raw(&self) -> RawResult {
        RawResult {
            judgement: self.judgement,
            event_index: self.event_index,
            label: self.label,
            confidence: self.confidence(),
            judge_version: self.judge_version().cloned(),
            receipt: self.receipt().cloned(),
        }
    }
}

impl Serialize for JudgementResult {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.to_raw().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for JudgementResult {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = RawResult::deserialize(deserializer)?;
        let result = match (raw.label, raw.confidence, raw.judge_version) {
            (JudgementLabel::Unjudged, None, None) if raw.receipt.is_none() => {
                if raw.judgement != JudgementKind::EventKind {
                    return Err(serde::de::Error::custom("only an event mark is unjudged"));
                }
                let index = raw
                    .event_index
                    .ok_or_else(|| serde::de::Error::custom("an event mark needs its index"))?;
                Self::unjudged_event(index)
            }
            (JudgementLabel::Unjudged, _, _) => {
                return Err(serde::de::Error::custom(
                    "an unjudged mark carries no judgement",
                ));
            }
            (label, Some(confidence), Some(judge_version)) => Self::judged(
                raw.judgement,
                raw.event_index,
                label,
                confidence,
                judge_version,
                raw.receipt,
            ),
            _ => {
                return Err(serde::de::Error::custom(
                    "a judged result needs its confidence and judge version",
                ));
            }
        };
        result.map_err(serde::de::Error::custom)
    }
}

/// A coarse bucket of the account's remaining monthly budget, never a
/// dollar figure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpendBucket {
    OverHalf,
    UnderHalf,
    UnderTenth,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JudgementSpend {
    pub account_month_remaining_bucket: SpendBucket,
}

// ---------------------------------------------------------------------------
// Request
// ---------------------------------------------------------------------------

/// `POST /v1/judgement` body. `Debug` shows the digest's hash, never its
/// text: the digest is scrubbed contributor prompts and excerpts.
#[derive(Clone, PartialEq, Eq)]
pub struct JudgementRequest {
    digest: String,
    judgements: Vec<JudgementKind>,
}

#[derive(Deserialize)]
struct VersionOnly {
    wire_version: u32,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRequest {
    wire_version: u32,
    digest: String,
    judgements: Vec<JudgementKind>,
}

impl std::fmt::Debug for JudgementRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JudgementRequest")
            .field("digest_sha256", &self.digest_sha256())
            .field("judgements", &self.judgements)
            .finish()
    }
}

impl JudgementRequest {
    /// The digest must be non-empty and at most
    /// [`JUDGEMENT_DIGEST_MAX_BYTES`] bytes; the judgements non-empty and
    /// distinct.
    pub fn new(digest: String, judgements: Vec<JudgementKind>) -> Result<Self, JudgementRefusal> {
        if digest.len() > JUDGEMENT_DIGEST_MAX_BYTES {
            return Err(JudgementRefusal::DigestTooLarge);
        }
        if digest.is_empty() || judgements.is_empty() {
            return Err(JudgementRefusal::RequestMalformed);
        }
        let distinct: BTreeSet<_> = judgements.iter().collect();
        if distinct.len() != judgements.len() {
            return Err(JudgementRefusal::RequestMalformed);
        }
        Ok(Self { digest, judgements })
    }

    pub fn digest(&self) -> &str {
        &self.digest
    }

    pub fn judgements(&self) -> &[JudgementKind] {
        &self.judgements
    }

    /// Lowercase hex SHA-256 of the digest bytes: the only thing about the
    /// digest that may be logged.
    pub fn digest_sha256(&self) -> String {
        hex::encode(Sha256::digest(self.digest.as_bytes()))
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(&RawRequest {
            wire_version: JUDGEMENT_WIRE_VERSION,
            digest: self.digest.clone(),
            judgements: self.judgements.clone(),
        })
        .expect("a judgement request is JSON serializable")
    }
}

/// Parse a request body into the refusal the service sends for it.
///
/// Order: the body cap, then the wire version (read leniently, so a later
/// version with new fields is reported as a version, not as malformed),
/// then the closed shape, then the digest cap.
pub fn parse_judgement_request(body: &[u8]) -> Result<JudgementRequest, JudgementRefusal> {
    if body.len() > JUDGEMENT_REQUEST_MAX_BYTES {
        return Err(JudgementRefusal::DigestTooLarge);
    }
    let version: VersionOnly =
        serde_json::from_slice(body).map_err(|_| JudgementRefusal::RequestMalformed)?;
    if version.wire_version != JUDGEMENT_WIRE_VERSION {
        return Err(JudgementRefusal::WireVersionUnsupported);
    }
    let raw: RawRequest =
        serde_json::from_slice(body).map_err(|_| JudgementRefusal::RequestMalformed)?;
    JudgementRequest::new(raw.digest, raw.judgements)
}

// ---------------------------------------------------------------------------
// Response
// ---------------------------------------------------------------------------

/// `200` body of `POST /v1/judgement`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JudgementResponse {
    digest_sha256: String,
    results: Vec<JudgementResult>,
    spend: JudgementSpend,
    signed_at: i64,
    signature: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawResponse {
    wire_version: u32,
    digest_sha256: String,
    results: Vec<JudgementResult>,
    spend: JudgementSpend,
    signed_at: i64,
    signature: String,
}

impl JudgementResponse {
    /// `digest_sha256` is 64 lowercase hex; `results` is non-empty with no
    /// two results for the same judgement and event index; `signature` is
    /// `0x` and 130 lowercase hex (65 bytes, `r || s || v`).
    ///
    /// This checks shape only. Recovering the signer and comparing the
    /// nonce is the client's job.
    pub fn new(
        digest_sha256: String,
        results: Vec<JudgementResult>,
        spend: JudgementSpend,
        signed_at: i64,
        signature: String,
    ) -> Result<Self, JudgementWireError> {
        if !is_lower_hex(&digest_sha256, SHA256_HEX_LEN) {
            return Err(JudgementWireError::Malformed("digest_sha256"));
        }
        if results.is_empty() {
            return Err(JudgementWireError::Malformed("results"));
        }
        let keys: BTreeSet<_> = results
            .iter()
            .map(|r| (r.judgement, r.event_index))
            .collect();
        if keys.len() != results.len() {
            return Err(JudgementWireError::Malformed("results"));
        }
        let signature_hex_ok = signature
            .strip_prefix("0x")
            .is_some_and(|hex| is_lower_hex(hex, SIGNATURE_BYTES * 2));
        if !signature_hex_ok {
            return Err(JudgementWireError::Malformed("signature"));
        }
        Ok(Self {
            digest_sha256,
            results,
            spend,
            signed_at,
            signature,
        })
    }

    pub fn digest_sha256(&self) -> &str {
        &self.digest_sha256
    }

    pub fn results(&self) -> &[JudgementResult] {
        &self.results
    }

    pub fn spend(&self) -> JudgementSpend {
        self.spend
    }

    pub fn signed_at(&self) -> i64 {
        self.signed_at
    }

    pub fn signature(&self) -> &str {
        &self.signature
    }

    /// The bytes `signature` is over, for the nonce this exchange used.
    pub fn signing_bytes(&self, nonce: &JudgementNonce) -> Vec<u8> {
        judgement_signing_bytes(nonce, &self.digest_sha256, &self.results, self.signed_at)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(&RawResponse {
            wire_version: JUDGEMENT_WIRE_VERSION,
            digest_sha256: self.digest_sha256.clone(),
            results: self.results.clone(),
            spend: self.spend,
            signed_at: self.signed_at,
            signature: self.signature.clone(),
        })
        .expect("a judgement response is JSON serializable")
    }
}

/// Parse a `200` body, closed: unknown fields, unknown labels and
/// inconsistent results are refused.
pub fn parse_judgement_response(body: &[u8]) -> Result<JudgementResponse, JudgementWireError> {
    let raw: RawResponse =
        serde_json::from_slice(body).map_err(|_| JudgementWireError::Malformed("response"))?;
    if raw.wire_version != JUDGEMENT_WIRE_VERSION {
        return Err(JudgementWireError::WireVersionUnsupported);
    }
    JudgementResponse::new(
        raw.digest_sha256,
        raw.results,
        raw.spend,
        raw.signed_at,
        raw.signature,
    )
}

// ---------------------------------------------------------------------------
// Signing bytes
// ---------------------------------------------------------------------------

fn append_length_prefixed(out: &mut Vec<u8>, field: &[u8]) {
    out.extend_from_slice(&(field.len() as u64).to_le_bytes());
    out.extend_from_slice(field);
}

/// The canonical bytes the service signs (EIP-191) and the client verifies.
///
/// A fixed-order, length-prefixed encoder rather than `Serialize`, for the
/// reason the witness certificate gives: `serde_json/preserve_order` once
/// shifted every untyped-JSON digest. Layout:
///
/// - [`JUDGEMENT_SIGNING_DOMAIN`] (carries the wire version);
/// - the nonce, 32 raw bytes;
/// - `digest_sha256` as length-prefixed ASCII;
/// - the result count, u64 LE, then per result:
///   - judgement tag (1 task, 2 event_kind, 3 finished);
///   - event index: 0 absent, or 1 then u32 LE;
///   - label as length-prefixed wire string;
///   - 0 unjudged, or 1 then: bucket tag (1 high, 2 medium, 3 low), basis
///     tag (1 probability, 2 self_reported), length-prefixed
///     `prompt_version`, length-prefixed `model_id`, then receipt: 0 absent,
///     or 1, kind tag (1 provider_tee), length-prefixed model,
///     length-prefixed signer;
/// - `signed_at`, i64 LE.
///
/// Every length prefix is u64 LE. `spend` and `wire_version` as a field are
/// not signed, as the spec lists the signed fields; the version is in the
/// domain string.
pub fn judgement_signing_bytes(
    nonce: &JudgementNonce,
    digest_sha256: &str,
    results: &[JudgementResult],
    signed_at: i64,
) -> Vec<u8> {
    let mut out = JUDGEMENT_SIGNING_DOMAIN.to_vec();
    out.extend_from_slice(nonce.as_bytes());
    append_length_prefixed(&mut out, digest_sha256.as_bytes());
    out.extend_from_slice(&(results.len() as u64).to_le_bytes());
    for result in results {
        out.push(result.judgement.tag());
        match result.event_index {
            None => out.push(0),
            Some(index) => {
                out.push(1);
                out.extend_from_slice(&index.to_le_bytes());
            }
        }
        append_length_prefixed(&mut out, result.label.as_wire().as_bytes());
        match &result.judged {
            None => out.push(0),
            Some(judged) => {
                out.push(1);
                out.push(judged.confidence.bucket.tag());
                out.push(judged.confidence.basis.tag());
                append_length_prefixed(&mut out, judged.judge_version.prompt_version.as_bytes());
                append_length_prefixed(&mut out, judged.judge_version.model_id.as_bytes());
                match &judged.receipt {
                    None => out.push(0),
                    Some(receipt) => {
                        out.push(1);
                        out.push(receipt.signature_kind.tag());
                        append_length_prefixed(&mut out, receipt.model.as_bytes());
                        append_length_prefixed(&mut out, receipt.signer.as_bytes());
                    }
                }
            }
        }
    }
    out.extend_from_slice(&signed_at.to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const FROZEN_VECTOR: &str = concat!(
        // domain "trace_commons.insights_judgement.v1\n"
        "74726163655f636f6d6d6f6e732e696e7369676874735f6a756467656d656e742e76310a",
        // nonce 00..1f
        "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
        // digest_sha256, length-prefixed hex
        "4000000000000000",
        "6162616261626162616261626162616261626162616261626162616261626162",
        "6162616261626162616261626162616261626162616261626162616261626162",
        // three results
        "0300000000000000",
        // task, no index, "bug_fix", judged, high, probability
        "0100",
        "0700000000000000",
        "6275675f666978",
        "010101",
        "1b00000000000000",
        "7461736b2d76312b7368613235363a303132333435363738396162",
        "0100000000000000",
        "6d",
        // receipt present, provider_tee, model "m", signer
        "0101",
        "0100000000000000",
        "6d",
        "4000000000000000",
        "6364636463646364636463646364636463646364636463646364636463646364",
        "6364636463646364636463646364636463646364636463646364636463646364",
        // event_kind, index 4, "unjudged", not judged
        "0201",
        "04000000",
        "0800000000000000",
        "756e6a756467656400",
        // finished, no index, "unclear", judged, low, self_reported
        "0300",
        "0700000000000000",
        "756e636c656172",
        "010302",
        "1f00000000000000",
        "66696e69736865642d76312b7368613235363a626139383736353433323130",
        "0100000000000000",
        "6d",
        // no receipt
        "00",
        // signed_at 1792540800, i64 LE
        "8000d86a00000000",
    );

    fn nonce() -> JudgementNonce {
        let bytes: [u8; 32] = std::array::from_fn(|i| i as u8);
        JudgementNonce::from_bytes(bytes)
    }

    fn version(prompt: &str) -> JudgeVersion {
        JudgeVersion::new(prompt.to_string(), "m".to_string()).unwrap()
    }

    fn receipt() -> JudgementReceipt {
        JudgementReceipt::new("m".to_string(), "cd".repeat(32)).unwrap()
    }

    fn results() -> Vec<JudgementResult> {
        vec![
            JudgementResult::judged(
                JudgementKind::Task,
                None,
                JudgementLabel::BugFix,
                JudgementConfidence {
                    bucket: ConfidenceBucket::High,
                    basis: ConfidenceBasis::Probability,
                },
                version("task-v1+sha256:0123456789ab"),
                Some(receipt()),
            )
            .unwrap(),
            JudgementResult::unjudged_event(4).unwrap(),
            JudgementResult::judged(
                JudgementKind::Finished,
                None,
                JudgementLabel::Unclear,
                JudgementConfidence {
                    bucket: ConfidenceBucket::Low,
                    basis: ConfidenceBasis::SelfReported,
                },
                version("finished-v1+sha256:ba9876543210"),
                None,
            )
            .unwrap(),
        ]
    }

    fn response() -> JudgementResponse {
        JudgementResponse::new(
            "ab".repeat(32),
            results(),
            JudgementSpend {
                account_month_remaining_bucket: SpendBucket::OverHalf,
            },
            1_792_540_800,
            format!("0x{}", "ef".repeat(65)),
        )
        .unwrap()
    }

    // ---- constants and refusals ----

    #[test]
    fn wire_constants_are_exact() {
        assert_eq!(JUDGEMENT_WIRE_VERSION, 1);
        assert_eq!(JUDGEMENT_DIGEST_MAX_BYTES, 8192);
        assert_eq!(JUDGEMENT_REQUEST_MAX_BYTES, 16 * 1024);
        assert_eq!(JUDGEMENT_NONCE_HEADER, "x-trace-judgement-nonce");
        assert_eq!(JUDGEMENT_SATURATED_RETRY_AFTER_SECS, 30);
        assert_eq!(JUDGEMENT_EVENT_KIND_MAX, 4);
        assert_eq!(
            JUDGEMENT_SIGNING_DOMAIN,
            b"trace_commons.insights_judgement.v1\n"
        );
    }

    /// Literal strings, not `label()`, so the test is not derived from the
    /// table it checks.
    #[test]
    fn refusal_labels_and_statuses_are_exact() {
        assert_eq!(
            JudgementRefusal::ALL.map(|r| (r.label(), r.status())),
            [
                ("judgement_saturated", 503),
                ("judgement_account_budget_exhausted", 503),
                ("judgement_operator_budget_exhausted", 503),
                ("judgement_request_malformed", 400),
                ("judgement_digest_too_large", 413),
                ("judgement_wire_version_unsupported", 400),
                ("judgement_credential_refused", 401),
                ("judgement_account_ineligible", 403),
                ("judgement_model_unattested", 502),
                ("judgement_timeout", 504),
            ]
        );
    }

    #[test]
    fn only_the_three_capacity_labels_on_503_are_capacity() {
        assert!(is_judgement_capacity(503, "judgement_saturated"));
        assert!(is_judgement_capacity(
            503,
            "judgement_account_budget_exhausted"
        ));
        assert!(is_judgement_capacity(
            503,
            "judgement_operator_budget_exhausted"
        ));
        assert!(!is_judgement_capacity(429, "judgement_saturated"));
        assert!(!is_judgement_capacity(504, "judgement_saturated"));
        assert!(!is_judgement_capacity(503, "judgement_saturated_extra"));
        assert!(!is_judgement_capacity(503, "witness_saturated"));
        // A model or timeout failure is not capacity, whatever its status.
        assert!(!is_judgement_capacity(503, "judgement_model_unattested"));
        assert!(!is_judgement_capacity(503, "judgement_timeout"));
        assert!(!is_judgement_capacity(502, "judgement_model_unattested"));
        assert!(!is_judgement_capacity(504, "judgement_timeout"));
        let capacity: Vec<_> = JudgementRefusal::ALL
            .into_iter()
            .filter(|r| r.is_capacity())
            .map(JudgementRefusal::label)
            .collect();
        assert_eq!(
            capacity,
            [
                "judgement_saturated",
                "judgement_account_budget_exhausted",
                "judgement_operator_budget_exhausted",
            ]
        );
    }

    #[test]
    fn a_refusal_is_recognised_only_when_status_and_label_agree() {
        for refusal in JudgementRefusal::ALL {
            assert_eq!(
                JudgementRefusal::from_response(refusal.status(), refusal.label()),
                Some(refusal)
            );
            assert_eq!(JudgementRefusal::from_label(refusal.label()), Some(refusal));
        }
        assert_eq!(
            JudgementRefusal::from_response(400, "judgement_saturated"),
            None
        );
        assert_eq!(JudgementRefusal::from_response(503, "unknown"), None);
        assert_eq!(JudgementRefusal::from_label("witness_saturated"), None);
    }

    #[test]
    fn only_budget_refusals_carry_reset_at() {
        let reset = 1_793_491_200; // 2026-11-01T00:00:00Z
        for refusal in JudgementRefusal::ALL {
            let budget = matches!(
                refusal,
                JudgementRefusal::AccountBudgetExhausted
                    | JudgementRefusal::OperatorBudgetExhausted
            );
            assert_eq!(refusal.carries_reset_at(), budget, "{refusal:?}");
            assert_eq!(
                JudgementRefusalBody::new(refusal, Some(reset)).is_ok(),
                budget
            );
            assert_eq!(JudgementRefusalBody::new(refusal, None).is_ok(), !budget);
        }
        let body = JudgementRefusalBody::new(JudgementRefusal::AccountBudgetExhausted, Some(reset))
            .unwrap();
        let json = body.to_json();
        assert_eq!(
            json,
            r#"{"error":"judgement_account_budget_exhausted","reset_at":1793491200}"#
        );
        assert_eq!(
            parse_judgement_refusal(503, json.as_bytes()),
            Some(body.clone())
        );
        assert_eq!(body.refusal(), JudgementRefusal::AccountBudgetExhausted);
        assert_eq!(body.reset_at(), Some(reset));
        // The constructor applies the same month-start rule, so the service
        // cannot build a body the client would refuse.
        for bad in [i64::MAX, i64::MIN, -1, reset + 1, 1_792_497_600] {
            assert_eq!(
                JudgementRefusalBody::new(JudgementRefusal::OperatorBudgetExhausted, Some(bad)),
                Err(JudgementWireError::Malformed("reset_at")),
                "{bad}"
            );
        }
        // 2027-01-01T00:00:00Z, a year boundary, is a month start.
        assert!(
            JudgementRefusalBody::new(
                JudgementRefusal::AccountBudgetExhausted,
                Some(1_798_761_600)
            )
            .is_ok()
        );
        let plain = JudgementRefusalBody::new(JudgementRefusal::Timeout, None).unwrap();
        assert_eq!(plain.to_json(), r#"{"error":"judgement_timeout"}"#);
        assert_eq!(
            parse_judgement_refusal(504, plain.to_json().as_bytes()),
            Some(plain)
        );
    }

    #[test]
    fn a_refusal_body_is_closed() {
        for (status, body) in [
            // status and label disagree
            (400, r#"{"error":"judgement_saturated"}"#),
            // budget label without reset_at
            (503, r#"{"error":"judgement_operator_budget_exhausted"}"#),
            // reset_at on a label that never carries it
            (503, r#"{"error":"judgement_saturated","reset_at":1}"#),
            // unknown field
            (504, r#"{"error":"judgement_timeout","detail":"x"}"#),
            // unknown label
            (503, r#"{"error":"judgement_bogus"}"#),
            (503, "not json"),
            // reset_at that is not the first instant of a UTC month
            (
                503,
                r#"{"error":"judgement_account_budget_exhausted","reset_at":9223372036854775807}"#,
            ),
            (
                503,
                r#"{"error":"judgement_account_budget_exhausted","reset_at":-1}"#,
            ),
            (
                503,
                r#"{"error":"judgement_operator_budget_exhausted","reset_at":-9223372036854775808}"#,
            ),
            // 2026-11-01T00:00:01Z, one second past a month start
            (
                503,
                r#"{"error":"judgement_operator_budget_exhausted","reset_at":1793491201}"#,
            ),
            // 2026-10-20T12:00:00Z, mid-month
            (
                503,
                r#"{"error":"judgement_account_budget_exhausted","reset_at":1792497600}"#,
            ),
        ] {
            assert_eq!(
                parse_judgement_refusal(status, body.as_bytes()),
                None,
                "{body}"
            );
        }
    }

    #[test]
    fn budget_reset_is_the_first_instant_of_the_next_utc_month() {
        // 2026-10-20T12:00:00Z -> 2026-11-01T00:00:00Z
        assert_eq!(
            judgement_budget_reset_at(1_792_497_600),
            Some(1_793_491_200)
        );
        // 2026-12-31T23:59:59Z -> 2027-01-01T00:00:00Z
        assert_eq!(
            judgement_budget_reset_at(1_798_761_599),
            Some(1_798_761_600)
        );
        // The first instant of a month resets at the next month, not itself.
        assert_eq!(
            judgement_budget_reset_at(1_793_491_200),
            Some(1_796_083_200)
        );
    }

    // ---- nonce ----

    #[test]
    fn a_nonce_is_exactly_64_lowercase_hex() {
        let hex = "0f".repeat(32);
        assert_eq!(JudgementNonce::from_hex(&hex).unwrap().to_hex(), hex);
        assert_eq!(
            JudgementNonce::from_hex(&hex).unwrap().as_bytes(),
            &[0x0f; 32]
        );
        for bad in [
            "0F".repeat(32),
            "0f".repeat(31),
            "0f".repeat(33),
            "zz".repeat(32),
            String::new(),
        ] {
            assert!(JudgementNonce::from_hex(&bad).is_err(), "{bad}");
        }
    }

    // ---- request ----

    fn request_json(version: u32, digest: &str, judgements: &str) -> String {
        format!(
            r#"{{"wire_version":{version},"digest":{},"judgements":{judgements}}}"#,
            serde_json::to_string(digest).unwrap()
        )
    }

    #[test]
    fn a_request_parses_and_hashes_its_digest() {
        let body = request_json(1, "{\"turns\":3}", r#"["task","event_kind","finished"]"#);
        let request = parse_judgement_request(body.as_bytes()).unwrap();
        assert_eq!(request.digest(), "{\"turns\":3}");
        assert_eq!(
            request.judgements(),
            [
                JudgementKind::Task,
                JudgementKind::EventKind,
                JudgementKind::Finished
            ]
        );
        // sha256("{\"turns\":3}")
        use sha2::{Digest as _, Sha256};
        assert_eq!(
            request.digest_sha256(),
            hex::encode(Sha256::digest(b"{\"turns\":3}"))
        );
        let rebuilt = JudgementRequest::new(
            "{\"turns\":3}".to_string(),
            vec![
                JudgementKind::Task,
                JudgementKind::EventKind,
                JudgementKind::Finished,
            ],
        )
        .unwrap();
        assert_eq!(rebuilt, request);
        assert_eq!(
            parse_judgement_request(rebuilt.to_json().as_bytes()).unwrap(),
            request
        );
    }

    #[test]
    fn the_digest_cap_is_8192_bytes_and_refused_whole() {
        let at_cap = "a".repeat(JUDGEMENT_DIGEST_MAX_BYTES);
        assert!(
            parse_judgement_request(request_json(1, &at_cap, r#"["task"]"#).as_bytes()).is_ok()
        );
        let over = "a".repeat(JUDGEMENT_DIGEST_MAX_BYTES + 1);
        assert_eq!(
            parse_judgement_request(request_json(1, &over, r#"["task"]"#).as_bytes()),
            Err(JudgementRefusal::DigestTooLarge)
        );
        assert_eq!(
            JudgementRequest::new(over, vec![JudgementKind::Task]),
            Err(JudgementRefusal::DigestTooLarge)
        );
        // Multi-byte characters count as bytes, not characters.
        let multibyte = "\u{e9}".repeat(JUDGEMENT_DIGEST_MAX_BYTES / 2 + 1);
        assert_eq!(
            JudgementRequest::new(multibyte, vec![JudgementKind::Task]),
            Err(JudgementRefusal::DigestTooLarge)
        );
        // A body over the total cap is refused before it is parsed.
        let mut huge = request_json(1, "x", r#"["task"]"#).into_bytes();
        huge.resize(JUDGEMENT_REQUEST_MAX_BYTES + 1, b' ');
        assert_eq!(
            parse_judgement_request(&huge),
            Err(JudgementRefusal::DigestTooLarge)
        );
    }

    #[test]
    fn an_unknown_wire_version_is_unsupported_not_malformed() {
        assert_eq!(
            parse_judgement_request(request_json(2, "x", r#"["task"]"#).as_bytes()),
            Err(JudgementRefusal::WireVersionUnsupported)
        );
        // A later version may add fields; it is still reported as a version.
        let later = r#"{"wire_version":2,"digest":"x","judgements":["task"],"new":1}"#;
        assert_eq!(
            parse_judgement_request(later.as_bytes()),
            Err(JudgementRefusal::WireVersionUnsupported)
        );
    }

    #[test]
    fn a_malformed_request_is_refused() {
        for body in [
            "not json".to_string(),
            r#"{"digest":"x","judgements":["task"]}"#.to_string(),
            request_json(1, "x", "[]"),
            request_json(1, "x", r#"["task","task"]"#),
            request_json(1, "x", r#"["mood"]"#),
            request_json(1, "", r#"["task"]"#),
            r#"{"wire_version":1,"digest":"x","judgements":["task"],"extra":1}"#.to_string(),
        ] {
            assert_eq!(
                parse_judgement_request(body.as_bytes()),
                Err(JudgementRefusal::RequestMalformed),
                "{body}"
            );
        }
    }

    #[test]
    fn request_debug_never_shows_the_digest() {
        let request = JudgementRequest::new(
            "secret contributor prompt".to_string(),
            vec![JudgementKind::Task],
        )
        .unwrap();
        let debug = format!("{request:?}");
        assert!(!debug.contains("secret contributor prompt"));
        assert!(debug.contains(&request.digest_sha256()));
    }

    // ---- results ----

    #[test]
    fn label_sets_are_exact_and_closed() {
        let wire = |kind: JudgementKind| -> Vec<&'static str> {
            kind.choices().iter().map(|l| l.as_wire()).collect()
        };
        assert_eq!(
            wire(JudgementKind::Task),
            ["bug_fix", "refactor", "tests", "docs", "other"]
        );
        assert_eq!(
            wire(JudgementKind::EventKind),
            ["rejection", "redirect", "iteration", "unclear"]
        );
        assert_eq!(wire(JudgementKind::Finished), ["yes", "no", "unclear"]);
        assert_eq!(JudgementKind::Task.as_wire(), "task");
        assert_eq!(JudgementKind::EventKind.as_wire(), "event_kind");
        assert_eq!(JudgementKind::Finished.as_wire(), "finished");
        assert_eq!(JudgementLabel::Unjudged.as_wire(), "unjudged");
    }

    #[test]
    fn a_result_must_be_consistent_with_its_judgement() {
        let confidence = JudgementConfidence {
            bucket: ConfidenceBucket::Medium,
            basis: ConfidenceBasis::SelfReported,
        };
        let judged = |kind, index, label, receipt| {
            JudgementResult::judged(
                kind,
                index,
                label,
                confidence,
                version("task-v1+sha256:0123456789ab"),
                receipt,
            )
        };
        // A label from another judgement's set.
        assert!(judged(JudgementKind::Task, None, JudgementLabel::Yes, None).is_err());
        assert!(
            judged(
                JudgementKind::Finished,
                None,
                JudgementLabel::Redirect,
                None
            )
            .is_err()
        );
        // Unjudged is never a judged label.
        assert!(
            judged(
                JudgementKind::EventKind,
                Some(0),
                JudgementLabel::Unjudged,
                None
            )
            .is_err()
        );
        // event_index is present exactly for event_kind.
        assert!(judged(JudgementKind::Task, Some(0), JudgementLabel::Docs, None).is_err());
        assert!(
            judged(
                JudgementKind::EventKind,
                None,
                JudgementLabel::Redirect,
                None
            )
            .is_err()
        );
        // Only the first four marks are judged; the rest come back unjudged.
        assert!(
            judged(
                JudgementKind::EventKind,
                Some(3),
                JudgementLabel::Redirect,
                None
            )
            .is_ok()
        );
        assert!(
            judged(
                JudgementKind::EventKind,
                Some(4),
                JudgementLabel::Redirect,
                None
            )
            .is_err()
        );
        assert!(JudgementResult::unjudged_event(3).is_err());
        assert!(JudgementResult::unjudged_event(4).is_ok());
        // A receipt must name the model the judge version names.
        let other = JudgementReceipt::new("other".to_string(), "cd".repeat(32)).unwrap();
        assert!(judged(JudgementKind::Task, None, JudgementLabel::Docs, Some(other)).is_err());
        assert!(
            judged(
                JudgementKind::Task,
                None,
                JudgementLabel::Docs,
                Some(receipt())
            )
            .is_ok()
        );
    }

    #[test]
    fn judge_versions_and_receipts_are_validated() {
        for good in [
            "task-v1+sha256:0123456789ab",
            "event-kind-v12+sha256:abcdefabcdef",
        ] {
            assert!(
                JudgeVersion::new(good.to_string(), "m".to_string()).is_ok(),
                "{good}"
            );
        }
        for bad in [
            "task+sha256:0123456789ab",
            "task-v0+sha256:0123456789ab",
            "task-v01+sha256:0123456789ab",
            "task-v1+sha256:0123456789AB",
            "task-v1+sha256:0123456789a",
            "task-v1+sha256:0123456789abc",
            "Task-v1+sha256:0123456789ab",
            "-v1+sha256:0123456789ab",
            "task-v1",
        ] {
            assert!(
                JudgeVersion::new(bad.to_string(), "m".to_string()).is_err(),
                "{bad}"
            );
        }
        assert!(
            JudgeVersion::new("task-v1+sha256:0123456789ab".to_string(), String::new()).is_err()
        );
        assert!(JudgementReceipt::new(String::new(), "cd".repeat(32)).is_err());
        for signer in [
            "CD".repeat(32),
            "cd".repeat(20),
            format!("0x{}", "cd".repeat(32)),
        ] {
            assert!(
                JudgementReceipt::new("m".to_string(), signer.clone()).is_err(),
                "{signer}"
            );
        }
    }

    /// The only receipt kind the service returns is NEAR AI's `provider_tee`,
    /// spelled as the admission vocabulary spells it.
    #[test]
    fn receipt_kind_spelling_matches_admission() {
        assert_eq!(
            JudgementReceiptKind::ProviderTee.as_wire(),
            crate::admission::AdmissionSignatureKind::ProviderTee.as_wire()
        );
    }

    // ---- response ----

    #[test]
    fn a_response_round_trips_in_the_spec_shape() {
        let response = response();
        let json = response.to_json();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["wire_version"], 1);
        assert_eq!(value["results"][0]["judgement"], "task");
        assert_eq!(value["results"][0]["label"], "bug_fix");
        assert!(value["results"][0].get("event_index").is_none());
        assert_eq!(value["results"][0]["confidence"]["bucket"], "high");
        assert_eq!(value["results"][0]["confidence"]["basis"], "probability");
        assert_eq!(
            value["results"][0]["judge_version"]["prompt_version"],
            "task-v1+sha256:0123456789ab"
        );
        assert_eq!(
            value["results"][0]["receipt"]["signature_kind"],
            "provider_tee"
        );
        assert_eq!(value["results"][0]["receipt"]["verified"], true);
        assert_eq!(
            value["results"][1],
            serde_json::json!({"judgement": "event_kind", "event_index": 4, "label": "unjudged"})
        );
        assert!(value["results"][2].get("receipt").is_none());
        assert_eq!(value["results"][2]["confidence"]["basis"], "self_reported");
        assert_eq!(
            value["spend"]["account_month_remaining_bucket"],
            "over_half"
        );
        assert_eq!(value["signed_at"], 1_792_540_800);
        assert_eq!(parse_judgement_response(json.as_bytes()).unwrap(), response);
        assert!(response.results()[0].is_attested());
        assert!(!response.results()[2].is_attested());
        assert!(!response.results()[1].is_attested());
    }

    #[test]
    fn a_malformed_response_is_refused() {
        let good: serde_json::Value = serde_json::from_str(&response().to_json()).unwrap();
        let mutations: Vec<(&str, Box<dyn Fn(&mut serde_json::Value)>)> = vec![
            ("wire version", Box::new(|v| v["wire_version"] = 2.into())),
            ("unknown top field", Box::new(|v| v["extra"] = 1.into())),
            (
                "digest hex",
                Box::new(|v| v["digest_sha256"] = "AB".repeat(32).into()),
            ),
            (
                "signature length",
                Box::new(|v| v["signature"] = format!("0x{}", "ef".repeat(64)).into()),
            ),
            (
                "signature prefix",
                Box::new(|v| v["signature"] = "ef".repeat(65).into()),
            ),
            (
                "no results",
                Box::new(|v| v["results"] = serde_json::json!([])),
            ),
            (
                "verified false",
                Box::new(|v| v["results"][0]["receipt"]["verified"] = false.into()),
            ),
            (
                "gateway receipt",
                Box::new(|v| v["results"][0]["receipt"]["signature_kind"] = "gateway".into()),
            ),
            (
                "receipt hashes",
                Box::new(|v| v["results"][0]["receipt"]["request_sha256"] = "aa".into()),
            ),
            (
                "unknown result field",
                Box::new(|v| v["results"][0]["chat_id"] = "x".into()),
            ),
            (
                "label set",
                Box::new(|v| v["results"][0]["label"] = "yes".into()),
            ),
            (
                "unjudged with confidence",
                Box::new(|v| {
                    v["results"][1]["confidence"] =
                        serde_json::json!({"bucket": "low", "basis": "probability"})
                }),
            ),
            (
                "judged without confidence",
                Box::new(|v| {
                    v["results"][0]
                        .as_object_mut()
                        .unwrap()
                        .remove("confidence");
                }),
            ),
            (
                "duplicate result",
                Box::new(|v| {
                    let first = v["results"][0].clone();
                    v["results"].as_array_mut().unwrap().push(first);
                }),
            ),
            (
                "spend bucket",
                Box::new(|v| v["spend"]["account_month_remaining_bucket"] = "4.20".into()),
            ),
        ];
        for (name, mutate) in mutations {
            let mut value = good.clone();
            mutate(&mut value);
            assert!(
                parse_judgement_response(value.to_string().as_bytes()).is_err(),
                "{name}"
            );
        }
        // Two distinct event marks are not duplicates.
        let mut value = good.clone();
        let mut second = value["results"][1].clone();
        second["event_index"] = 5.into();
        value["results"].as_array_mut().unwrap().push(second);
        assert!(parse_judgement_response(value.to_string().as_bytes()).is_ok());
    }

    // ---- signing bytes ----

    #[test]
    fn signing_bytes_frozen_vector() {
        assert_eq!(
            hex::encode(response().signing_bytes(&nonce())),
            FROZEN_VECTOR
        );
        assert_eq!(
            judgement_signing_bytes(&nonce(), &"ab".repeat(32), &results(), 1_792_540_800),
            response().signing_bytes(&nonce())
        );
    }

    /// Changing any signed field changes the bytes.
    #[test]
    fn every_signed_field_moves_the_signing_bytes() {
        let original = response().signing_bytes(&nonce());
        let digest = "ab".repeat(32);
        let sign = |n: &JudgementNonce, d: &str, r: &[JudgementResult], t: i64| {
            judgement_signing_bytes(n, d, r, t)
        };
        let mut other_nonce = [0u8; 32];
        other_nonce[31] = 1;
        assert_ne!(
            original,
            sign(
                &JudgementNonce::from_bytes(other_nonce),
                &digest,
                &results(),
                1_792_540_800
            )
        );
        assert_ne!(
            original,
            sign(&nonce(), &"ac".repeat(32), &results(), 1_792_540_800)
        );
        assert_ne!(original, sign(&nonce(), &digest, &results(), 1_792_540_801));
        assert_ne!(
            original,
            sign(&nonce(), &digest, &results()[..2], 1_792_540_800)
        );
        let mut reordered = results();
        reordered.swap(0, 2);
        assert_ne!(original, sign(&nonce(), &digest, &reordered, 1_792_540_800));

        let base_conf = JudgementConfidence {
            bucket: ConfidenceBucket::High,
            basis: ConfidenceBasis::Probability,
        };
        let task = |label, conf, prompt: &str, model: &str, receipt: Option<JudgementReceipt>| {
            let mut r = results();
            r[0] = JudgementResult::judged(
                JudgementKind::Task,
                None,
                label,
                conf,
                JudgeVersion::new(prompt.to_string(), model.to_string()).unwrap(),
                receipt,
            )
            .unwrap();
            sign(&nonce(), &digest, &r, 1_792_540_800)
        };
        let p = "task-v1+sha256:0123456789ab";
        assert_eq!(
            original,
            task(JudgementLabel::BugFix, base_conf, p, "m", Some(receipt()))
        );
        assert_ne!(
            original,
            task(JudgementLabel::Refactor, base_conf, p, "m", Some(receipt()))
        );
        assert_ne!(
            original,
            task(
                JudgementLabel::BugFix,
                JudgementConfidence {
                    bucket: ConfidenceBucket::Medium,
                    ..base_conf
                },
                p,
                "m",
                Some(receipt())
            )
        );
        assert_ne!(
            original,
            task(
                JudgementLabel::BugFix,
                JudgementConfidence {
                    basis: ConfidenceBasis::SelfReported,
                    ..base_conf
                },
                p,
                "m",
                Some(receipt())
            )
        );
        assert_ne!(
            original,
            task(
                JudgementLabel::BugFix,
                base_conf,
                "task-v2+sha256:0123456789ab",
                "m",
                Some(receipt())
            )
        );
        assert_ne!(
            original,
            task(
                JudgementLabel::BugFix,
                base_conf,
                p,
                "n",
                Some(JudgementReceipt::new("n".to_string(), "cd".repeat(32)).unwrap())
            )
        );
        assert_ne!(
            original,
            task(JudgementLabel::BugFix, base_conf, p, "m", None)
        );
        assert_ne!(
            original,
            task(
                JudgementLabel::BugFix,
                base_conf,
                p,
                "m",
                Some(JudgementReceipt::new("m".to_string(), "ce".repeat(32)).unwrap())
            )
        );
        // Event index moves the bytes.
        let mut moved = results();
        moved[1] = JudgementResult::unjudged_event(5).unwrap();
        assert_ne!(original, sign(&nonce(), &digest, &moved, 1_792_540_800));
    }

    /// The spec signs the nonce, the digest hash, the results and the time.
    /// `wire_version` is carried by the domain string, and `spend` is
    /// deliberately outside the signature.
    #[test]
    fn spend_and_signature_are_not_signed() {
        let mut other = response();
        other.spend = JudgementSpend {
            account_month_remaining_bucket: SpendBucket::UnderTenth,
        };
        other.signature = format!("0x{}", "01".repeat(65));
        assert_eq!(
            other.signing_bytes(&nonce()),
            response().signing_bytes(&nonce())
        );
    }

    #[test]
    fn adjacent_boundary_shifts_do_not_collide() {
        // The digest hash is the one variable field next to a fixed one; a
        // byte moved across its end must not read alike.
        let r = results();
        assert_ne!(
            judgement_signing_bytes(&nonce(), "abc", &r, 0),
            judgement_signing_bytes(&nonce(), "ab", &r, 0)
        );
        // An absent event index and a present one cannot read alike.
        let task_only = vec![results().remove(0)];
        let event = vec![JudgementResult::unjudged_event(4).unwrap()];
        assert_ne!(
            judgement_signing_bytes(&nonce(), &"ab".repeat(32), &task_only, 0),
            judgement_signing_bytes(&nonce(), &"ab".repeat(32), &event, 0)
        );
    }
}
