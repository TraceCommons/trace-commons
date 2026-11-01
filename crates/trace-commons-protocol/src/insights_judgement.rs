//! Wire contract for the Insights judgement service.
//!
//! The service runs pinned judgements and returns labels signed by its
//! per-boot key: three (`task`, `event_kind`, `finished`) over one
//! session's judgement digest, and `stuck` over one turn's [`TurnDigest`],
//! which the contributor's economizer asks for alone, while the turn runs. This module holds the shared vocabulary only: the
//! attestation response, the request plaintext and the response that travel
//! inside the judgement seal, the two nonces, the closed refusal set, the
//! capacity rule, the digest and plaintext caps, the signing-bytes encoder
//! and the pure check that a response answers its send. It grants no trust
//! and does no cryptography: the seal, the report-data rebuild and the
//! signature recovery live in `trace-commons-attestation`. The service
//! verifies its own claims and receipts, and the client verifies the
//! service's quote and signature against its own pins.
//!
//! Design: `docs/superpowers/specs/2026-10-20-insights-judgement-service-design.md`,
//! section 3.
//!
//! Every type parses closed: unknown fields, unknown labels and inconsistent
//! combinations are refused rather than carried. The digest is scrubbed
//! contributor text and the claim is a bearer credential, so no `Debug` or
//! error here ever contains either.

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
/// The request plaintext cap: the JSON inside the seal, checked before it
/// is parsed. The daemon never builds a request over it.
pub const JUDGEMENT_REQUEST_MAX_BYTES: usize = 16 * 1024;
/// The sealed request body cap on the wire: the plaintext cap plus the
/// seal's 50 bytes (version byte, 33-byte compressed ephemeral key, 16-byte
/// AEAD tag). The service reads no more than this before opening the seal.
pub const JUDGEMENT_SEALED_REQUEST_MAX_BYTES: usize = JUDGEMENT_REQUEST_MAX_BYTES + 1 + 33 + 16;
/// `Retry-After` on `judgement_saturated`, as `witness_pacing`.
pub const JUDGEMENT_SATURATED_RETRY_AFTER_SECS: u32 = 30;
/// How far the client's clock may be from the service's when it reads a
/// budget refusal's `reset_at`; see [`JudgementRefusalBody::retry_not_before`].
pub const JUDGEMENT_RESET_AT_SKEW_SECS: i64 = 24 * 60 * 60;
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
///
/// A v1 refusal is plaintext and unsigned, so anything on the path can
/// send one. The lifetimes on the variants below ("never retry this
/// digest", "not today", "until `reset_at`") are what the service means by
/// each refusal; a client acts on an unsigned refusal only as
/// [`JudgementRefusal::unsigned_retry`] says, which defers for long only
/// where a forgery gains nothing. A signed refusal, which could carry those
/// lifetimes, is open on the design spec.
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
    /// The sealed request did not open, usually because the service
    /// restarted and has new per-boot keys. Re-verify a fresh quote and
    /// retry once.
    SealStale,
    /// This request nonce was already accepted. Never retried: a daemon
    /// never resends a request nonce, so this is a replay on the path.
    RequestReplayed,
    /// `GET /v1/attestation` only: the quote could not be produced within
    /// its own bound. Retry after 30 seconds. Not judgement capacity.
    QuoteUnavailable,
    /// An internal fault after spend may already be recorded: the response
    /// could not be signed, or the settle could not be written. Not today,
    /// because a retry would be charged again.
    ServiceFault,
}

impl JudgementRefusal {
    /// Every refusal, for tests and exhaustive mappings.
    pub const ALL: [JudgementRefusal; 14] = [
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
        JudgementRefusal::SealStale,
        JudgementRefusal::RequestReplayed,
        JudgementRefusal::QuoteUnavailable,
        JudgementRefusal::ServiceFault,
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
            Self::SealStale => "judgement_seal_stale",
            Self::RequestReplayed => "judgement_request_replayed",
            Self::QuoteUnavailable => "judgement_quote_unavailable",
            Self::ServiceFault => "judgement_service_fault",
        }
    }

    /// The status this refusal is sent with.
    pub const fn status(self) -> u16 {
        match self {
            Self::Saturated
            | Self::AccountBudgetExhausted
            | Self::OperatorBudgetExhausted
            | Self::QuoteUnavailable => 503,
            Self::RequestMalformed | Self::WireVersionUnsupported | Self::SealStale => 400,
            Self::RequestReplayed => 409,
            Self::ServiceFault => 500,
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
    /// `judgement_quote_unavailable` shares the 503 status but belongs to the
    /// attestation route, so it is not judgement capacity either.
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

    /// Whether the service sends this refusal on `route`. The attestation
    /// route sends only `judgement_request_malformed` (a bad quote nonce)
    /// and `judgement_quote_unavailable`; the judgement route sends every
    /// refusal but the latter.
    pub const fn sent_on(self, route: JudgementRoute) -> bool {
        match route {
            JudgementRoute::Attestation => {
                matches!(self, Self::RequestMalformed | Self::QuoteUnavailable)
            }
            JudgementRoute::Judgement => !matches!(self, Self::QuoteUnavailable),
        }
    }

    /// Recognise a refusal from a response, requiring the status, the label
    /// and the route to agree. A label on a status or a route it is never
    /// sent with is not this refusal, so a body cannot pick how a status is
    /// handled.
    pub fn from_response(route: JudgementRoute, status: u16, label: &str) -> Option<Self> {
        Self::from_label(label).filter(|r| r.status() == status && r.sent_on(route))
    }

    /// What a client does with this refusal as v1 sends it: plaintext and
    /// unsigned. Every refusal is a short retry except the three a forgery
    /// gains nothing from: `judgement_saturated` and
    /// `judgement_quote_unavailable` already ask for only a short wait, a
    /// stale seal sends the client back for a fresh quote it verifies
    /// itself, and the client can see for itself that a digest is over its
    /// cap.
    pub const fn unsigned_retry(self) -> JudgementRetry {
        match self {
            Self::Saturated | Self::QuoteUnavailable => JudgementRetry::AfterRetryAfter,
            Self::SealStale => JudgementRetry::FreshQuote,
            Self::DigestTooLarge => JudgementRetry::DropDigest,
            Self::AccountBudgetExhausted
            | Self::OperatorBudgetExhausted
            | Self::RequestMalformed
            | Self::WireVersionUnsupported
            | Self::CredentialRefused
            | Self::AccountIneligible
            | Self::ModelUnattested
            | Self::Timeout
            | Self::RequestReplayed
            | Self::ServiceFault => JudgementRetry::Soon,
        }
    }
}

/// The two routes a refusal can arrive on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JudgementRoute {
    /// `GET /v1/attestation`.
    Attestation,
    /// `POST /v1/judgement`.
    Judgement,
}

/// What a client does after an unsigned refusal; see
/// [`JudgementRefusal::unsigned_retry`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JudgementRetry {
    /// Wait for `Retry-After`, [`JUDGEMENT_SATURATED_RETRY_AFTER_SECS`].
    AfterRetryAfter,
    /// Fetch and verify a fresh quote, then retry once.
    FreshQuote,
    /// Do not send this digest again.
    DropDigest,
    /// Retry after the client's short backoff, a bounded number of times.
    Soon,
}

/// True only for a `503` on the judgement route carrying one of the three
/// capacity labels.
pub fn is_judgement_capacity(status: u16, error: &str) -> bool {
    JudgementRefusal::from_response(JudgementRoute::Judgement, status, error)
        .is_some_and(JudgementRefusal::is_capacity)
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
    /// [`judgement_budget_reset_at`] computes it. Which month is checked
    /// against the client's clock, by [`Self::retry_not_before`]: this
    /// constructor has no clock.
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

    /// `reset_at` as sent. Read [`Self::retry_not_before`] to act on it.
    pub fn reset_at(&self) -> Option<i64> {
        self.reset_at
    }

    /// The `reset_at` a client may act on at `now_unix`: the one sent when
    /// it is the next month start as seen within
    /// [`JUDGEMENT_RESET_AT_SKEW_SECS`] of `now_unix`, and otherwise the
    /// next month start by the client's own clock. `None` for a refusal
    /// without `reset_at`.
    ///
    /// Refusal bodies are not signed. Without this a forged or broken month
    /// start such as 2099-01-01 would turn an account's judgements off for
    /// good, and 1970-01-01 would have the client retry straight away
    /// against an exhausted cap.
    pub fn retry_not_before(&self, now_unix: i64) -> Option<i64> {
        let sent = self.reset_at?;
        let next = |at: i64| judgement_budget_reset_at(at);
        let earliest = now_unix
            .checked_sub(JUDGEMENT_RESET_AT_SKEW_SECS)
            .and_then(next);
        let latest = now_unix
            .checked_add(JUDGEMENT_RESET_AT_SKEW_SECS)
            .and_then(next);
        if earliest == Some(sent) || latest == Some(sent) {
            return Some(sent);
        }
        next(now_unix)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(&RawRefusalBody {
            error: self.refusal.label().to_string(),
            reset_at: self.reset_at,
        })
        .expect("a refusal body is JSON serializable")
    }
}

/// Parse a refusal body against the route and status it arrived with.
/// `None` for anything outside the closed set: an unknown label, a label on
/// the wrong status or route, an unknown field, a body that is not a JSON
/// object, or `reset_at` where the rule says otherwise.
pub fn parse_judgement_refusal(
    route: JudgementRoute,
    status: u16,
    body: &[u8],
) -> Option<JudgementRefusalBody> {
    let raw: RawRefusalBody = parse_closed(body, &[]).ok()?;
    let refusal = JudgementRefusal::from_response(route, status, &raw.error)?;
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

/// Deserialize a wire struct from a JSON object only. serde's derived
/// structs also accept a JSON array of their fields in order, which
/// `deny_unknown_fields` does not stop, so every parse here first requires
/// objects throughout: an array only as the value of a field named in
/// `array_fields`, and never an array inside an array.
fn parse_closed<T: serde::de::DeserializeOwned>(
    bytes: &[u8],
    array_fields: &[&str],
) -> Result<T, ()> {
    fn objects_only(value: &serde_json::Value, array_fields: &[&str]) -> bool {
        match value {
            serde_json::Value::Object(map) => map.iter().all(|(name, field)| match field {
                serde_json::Value::Array(items) => {
                    array_fields.contains(&name.as_str())
                        && items
                            .iter()
                            .all(|item| !item.is_array() && objects_only(item, array_fields))
                }
                other => objects_only(other, array_fields),
            }),
            serde_json::Value::Array(_) => false,
            _ => true,
        }
    }
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| ())?;
    if !value.is_object() || !objects_only(&value, array_fields) {
        return Err(());
    }
    serde_json::from_value(value).map_err(|_| ())
}

// ---------------------------------------------------------------------------
// Nonces
// ---------------------------------------------------------------------------

/// The quote nonce: 32 bytes the daemon sends to `GET /v1/attestation` so
/// the quote it verifies is fresh. Used for nothing else; a response is
/// bound to its send by [`JudgementRequestNonce`].
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
        nonce_from_hex(value, "nonce").map(Self)
    }

    pub fn as_bytes(&self) -> &[u8; NONCE_LEN] {
        &self.0
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }
}

/// The request nonce: 32 bytes the daemon generates for one send, carries
/// inside the sealed request, and requires back in the signed response. A
/// distinct type from [`JudgementNonce`] so the two cannot be swapped.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct JudgementRequestNonce([u8; NONCE_LEN]);

impl std::fmt::Debug for JudgementRequestNonce {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("JudgementRequestNonce")
            .field(&self.to_hex())
            .finish()
    }
}

impl JudgementRequestNonce {
    pub fn from_bytes(bytes: [u8; NONCE_LEN]) -> Self {
        Self(bytes)
    }

    /// Exactly 64 lowercase hex characters.
    pub fn from_hex(value: &str) -> Result<Self, JudgementWireError> {
        nonce_from_hex(value, "request_nonce").map(Self)
    }

    pub fn as_bytes(&self) -> &[u8; NONCE_LEN] {
        &self.0
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }
}

fn nonce_from_hex(value: &str, field: &'static str) -> Result<[u8; NONCE_LEN], JudgementWireError> {
    if !is_lower_hex(value, NONCE_LEN * 2) {
        return Err(JudgementWireError::Malformed(field));
    }
    let mut bytes = [0u8; NONCE_LEN];
    hex::decode_to_slice(value, &mut bytes).map_err(|_| JudgementWireError::Malformed(field))?;
    Ok(bytes)
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

/// The pinned judgements. `stuck` judges a [`TurnDigest`] and is always
/// requested alone; the other three judge a session digest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JudgementKind {
    Task,
    EventKind,
    Finished,
    Stuck,
}

impl JudgementKind {
    pub const fn as_wire(self) -> &'static str {
        match self {
            Self::Task => "task",
            Self::EventKind => "event_kind",
            Self::Finished => "finished",
            Self::Stuck => "stuck",
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
            Self::Stuck => &[
                JudgementLabel::Stuck,
                JudgementLabel::Productive,
                JudgementLabel::Unclear,
            ],
        }
    }

    const fn tag(self) -> u8 {
        match self {
            Self::Task => 1,
            Self::EventKind => 2,
            Self::Finished => 3,
            Self::Stuck => 4,
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
    Stuck,
    Productive,
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
            Self::Stuck => "stuck",
            Self::Productive => "productive",
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
            (JudgementKind::Task | JudgementKind::Finished | JudgementKind::Stuck, None) => {}
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

impl SpendBucket {
    const fn tag(self) -> u8 {
        match self {
            Self::OverHalf => 1,
            Self::UnderHalf => 2,
            Self::UnderTenth => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JudgementSpend {
    pub account_month_remaining_bucket: SpendBucket,
}

// ---------------------------------------------------------------------------
// Attestation
// ---------------------------------------------------------------------------

/// SEC1 compressed secp256k1 public key length.
const SEAL_PUBLIC_KEY_BYTES: usize = 33;
/// EIP-191 address length.
const ADDRESS_BYTES: usize = 20;

/// `200` body of `GET /v1/attestation?nonce=<64 hex>`: the quote, and the
/// two per-boot public values its report data binds. The client recomputes
/// the binding from `seal_public_key` and `signing_address` and requires the
/// quote's report data to match before it seals anything to the key.
///
/// This checks shape only. Verifying the quote is the client's job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JudgementAttestationResponse {
    quote_hex: String,
    seal_public_key: String,
    signing_address: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAttestationResponse {
    quote_hex: String,
    seal_public_key: String,
    signing_address: String,
}

impl JudgementAttestationResponse {
    /// `quote_hex` is non-empty, even-length lowercase hex;
    /// `seal_public_key` is 66 lowercase hex, a SEC1 compressed point
    /// (`02` or `03` prefix); `signing_address` is `0x` and 40 lowercase
    /// hex.
    pub fn new(
        quote_hex: String,
        seal_public_key: String,
        signing_address: String,
    ) -> Result<Self, JudgementWireError> {
        let quote_ok = !quote_hex.is_empty()
            && quote_hex.len().is_multiple_of(2)
            && is_lower_hex(&quote_hex, quote_hex.len());
        if !quote_ok {
            return Err(JudgementWireError::Malformed("quote_hex"));
        }
        let seal_ok = is_lower_hex(&seal_public_key, SEAL_PUBLIC_KEY_BYTES * 2)
            && (seal_public_key.starts_with("02") || seal_public_key.starts_with("03"));
        if !seal_ok {
            return Err(JudgementWireError::Malformed("seal_public_key"));
        }
        let address_ok = signing_address
            .strip_prefix("0x")
            .is_some_and(|hex| is_lower_hex(hex, ADDRESS_BYTES * 2));
        if !address_ok {
            return Err(JudgementWireError::Malformed("signing_address"));
        }
        Ok(Self {
            quote_hex,
            seal_public_key,
            signing_address,
        })
    }

    pub fn quote_hex(&self) -> &str {
        &self.quote_hex
    }

    pub fn quote_bytes(&self) -> Vec<u8> {
        hex::decode(&self.quote_hex).expect("validated lowercase hex")
    }

    pub fn seal_public_key(&self) -> &str {
        &self.seal_public_key
    }

    pub fn seal_public_key_bytes(&self) -> [u8; SEAL_PUBLIC_KEY_BYTES] {
        let mut out = [0u8; SEAL_PUBLIC_KEY_BYTES];
        hex::decode_to_slice(&self.seal_public_key, &mut out).expect("validated lowercase hex");
        out
    }

    pub fn signing_address(&self) -> &str {
        &self.signing_address
    }

    pub fn signing_address_bytes(&self) -> [u8; ADDRESS_BYTES] {
        let mut out = [0u8; ADDRESS_BYTES];
        hex::decode_to_slice(&self.signing_address[2..], &mut out)
            .expect("validated lowercase hex");
        out
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(&RawAttestationResponse {
            quote_hex: self.quote_hex.clone(),
            seal_public_key: self.seal_public_key.clone(),
            signing_address: self.signing_address.clone(),
        })
        .expect("an attestation response is JSON serializable")
    }
}

/// Parse a `GET /v1/attestation` body, closed.
pub fn parse_judgement_attestation(
    body: &[u8],
) -> Result<JudgementAttestationResponse, JudgementWireError> {
    let raw: RawAttestationResponse =
        parse_closed(body, &[]).map_err(|_| JudgementWireError::Malformed("attestation"))?;
    JudgementAttestationResponse::new(raw.quote_hex, raw.seal_public_key, raw.signing_address)
}

// ---------------------------------------------------------------------------
// Request
// ---------------------------------------------------------------------------

/// The plaintext of a `POST /v1/judgement` request, which the daemon seals
/// to the service's per-boot key. `Debug` shows the digest's hash only: the
/// digest is scrubbed contributor text, the claim is a bearer credential
/// naming the account, and the request nonce binds the response to this one
/// send.
#[derive(Clone, PartialEq, Eq)]
pub struct JudgementRequest {
    request_nonce: JudgementRequestNonce,
    claim: String,
    digest: Vec<u8>,
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
    request_nonce: String,
    claim: String,
    digest_b64: String,
    judgements: Vec<JudgementKind>,
}

impl std::fmt::Debug for JudgementRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JudgementRequest")
            .field("digest_sha256", &self.digest_sha256())
            .field("judgements", &self.judgements)
            .finish_non_exhaustive()
    }
}

impl JudgementRequest {
    /// The digest must be non-empty and at most
    /// [`JUDGEMENT_DIGEST_MAX_BYTES`] bytes; the claim non-empty; the
    /// judgements non-empty and distinct, with `stuck` only alone and
    /// exactly when the digest is a valid [`TurnDigest`]; and the
    /// encoded plaintext at most
    /// [`JUDGEMENT_REQUEST_MAX_BYTES`], so a request is never built that the
    /// service would refuse for size. The claim is opaque here; the service
    /// verifies it.
    pub fn new(
        request_nonce: JudgementRequestNonce,
        claim: String,
        digest: Vec<u8>,
        judgements: Vec<JudgementKind>,
    ) -> Result<Self, JudgementRefusal> {
        if digest.len() > JUDGEMENT_DIGEST_MAX_BYTES {
            return Err(JudgementRefusal::DigestTooLarge);
        }
        if digest.is_empty() || claim.is_empty() || judgements.is_empty() {
            return Err(JudgementRefusal::RequestMalformed);
        }
        let distinct: BTreeSet<_> = judgements.iter().collect();
        if distinct.len() != judgements.len() {
            return Err(JudgementRefusal::RequestMalformed);
        }
        // `stuck` judges a turn digest, the others a session digest: one
        // request carries one kind of digest, and the digest must be that
        // kind. A turn digest carries no contributor text, so a session
        // digest sent for `stuck` would send text the contract says this
        // judgement never sees; both ends refuse it.
        let stuck = judgements.contains(&JudgementKind::Stuck);
        if stuck && judgements.len() > 1 {
            return Err(JudgementRefusal::RequestMalformed);
        }
        match (stuck, TurnDigest::parse(&digest)) {
            (true, Ok(_)) | (false, Err(_)) => {}
            // A newer turn digest is a version the service does not speak
            // yet, so the client can step down rather than drop the turn.
            (true, Err(JudgementWireError::WireVersionUnsupported)) => {
                return Err(JudgementRefusal::WireVersionUnsupported);
            }
            (true, Err(_)) | (false, Ok(_)) => return Err(JudgementRefusal::RequestMalformed),
        }
        let request = Self {
            request_nonce,
            claim,
            digest,
            judgements,
        };
        if request.to_json().len() > JUDGEMENT_REQUEST_MAX_BYTES {
            return Err(JudgementRefusal::DigestTooLarge);
        }
        Ok(request)
    }

    pub fn request_nonce(&self) -> &JudgementRequestNonce {
        &self.request_nonce
    }

    pub fn claim(&self) -> &str {
        &self.claim
    }

    /// The decoded digest bytes.
    pub fn digest(&self) -> &[u8] {
        &self.digest
    }

    pub fn judgements(&self) -> &[JudgementKind] {
        &self.judgements
    }

    /// Lowercase hex SHA-256 of the decoded digest bytes: the only thing
    /// about the digest that may be logged.
    pub fn digest_sha256(&self) -> String {
        hex::encode(Sha256::digest(&self.digest))
    }

    /// The plaintext JSON the seal wraps. The digest travels as standard
    /// base64 (`digest_b64`) rather than a JSON string, so an escape-heavy
    /// digest does not nearly double in size.
    pub fn to_json(&self) -> String {
        serde_json::to_string(&RawRequest {
            wire_version: JUDGEMENT_WIRE_VERSION,
            request_nonce: self.request_nonce.to_hex(),
            claim: self.claim.clone(),
            digest_b64: base64_encode(&self.digest),
            judgements: self.judgements.clone(),
        })
        .expect("a judgement request is JSON serializable")
    }
}

/// Parse an opened request plaintext into the refusal the service sends
/// for it.
///
/// Order: the plaintext cap, then the wire version (read leniently, so a
/// later version with new fields is reported as a version, not as
/// malformed), then the closed shape, then the base64 digest and the request
/// nonce, then the decoded digest cap and the rest of
/// [`JudgementRequest::new`].
pub fn parse_judgement_request(plaintext: &[u8]) -> Result<JudgementRequest, JudgementRefusal> {
    if plaintext.len() > JUDGEMENT_REQUEST_MAX_BYTES {
        return Err(JudgementRefusal::DigestTooLarge);
    }
    let version: VersionOnly =
        serde_json::from_slice(plaintext).map_err(|_| JudgementRefusal::RequestMalformed)?;
    if version.wire_version != JUDGEMENT_WIRE_VERSION {
        return Err(JudgementRefusal::WireVersionUnsupported);
    }
    let raw: RawRequest =
        parse_closed(plaintext, &["judgements"]).map_err(|_| JudgementRefusal::RequestMalformed)?;
    let digest = base64_decode(&raw.digest_b64).ok_or(JudgementRefusal::RequestMalformed)?;
    let request_nonce = JudgementRequestNonce::from_hex(&raw.request_nonce)
        .map_err(|_| JudgementRefusal::RequestMalformed)?;
    JudgementRequest::new(request_nonce, raw.claim, digest, raw.judgements)
}

// ---------------------------------------------------------------------------
// Base64
// ---------------------------------------------------------------------------
//
// Standard base64 (RFC 4648 section 4) with padding, inline because this
// permissive crate has no base64 dependency. Decoding is strict: the
// standard alphabet only, padding required, no whitespace, and the bits
// under the padding must be zero, so each byte string has exactly one
// accepted encoding.

const BASE64_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let mut group = [0u8; 3];
        group[..chunk.len()].copy_from_slice(chunk);
        let n = u32::from(group[0]) << 16 | u32::from(group[1]) << 8 | u32::from(group[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(
                    BASE64_ALPHABET[(n >> (18 - 6 * i)) as usize & 63],
                ));
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn base64_value(c: u8) -> Option<u32> {
    let value = match c {
        b'A'..=b'Z' => c - b'A',
        b'a'..=b'z' => c - b'a' + 26,
        b'0'..=b'9' => c - b'0' + 52,
        b'+' => 62,
        b'/' => 63,
        _ => return None,
    };
    Some(u32::from(value))
}

fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    if !bytes.len().is_multiple_of(4) {
        return None;
    }
    let quads = bytes.len() / 4;
    let mut out = Vec::with_capacity(quads * 3);
    for (i, quad) in bytes.as_chunks::<4>().0.iter().enumerate() {
        let pad = quad.iter().rev().take_while(|&&c| c == b'=').count();
        if pad > 2 || (pad > 0 && i + 1 != quads) {
            return None;
        }
        let mut n = 0u32;
        for &c in &quad[..4 - pad] {
            n = n << 6 | base64_value(c)?;
        }
        n <<= 6 * pad as u32;
        let group = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
        let keep = 3 - pad;
        if group[keep..].iter().any(|&b| b != 0) {
            return None;
        }
        out.extend_from_slice(&group[..keep]);
    }
    Some(out)
}

// ---------------------------------------------------------------------------
// Response
// ---------------------------------------------------------------------------

/// The signed plaintext of a `200` from `POST /v1/judgement`, which the
/// service seals under the request's response key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JudgementResponse {
    wire_version: u32,
    request_nonce: JudgementRequestNonce,
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
    request_nonce: String,
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
    /// This checks shape only. Recovering the signer is the client's job,
    /// and [`JudgementResponse::check_binding`] ties the response to the
    /// send it answers.
    pub fn new(
        request_nonce: JudgementRequestNonce,
        digest_sha256: String,
        results: Vec<JudgementResult>,
        spend: JudgementSpend,
        signed_at: i64,
        signature: String,
    ) -> Result<Self, JudgementWireError> {
        if !is_lower_hex(&digest_sha256, SHA256_HEX_LEN) {
            return Err(JudgementWireError::Malformed("digest_sha256"));
        }
        // Empty is allowed: an `event_kind`-only request on a digest with no
        // marks is answered with no results. `check_binding` refuses an empty
        // response to any request that needed one.
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
            wire_version: JUDGEMENT_WIRE_VERSION,
            request_nonce,
            digest_sha256,
            results,
            spend,
            signed_at,
            signature,
        })
    }

    pub fn wire_version(&self) -> u32 {
        self.wire_version
    }

    /// The request nonce the service echoed. Signed.
    pub fn request_nonce(&self) -> &JudgementRequestNonce {
        &self.request_nonce
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

    /// The bytes `signature` is over: every field but the signature.
    pub fn signing_bytes(&self) -> Vec<u8> {
        judgement_signing_bytes(
            self.wire_version,
            &self.request_nonce,
            &self.digest_sha256,
            &self.results,
            self.spend,
            self.signed_at,
        )
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(&RawResponse {
            wire_version: self.wire_version,
            request_nonce: self.request_nonce.to_hex(),
            digest_sha256: self.digest_sha256.clone(),
            results: self.results.clone(),
            spend: self.spend,
            signed_at: self.signed_at,
            signature: self.signature.clone(),
        })
        .expect("a judgement response is JSON serializable")
    }
}

/// Why a well-formed response does not answer the send it came back for.
/// Carries no input bytes. Any of these discards every label in the
/// response.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum JudgementBindingError {
    #[error("judgement response wire version is not 1")]
    WireVersionUnsupported,
    #[error("judgement response request nonce does not match this send")]
    RequestNonceMismatch,
    #[error("judgement response digest hash does not match the digest sent")]
    DigestMismatch,
    #[error("judgement response is missing a requested judgement")]
    JudgementMissing,
    #[error("judgement response carries a judgement that was not requested")]
    JudgementUnrequested,
    #[error("judgement response names an event mark the digest does not have")]
    EventIndexOutOfRange,
    #[error("judgement response is missing an event mark the digest has")]
    EventMarkMissing,
}

impl JudgementResponse {
    /// Whether this response answers `sent`, whose digest carries
    /// `event_marker_count` event markers. Pure, and shared by the daemon
    /// client and the ingest drill. It does not check the signature: run it
    /// on a response whose signer was already recovered and compared.
    ///
    /// Requires, in order: `wire_version == 1`; the signed request nonce and
    /// digest hash equal the ones sent; `task` and `finished` appear exactly
    /// once if requested and not at all otherwise, as does `stuck`; and, if
    /// `event_kind` was requested, exactly one result for every mark
    /// `0..event_marker_count`,
    /// no other index, and none if it was not requested. The result
    /// constructors already hold that only the first
    /// [`JUDGEMENT_EVENT_KIND_MAX`] marks are judged and that no
    /// `(judgement, event_index)` repeats. A digest with no marks has no
    /// `event_kind` results even when `event_kind` was requested.
    pub fn check_binding(
        &self,
        sent: &JudgementRequest,
        event_marker_count: u32,
    ) -> Result<(), JudgementBindingError> {
        if self.wire_version != JUDGEMENT_WIRE_VERSION {
            return Err(JudgementBindingError::WireVersionUnsupported);
        }
        if self.request_nonce != *sent.request_nonce() {
            return Err(JudgementBindingError::RequestNonceMismatch);
        }
        if self.digest_sha256 != sent.digest_sha256() {
            return Err(JudgementBindingError::DigestMismatch);
        }
        let requested = |kind| sent.judgements().contains(&kind);
        let mut marks = 0u32;
        for result in &self.results {
            if !requested(result.judgement) {
                return Err(JudgementBindingError::JudgementUnrequested);
            }
            if let Some(index) = result.event_index {
                if index >= event_marker_count {
                    return Err(JudgementBindingError::EventIndexOutOfRange);
                }
                marks += 1;
            }
        }
        for kind in [
            JudgementKind::Task,
            JudgementKind::Finished,
            JudgementKind::Stuck,
        ] {
            if requested(kind) && !self.results.iter().any(|r| r.judgement == kind) {
                return Err(JudgementBindingError::JudgementMissing);
            }
        }
        // Indices are distinct (the constructor refuses a repeat) and each is
        // below the count, so equal counts mean every mark is present.
        if requested(JudgementKind::EventKind) && marks != event_marker_count {
            return Err(JudgementBindingError::EventMarkMissing);
        }
        Ok(())
    }
}

/// Parse a `200` body, closed: unknown fields, unknown labels and
/// inconsistent results are refused.
///
/// The wire version is read leniently first, as [`parse_judgement_request`]
/// does, so a later version that also adds fields is reported as
/// [`JudgementWireError::WireVersionUnsupported`], not as malformed.
pub fn parse_judgement_response(body: &[u8]) -> Result<JudgementResponse, JudgementWireError> {
    let version: VersionOnly =
        serde_json::from_slice(body).map_err(|_| JudgementWireError::Malformed("response"))?;
    if version.wire_version != JUDGEMENT_WIRE_VERSION {
        return Err(JudgementWireError::WireVersionUnsupported);
    }
    let raw: RawResponse =
        parse_closed(body, &["results"]).map_err(|_| JudgementWireError::Malformed("response"))?;
    let request_nonce = JudgementRequestNonce::from_hex(&raw.request_nonce)?;
    JudgementResponse::new(
        request_nonce,
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
/// - [`JUDGEMENT_SIGNING_DOMAIN`];
/// - `wire_version`, u32 LE;
/// - the request nonce, 32 raw bytes;
/// - `digest_sha256` as length-prefixed ASCII;
/// - the result count, u64 LE, then per result:
///   - judgement tag (1 task, 2 event_kind, 3 finished, 4 stuck);
///   - event index: 0 absent, or 1 then u32 LE;
///   - label as length-prefixed wire string;
///   - 0 unjudged, or 1 then: bucket tag (1 high, 2 medium, 3 low), basis
///     tag (1 probability, 2 self_reported), length-prefixed
///     `prompt_version`, length-prefixed `model_id`, then receipt: 0 absent,
///     or 1, kind tag (1 provider_tee), length-prefixed model,
///     length-prefixed signer;
/// - the spend bucket tag (1 over_half, 2 under_half, 3 under_tenth);
/// - `signed_at`, i64 LE.
///
/// Every length prefix is u64 LE. Every response field but the signature is
/// signed (#1501 review point 12): an unsigned `wire_version` or `spend`
/// could be rewritten on the path, and the request nonce binds the response
/// to one send where the quote nonce did not.
pub fn judgement_signing_bytes(
    wire_version: u32,
    request_nonce: &JudgementRequestNonce,
    digest_sha256: &str,
    results: &[JudgementResult],
    spend: JudgementSpend,
    signed_at: i64,
) -> Vec<u8> {
    let mut out = JUDGEMENT_SIGNING_DOMAIN.to_vec();
    out.extend_from_slice(&wire_version.to_le_bytes());
    out.extend_from_slice(request_nonce.as_bytes());
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
    out.push(spend.account_month_remaining_bucket.tag());
    out.extend_from_slice(&signed_at.to_le_bytes());
    out
}

// ---------------------------------------------------------------------------
// The judgement claim, minted by the upload-claim issuer
// ---------------------------------------------------------------------------

/// Issuer route that mints a judgement claim (spec section 3, "The account
/// credential").
///
/// The request is authenticated exactly as a device-key upload claim is: the
/// device signs the exact JSON body bytes with raw Ed25519 and sends
/// `x-trace-device-key-id` and `x-trace-device-signature` beside it. The
/// issuer mints only for a device registered, and not revoked, under a
/// per-account tenant; any other tenant is refused with
/// `judgement_account_ineligible`. `issued_at` bounds replay, and the schema
/// version keeps an upload-claim body from being replayed here or the
/// reverse.
pub const JUDGEMENT_CLAIM_PATH: &str = "/v1/insights-judgement/claim";

pub const JUDGEMENT_CLAIM_REQUEST_SCHEMA_VERSION: &str =
    "trace_commons.insights_judgement_claim_request.v1";

/// How far `issued_at` may sit from the issuer's clock, either way.
pub const JUDGEMENT_CLAIM_MAX_SKEW_SECONDS: i64 = 300;

/// Issuer refusal: `issued_at` is outside [`JUDGEMENT_CLAIM_MAX_SKEW_SECONDS`].
/// The label of [`JudgementClaimRefusal::Stale`]; every refusal on this route
/// is a member of [`JudgementClaimRefusal`].
pub const JUDGEMENT_CLAIM_REQUEST_STALE: &str = "insights_judgement_claim_request_stale";

/// The `Retry-After` the issuer sends with a
/// [`JudgementClaimRetry::AfterDelay`] refusal, in seconds.
pub const JUDGEMENT_CLAIM_RETRY_AFTER_SECS: u64 = 30;

/// What a daemon may do after a [`JudgementClaimRefusal`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JudgementClaimRetry {
    /// Do not retry. The device, its account or the request is refused, and
    /// asking again changes nothing.
    Never,
    /// Retry once, with a fresh `issued_at`. If that is also stale, the
    /// device's clock is wrong, so do not loop.
    WithFreshTimestamp,
    /// The issuer could not answer. Retry after
    /// [`JUDGEMENT_CLAIM_RETRY_AFTER_SECS`], which the issuer also sends as
    /// `Retry-After`.
    AfterDelay,
}

/// The refusals `POST /v1/insights-judgement/claim` returns, as a closed set.
///
/// A refusal reaches the daemon as a status and a `{"error": "<label>"}`
/// body, and the daemon's re-mint logic matches the label against this set,
/// never prose. These labels are this route's own. The upload-claim route
/// refuses the same conditions with different, space-separated text
/// (`device key not registered`), which is not a member.
///
/// Two answers come from the issuer's shared router layers, not from this
/// route, and are not members: `413` for a body over the issuer's request
/// cap, and `408` when the request outlives the issuer's request timeout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JudgementClaimRefusal {
    /// The body is not a closed v1 claim request, or the device-auth headers
    /// are missing or malformed.
    Malformed,
    /// The `x-trace-device-key-id` header and the body's `device_key_id`
    /// differ.
    DeviceKeyMismatch,
    /// `issued_at` is outside [`JUDGEMENT_CLAIM_MAX_SKEW_SECONDS`] of the
    /// issuer's clock.
    Stale,
    /// The signature is not the registered device key's over these bytes.
    SignatureInvalid,
    /// No device key with this id is registered under the named tenant.
    DeviceKeyNotRegistered,
    /// The device key is revoked.
    DeviceKeyRevoked,
    /// The tenant is not a per-account tenant (v1 refuses pooled and legacy
    /// invite tenants). The same label as the service's
    /// [`JudgementRefusal::AccountIneligible`].
    AccountIneligible,
    /// The issuer requires tenant access grants, and this device holds no
    /// active contributor grant, so it could not mint an upload claim
    /// either.
    GrantRequired,
    /// The issuer has no device-key registry configured.
    RegistryUnavailable,
    /// The issuer failed to mint, for a reason that is its own.
    IssueFailed,
}

impl JudgementClaimRefusal {
    /// Every refusal, for tests and exhaustive mappings.
    pub const ALL: [JudgementClaimRefusal; 10] = [
        JudgementClaimRefusal::Malformed,
        JudgementClaimRefusal::DeviceKeyMismatch,
        JudgementClaimRefusal::Stale,
        JudgementClaimRefusal::SignatureInvalid,
        JudgementClaimRefusal::DeviceKeyNotRegistered,
        JudgementClaimRefusal::DeviceKeyRevoked,
        JudgementClaimRefusal::AccountIneligible,
        JudgementClaimRefusal::GrantRequired,
        JudgementClaimRefusal::RegistryUnavailable,
        JudgementClaimRefusal::IssueFailed,
    ];

    /// The wire label, which is what the issuer puts in the `error` field.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Malformed => "judgement_claim_malformed",
            Self::DeviceKeyMismatch => "judgement_claim_device_key_mismatch",
            Self::Stale => JUDGEMENT_CLAIM_REQUEST_STALE,
            Self::SignatureInvalid => "judgement_claim_signature_invalid",
            Self::DeviceKeyNotRegistered => "device_key_not_registered",
            Self::DeviceKeyRevoked => "device_key_revoked",
            Self::AccountIneligible => JudgementRefusal::AccountIneligible.label(),
            Self::GrantRequired => "judgement_claim_grant_required",
            Self::RegistryUnavailable => "judgement_claim_registry_unavailable",
            Self::IssueFailed => "judgement_claim_issue_failed",
        }
    }

    /// The status this refusal is sent with.
    pub const fn status(self) -> u16 {
        match self {
            Self::Malformed | Self::DeviceKeyMismatch | Self::Stale => 400,
            Self::SignatureInvalid
            | Self::DeviceKeyNotRegistered
            | Self::DeviceKeyRevoked
            | Self::AccountIneligible
            | Self::GrantRequired => 403,
            Self::RegistryUnavailable => 503,
            Self::IssueFailed => 500,
        }
    }

    /// What the daemon may do next.
    pub const fn retry(self) -> JudgementClaimRetry {
        match self {
            Self::Stale => JudgementClaimRetry::WithFreshTimestamp,
            Self::RegistryUnavailable | Self::IssueFailed => JudgementClaimRetry::AfterDelay,
            Self::Malformed
            | Self::DeviceKeyMismatch
            | Self::SignatureInvalid
            | Self::DeviceKeyNotRegistered
            | Self::DeviceKeyRevoked
            | Self::AccountIneligible
            | Self::GrantRequired => JudgementClaimRetry::Never,
        }
    }

    /// Recognise a refusal from its label. `None` for anything else,
    /// including the upload-claim route's prose.
    pub fn from_label(label: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.label() == label)
    }
}

/// `POST /v1/insights-judgement/claim` body. Signed as sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JudgementClaimRequest {
    pub schema_version: String,
    /// The tenant the device is registered under. Becomes the claim's
    /// `account_ref` when it is a per-account tenant.
    pub tenant_id: String,
    /// Must equal the `x-trace-device-key-id` header.
    pub device_key_id: String,
    /// Unix seconds.
    pub issued_at: i64,
}

/// The minted judgement claim. The token is an EdDSA JWT with
/// `aud = "trace-commons-insights-judgement"` and an `account_ref` claim. The
/// daemon puts it in the sealed request plaintext's `claim` field (see
/// [`JudgementRequest`]), never in a header, and never reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JudgementClaimResponse {
    pub access_token: String,
    /// Always `Bearer`.
    pub token_type: String,
    /// Unix seconds.
    pub expires_at: i64,
    /// Seconds from issue to `expires_at`, at most 15 minutes.
    pub expires_in: i64,
}

// ---------------------------------------------------------------------------
// Turn digest
// ---------------------------------------------------------------------------

/// The only turn digest version this crate speaks.
pub const TURN_DIGEST_VERSION: u32 = 1;
/// The most recent calls a [`TurnDigest`] carries.
pub const TURN_DIGEST_RECENT_MAX: usize = 24;

/// What one call in a turn did, as the economizer classifies it. Never the
/// tool's name or arguments.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnCallKind {
    /// Changed a file.
    Edit,
    /// Ran a test, build, lint or type check.
    Check,
    /// Ran any other command.
    Command,
    /// Read or searched files.
    Read,
    Other,
}

/// One recent call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TurnCall {
    pub kind: TurnCallKind,
    pub failed: bool,
    /// The failure matches an earlier failure of the same turn.
    pub repeated: bool,
}

/// What the `stuck` judgement sees of one running turn: counts and the
/// shape of its recent calls. It carries no contributor text at all: no
/// prompts, paths, commands, file contents or output. Parsed closed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TurnDigest {
    pub digest_version: u32,
    /// Tool calls so far in the turn.
    pub calls: u32,
    pub edits: u32,
    pub files_edited: u32,
    pub most_edits_to_one_file: u32,
    pub failures: u32,
    pub repeated_failures: u32,
    pub checks: u32,
    /// The latest check since the last edit failed.
    pub last_check_failed: bool,
    /// How many calls this folder's turns that ended well usually take (the
    /// 90th percentile), when the economizer has learned it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usual_calls: Option<u32>,
    /// The turn's last calls, oldest first, at most
    /// [`TURN_DIGEST_RECENT_MAX`].
    pub recent: Vec<TurnCall>,
}

impl TurnDigest {
    /// Whether the counts hang together: no count above the calls; the
    /// per-file and repeat counts within their totals; a failed last check
    /// only after a check; the first failure never a repeat; the busiest
    /// file holding at least its share of the edits; a learned usual count
    /// above zero; and at most [`TURN_DIGEST_RECENT_MAX`] recent calls, no
    /// more than the turn made, whose per-kind, failed and repeated counts
    /// fit within the totals.
    fn check(&self) -> Result<(), JudgementWireError> {
        let within = |part: u32, whole: u32| part <= whole;
        let recent = |keep: fn(&TurnCall) -> bool| {
            u32::try_from(self.recent.iter().filter(|call| keep(call)).count()).unwrap_or(u32::MAX)
        };
        if self.digest_version != TURN_DIGEST_VERSION {
            return Err(JudgementWireError::WireVersionUnsupported);
        }
        if !(within(self.edits, self.calls)
            && within(self.failures, self.calls)
            && within(self.checks, self.calls)
            && within(self.repeated_failures, self.failures)
            && within(self.most_edits_to_one_file, self.edits)
            && within(self.files_edited, self.edits)
            && (self.edits == 0) == (self.files_edited == 0)
            && (self.files_edited == 0
                || self.most_edits_to_one_file >= self.edits.div_ceil(self.files_edited))
            && (self.failures == 0 || self.repeated_failures < self.failures)
            && (self.checks > 0 || !self.last_check_failed)
            && self.usual_calls != Some(0)
            && within(recent(|call| call.kind == TurnCallKind::Edit), self.edits)
            && within(recent(|call| call.kind == TurnCallKind::Check), self.checks)
            && within(recent(|call| call.failed), self.failures)
            && within(recent(|call| call.repeated), self.repeated_failures)
            && self.recent.len() <= TURN_DIGEST_RECENT_MAX
            && self.recent.len() as u64 <= u64::from(self.calls)
            && self.recent.iter().all(|call| call.failed || !call.repeated))
        {
            return Err(JudgementWireError::Malformed("turn_digest"));
        }
        Ok(())
    }

    /// The digest's bytes, as sent. Refuses an inconsistent digest.
    pub fn to_bytes(&self) -> Result<Vec<u8>, JudgementWireError> {
        self.check()?;
        serde_json::to_vec(self).map_err(|_| JudgementWireError::Malformed("turn_digest"))
    }

    /// Parse a turn digest, closed. The version is read leniently first, so
    /// a later version is reported as unsupported, not malformed.
    pub fn parse(bytes: &[u8]) -> Result<Self, JudgementWireError> {
        #[derive(Deserialize)]
        struct Version {
            digest_version: u32,
        }
        let version: Version = serde_json::from_slice(bytes)
            .map_err(|_| JudgementWireError::Malformed("turn_digest"))?;
        if version.digest_version != TURN_DIGEST_VERSION {
            return Err(JudgementWireError::WireVersionUnsupported);
        }
        let digest: Self = parse_closed(bytes, &["recent"])
            .map_err(|_| JudgementWireError::Malformed("turn_digest"))?;
        digest.check()?;
        Ok(digest)
    }
}

#[cfg(test)]
mod turn_digest_tests {
    use super::*;

    fn digest() -> TurnDigest {
        TurnDigest {
            digest_version: TURN_DIGEST_VERSION,
            calls: 22,
            edits: 9,
            files_edited: 2,
            most_edits_to_one_file: 7,
            failures: 8,
            repeated_failures: 5,
            checks: 6,
            last_check_failed: true,
            usual_calls: Some(14),
            recent: vec![
                TurnCall {
                    kind: TurnCallKind::Edit,
                    failed: false,
                    repeated: false,
                },
                TurnCall {
                    kind: TurnCallKind::Check,
                    failed: true,
                    repeated: true,
                },
            ],
        }
    }

    #[test]
    fn a_turn_digest_round_trips_and_stays_small() {
        let mut full = digest();
        full.calls = 40;
        full.checks = 24;
        full.failures = 25;
        full.repeated_failures = 24;
        full.recent = vec![full.recent[1]; TURN_DIGEST_RECENT_MAX];
        let bytes = full.to_bytes().unwrap();
        assert_eq!(TurnDigest::parse(&bytes).unwrap(), full);
        assert!(
            bytes.len() < JUDGEMENT_DIGEST_MAX_BYTES / 4,
            "{}",
            bytes.len()
        );
    }

    #[test]
    fn a_turn_digest_is_closed_and_consistent() {
        let bytes = digest().to_bytes().unwrap();
        let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        value["prompt"] = serde_json::json!("fix the bug");
        assert_eq!(
            TurnDigest::parse(&serde_json::to_vec(&value).unwrap()),
            Err(JudgementWireError::Malformed("turn_digest"))
        );
        let mut later = serde_json::from_slice::<serde_json::Value>(&bytes).unwrap();
        later["digest_version"] = serde_json::json!(2);
        later["new_field"] = serde_json::json!(1);
        assert_eq!(
            TurnDigest::parse(&serde_json::to_vec(&later).unwrap()),
            Err(JudgementWireError::WireVersionUnsupported)
        );
        for broken in [
            TurnDigest {
                failures: 30,
                ..digest()
            },
            TurnDigest {
                repeated_failures: 9,
                ..digest()
            },
            TurnDigest {
                most_edits_to_one_file: 10,
                ..digest()
            },
            TurnDigest {
                files_edited: 0,
                ..digest()
            },
            TurnDigest {
                calls: 1,
                edits: 1,
                files_edited: 1,
                most_edits_to_one_file: 1,
                failures: 1,
                repeated_failures: 1,
                checks: 1,
                ..digest()
            },
            TurnDigest {
                recent: vec![TurnCall {
                    kind: TurnCallKind::Read,
                    failed: false,
                    repeated: true,
                }],
                ..digest()
            },
            TurnDigest {
                recent: vec![digest().recent[0]; TURN_DIGEST_RECENT_MAX + 1],
                calls: 40,
                ..digest()
            },
            // A failed last check with no check run.
            TurnDigest {
                checks: 0,
                recent: vec![digest().recent[0]],
                ..digest()
            },
            // The first failure cannot repeat an earlier one.
            TurnDigest {
                failures: 1,
                repeated_failures: 1,
                recent: vec![],
                ..digest()
            },
            // Two edits over one file put both in that file.
            TurnDigest {
                edits: 2,
                files_edited: 1,
                most_edits_to_one_file: 1,
                ..digest()
            },
            // An edit puts at least one edit in some file.
            TurnDigest {
                most_edits_to_one_file: 0,
                ..digest()
            },
            // More failed checks among the recent calls than in the turn.
            TurnDigest {
                failures: 1,
                repeated_failures: 0,
                checks: 0,
                last_check_failed: false,
                recent: vec![
                    TurnCall {
                        kind: TurnCallKind::Check,
                        failed: true,
                        repeated: false,
                    };
                    3
                ],
                ..digest()
            },
            // More recent edits than edits.
            TurnDigest {
                edits: 1,
                files_edited: 1,
                most_edits_to_one_file: 1,
                recent: vec![digest().recent[0]; 2],
                ..digest()
            },
            // More recent repeats than repeated failures.
            TurnDigest {
                repeated_failures: 1,
                recent: vec![digest().recent[1]; 2],
                ..digest()
            },
            // A learned usual count is never zero.
            TurnDigest {
                usual_calls: Some(0),
                ..digest()
            },
        ] {
            assert_eq!(
                broken.to_bytes(),
                Err(JudgementWireError::Malformed("turn_digest")),
                "{broken:?}"
            );
        }
    }

    #[test]
    fn stuck_is_requested_alone_and_answered_once() {
        let nonce = JudgementRequestNonce::from_bytes([7; 32]);
        let bytes = digest().to_bytes().unwrap();
        assert!(
            JudgementRequest::new(
                nonce,
                "claim".into(),
                bytes.clone(),
                vec![JudgementKind::Stuck]
            )
            .is_ok()
        );
        assert_eq!(
            JudgementRequest::new(
                nonce,
                "claim".into(),
                bytes,
                vec![JudgementKind::Stuck, JudgementKind::Task]
            )
            .err(),
            Some(JudgementRefusal::RequestMalformed)
        );
        assert_eq!(JudgementKind::Stuck.as_wire(), "stuck");
        let wire: Vec<_> = JudgementKind::Stuck
            .choices()
            .iter()
            .map(|label| label.as_wire())
            .collect();
        assert_eq!(wire, ["stuck", "productive", "unclear"]);
    }

    /// `stuck` takes a turn digest and nothing else, and a turn digest goes
    /// to `stuck` and nowhere else, on both ends of the wire.
    #[test]
    fn stuck_and_the_turn_digest_go_together() {
        let nonce = JudgementRequestNonce::from_bytes([7; 32]);
        let turn = digest().to_bytes().unwrap();
        let session = br#"{"prompt":"fix the bug in src/main.rs"}"#.to_vec();
        let mut inconsistent: serde_json::Value = serde_json::from_slice(&turn).unwrap();
        inconsistent["usual_calls"] = serde_json::json!(0);
        let inconsistent = serde_json::to_vec(&inconsistent).unwrap();
        let wire = |digest: &[u8], judgements: &[&str]| {
            serde_json::json!({
                "wire_version": JUDGEMENT_WIRE_VERSION,
                "request_nonce": nonce.to_hex(),
                "claim": "claim",
                "digest_b64": base64_encode(digest),
                "judgements": judgements,
            })
            .to_string()
        };
        for (digest, kind, label) in [
            (&session, JudgementKind::Stuck, "stuck"),
            (&inconsistent, JudgementKind::Stuck, "stuck"),
            (&turn, JudgementKind::Task, "task"),
            (&turn, JudgementKind::EventKind, "event_kind"),
            (&turn, JudgementKind::Finished, "finished"),
        ] {
            assert_eq!(
                JudgementRequest::new(nonce, "claim".into(), digest.clone(), vec![kind]).err(),
                Some(JudgementRefusal::RequestMalformed),
                "{label}"
            );
            assert_eq!(
                parse_judgement_request(wire(digest, &[label]).as_bytes()).err(),
                Some(JudgementRefusal::RequestMalformed),
                "{label}"
            );
        }
        assert!(parse_judgement_request(wire(&turn, &["stuck"]).as_bytes()).is_ok());
        // A newer turn digest is refused as a version, not as malformed.
        let mut newer: serde_json::Value = serde_json::from_slice(&turn).unwrap();
        newer["digest_version"] = serde_json::json!(TURN_DIGEST_VERSION + 1);
        newer["new_field"] = serde_json::json!(1);
        let newer = serde_json::to_vec(&newer).unwrap();
        assert_eq!(
            JudgementRequest::new(
                nonce,
                "claim".into(),
                newer.clone(),
                vec![JudgementKind::Stuck]
            )
            .err(),
            Some(JudgementRefusal::WireVersionUnsupported)
        );
        assert_eq!(
            parse_judgement_request(wire(&newer, &["stuck"]).as_bytes()).err(),
            Some(JudgementRefusal::WireVersionUnsupported)
        );
        assert!(parse_judgement_request(wire(&session, &["task"]).as_bytes()).is_ok());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FROZEN_VECTOR: &str = concat!(
        // domain "trace_commons.insights_judgement.v1\n"
        "74726163655f636f6d6d6f6e732e696e7369676874735f6a756467656d656e742e76310a",
        // wire_version 1, u32 LE
        "01000000",
        // request_nonce 00..1f, 32 raw bytes
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
        // spend bucket over_half
        "01",
        // signed_at 1792540800, i64 LE
        "8000d86a00000000",
    );

    /// The request nonce the frozen vector signs: bytes 00..1f.
    fn nonce() -> JudgementRequestNonce {
        let bytes: [u8; 32] = std::array::from_fn(|i| i as u8);
        JudgementRequestNonce::from_bytes(bytes)
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
            nonce(),
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
        assert_eq!(JUDGEMENT_SEALED_REQUEST_MAX_BYTES, 16_434);
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
                ("judgement_seal_stale", 400),
                ("judgement_request_replayed", 409),
                ("judgement_quote_unavailable", 503),
                ("judgement_service_fault", 500),
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
        // The attestation route's 503 is not judgement capacity, and the
        // other new refusals are not capacity on any status.
        assert!(!is_judgement_capacity(503, "judgement_quote_unavailable"));
        assert!(!is_judgement_capacity(500, "judgement_service_fault"));
        assert!(!is_judgement_capacity(503, "judgement_service_fault"));
        assert!(!is_judgement_capacity(400, "judgement_seal_stale"));
        assert!(!is_judgement_capacity(409, "judgement_request_replayed"));
        assert_eq!(JudgementRefusal::ALL.len(), 14);
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
            for route in [JudgementRoute::Attestation, JudgementRoute::Judgement] {
                assert_eq!(
                    JudgementRefusal::from_response(route, refusal.status(), refusal.label()),
                    refusal.sent_on(route).then_some(refusal),
                    "{refusal:?} {route:?}"
                );
            }
            assert_eq!(JudgementRefusal::from_label(refusal.label()), Some(refusal));
        }
        assert_eq!(
            JudgementRefusal::from_response(JudgementRoute::Judgement, 400, "judgement_saturated"),
            None
        );
        assert_eq!(
            JudgementRefusal::from_response(JudgementRoute::Judgement, 503, "unknown"),
            None
        );
        assert_eq!(JudgementRefusal::from_label("witness_saturated"), None);
    }

    /// Literal labels, so the table is not derived from the code it checks.
    #[test]
    fn each_route_recognises_only_its_own_refusals() {
        let on = |route| -> Vec<&str> {
            JudgementRefusal::ALL
                .into_iter()
                .filter(|r| r.sent_on(route))
                .map(JudgementRefusal::label)
                .collect()
        };
        assert_eq!(
            on(JudgementRoute::Attestation),
            ["judgement_request_malformed", "judgement_quote_unavailable"]
        );
        let judgement = on(JudgementRoute::Judgement);
        assert_eq!(judgement.len(), 13);
        assert!(!judgement.contains(&"judgement_quote_unavailable"));
        let body = br#"{"error":"judgement_quote_unavailable"}"#;
        assert_eq!(
            parse_judgement_refusal(JudgementRoute::Judgement, 503, body),
            None
        );
        assert!(parse_judgement_refusal(JudgementRoute::Attestation, 503, body).is_some());
        let body = br#"{"error":"judgement_request_replayed"}"#;
        assert_eq!(
            parse_judgement_refusal(JudgementRoute::Attestation, 409, body),
            None
        );
        assert!(parse_judgement_refusal(JudgementRoute::Judgement, 409, body).is_some());
    }

    /// An unsigned refusal defers for long only where a forgery gains
    /// nothing. Literal labels, so the table is not derived from the code.
    #[test]
    fn unsigned_refusals_retry_soon_but_for_three() {
        let table: Vec<_> = JudgementRefusal::ALL
            .into_iter()
            .map(|r| (r.label(), r.unsigned_retry()))
            .collect();
        use JudgementRetry::*;
        assert_eq!(
            table,
            [
                ("judgement_saturated", AfterRetryAfter),
                ("judgement_account_budget_exhausted", Soon),
                ("judgement_operator_budget_exhausted", Soon),
                ("judgement_request_malformed", Soon),
                ("judgement_digest_too_large", DropDigest),
                ("judgement_wire_version_unsupported", Soon),
                ("judgement_credential_refused", Soon),
                ("judgement_account_ineligible", Soon),
                ("judgement_model_unattested", Soon),
                ("judgement_timeout", Soon),
                ("judgement_seal_stale", FreshQuote),
                ("judgement_request_replayed", Soon),
                ("judgement_quote_unavailable", AfterRetryAfter),
                ("judgement_service_fault", Soon),
            ]
        );
    }

    /// `reset_at` is acted on only as the next month start by the client's
    /// clock, give or take the skew; anything else is clamped to it.
    #[test]
    fn reset_at_is_checked_against_the_clients_clock() {
        let now = 1_792_497_600; // 2026-10-20T12:00:00Z
        let next = 1_793_491_200; // 2026-11-01T00:00:00Z
        let at = |reset_at: i64, now: i64| {
            JudgementRefusalBody::new(JudgementRefusal::AccountBudgetExhausted, Some(reset_at))
                .unwrap()
                .retry_not_before(now)
        };
        assert_eq!(at(next, now), Some(next));
        // 1970-01-01, 2099-01-01, and a month start one month too late.
        for forged in [0, 4_070_908_800, 1_796_083_200] {
            assert_eq!(at(forged, now), Some(next), "{forged}");
        }
        // On Dec 31 the next month start is Jan 1, and a service clock a few
        // hours ahead that already reads January is believed.
        let dec_31 = 1_798_740_000; // 2026-12-31T18:00:00Z
        assert_eq!(at(1_798_761_600, dec_31), Some(1_798_761_600));
        assert_eq!(at(1_801_440_000, dec_31), Some(1_801_440_000));
        assert_eq!(at(1_803_859_200, dec_31), Some(1_798_761_600));
        assert_eq!(
            JudgementRefusalBody::new(JudgementRefusal::Timeout, None)
                .unwrap()
                .retry_not_before(now),
            None
        );
    }

    /// serde's derived structs also read a JSON array of their fields; no
    /// parse here does.
    #[test]
    fn no_wire_struct_parses_from_an_array() {
        assert_eq!(
            parse_judgement_refusal(
                JudgementRoute::Judgement,
                503,
                br#"["judgement_saturated"]"#
            ),
            None
        );
        let mut value: serde_json::Value = serde_json::from_str(&response().to_json()).unwrap();
        value["spend"] = serde_json::json!(["over_half"]);
        assert_eq!(
            parse_judgement_response(value.to_string().as_bytes()).err(),
            Some(JudgementWireError::Malformed("response"))
        );
        let mut value: serde_json::Value = serde_json::from_str(&response().to_json()).unwrap();
        let first = value["results"][0].clone();
        let as_array: Vec<_> = first.as_object().unwrap().values().cloned().collect();
        value["results"][0] = serde_json::Value::Array(as_array);
        assert_eq!(
            parse_judgement_response(value.to_string().as_bytes()).err(),
            Some(JudgementWireError::Malformed("response"))
        );
        let attestation = JudgementAttestationResponse::new(
            "00".repeat(8),
            format!("02{}", "11".repeat(32)),
            format!("0x{}", "22".repeat(20)),
        )
        .unwrap();
        let value: serde_json::Value = serde_json::from_str(&attestation.to_json()).unwrap();
        let fields: Vec<_> = value.as_object().unwrap().values().cloned().collect();
        assert!(parse_judgement_attestation(attestation.to_json().as_bytes()).is_ok());
        assert_eq!(
            parse_judgement_attestation(serde_json::Value::Array(fields).to_string().as_bytes())
                .err(),
            Some(JudgementWireError::Malformed("attestation"))
        );
        let request = JudgementRequest::new(
            nonce(),
            "claim".into(),
            b"d".to_vec(),
            vec![JudgementKind::Task],
        )
        .unwrap();
        let value: serde_json::Value = serde_json::from_str(&request.to_json()).unwrap();
        let fields: Vec<_> = value.as_object().unwrap().values().cloned().collect();
        assert!(parse_judgement_request(request.to_json().as_bytes()).is_ok());
        assert_eq!(
            parse_judgement_request(serde_json::Value::Array(fields).to_string().as_bytes()).err(),
            Some(JudgementRefusal::RequestMalformed)
        );
    }

    /// An `event_kind`-only request on a digest with no marks has a valid
    /// answer: no results. An empty answer to anything else is refused.
    #[test]
    fn an_event_kind_only_request_without_marks_is_answered_empty() {
        let sent = |kinds| JudgementRequest::new(nonce(), "claim".into(), b"ab".to_vec(), kinds);
        let request = sent(vec![JudgementKind::EventKind]).unwrap();
        let empty_for = |request: &JudgementRequest| {
            let empty = JudgementResponse::new(
                *request.request_nonce(),
                request.digest_sha256(),
                vec![],
                JudgementSpend {
                    account_month_remaining_bucket: SpendBucket::OverHalf,
                },
                1_792_540_800,
                format!("0x{}", "ef".repeat(65)),
            )
            .unwrap();
            parse_judgement_response(empty.to_json().as_bytes()).unwrap()
        };
        assert_eq!(empty_for(&request).check_binding(&request, 0), Ok(()));
        assert_eq!(
            empty_for(&request).check_binding(&request, 1),
            Err(JudgementBindingError::EventMarkMissing)
        );
        for kind in [JudgementKind::Task, JudgementKind::Finished] {
            let request = sent(vec![kind, JudgementKind::EventKind]).unwrap();
            assert_eq!(
                empty_for(&request).check_binding(&request, 0),
                Err(JudgementBindingError::JudgementMissing),
                "{kind:?}"
            );
        }
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
            parse_judgement_refusal(JudgementRoute::Judgement, 503, json.as_bytes()),
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
            parse_judgement_refusal(JudgementRoute::Judgement, 504, plain.to_json().as_bytes()),
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
                parse_judgement_refusal(JudgementRoute::Judgement, status, body.as_bytes()),
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

    // ---- nonces ----

    #[test]
    fn both_nonces_are_exactly_64_lowercase_hex() {
        let hex = "0f".repeat(32);
        assert_eq!(JudgementNonce::from_hex(&hex).unwrap().to_hex(), hex);
        assert_eq!(
            JudgementNonce::from_hex(&hex).unwrap().as_bytes(),
            &[0x0f; 32]
        );
        assert_eq!(JudgementRequestNonce::from_hex(&hex).unwrap().to_hex(), hex);
        assert_eq!(
            JudgementRequestNonce::from_hex(&hex).unwrap().as_bytes(),
            &[0x0f; 32]
        );
        for bad in [
            "0F".repeat(32),
            "0f".repeat(31),
            "0f".repeat(33),
            "zz".repeat(32),
            String::new(),
        ] {
            assert_eq!(
                JudgementNonce::from_hex(&bad),
                Err(JudgementWireError::Malformed("nonce")),
                "{bad}"
            );
            assert_eq!(
                JudgementRequestNonce::from_hex(&bad),
                Err(JudgementWireError::Malformed("request_nonce")),
                "{bad}"
            );
        }
    }

    // ---- base64 ----

    #[test]
    fn base64_matches_rfc_4648_vectors() {
        for (plain, encoded) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64_encode(plain.as_bytes()), encoded, "{plain}");
            assert_eq!(
                base64_decode(encoded).as_deref(),
                Some(plain.as_bytes()),
                "{encoded}"
            );
        }
        let every_byte: Vec<u8> = (0..=255u8).collect();
        for len in 0..every_byte.len() {
            let slice = &every_byte[..len];
            assert_eq!(base64_decode(&base64_encode(slice)).as_deref(), Some(slice));
        }
        // Both high alphabet characters.
        assert_eq!(base64_encode(&[0xfb, 0xff]), "+/8=");
    }

    #[test]
    fn base64_decoding_is_strict() {
        for bad in [
            "Zg=",       // length not a multiple of four
            "Zm9vYg",    // padding missing
            "Zh==",      // non-zero bits under the padding
            "Zm9=",      // non-zero bits under the padding
            "Z===",      // three padding characters
            "====",      // padding only
            "Zg==Zm9v",  // padding before the end
            "Zm=v",      // padding inside a quad
            "Zm9v\n",    // whitespace
            " Zm9v",     // whitespace
            "Zm-v",      // URL-safe alphabet
            "Zm_v",      // URL-safe alphabet
            "Zm9\u{e9}", // non-ASCII
        ] {
            assert_eq!(base64_decode(bad), None, "{bad:?}");
        }
    }

    // ---- attestation ----

    fn attestation_json(quote: &str, seal: &str, address: &str) -> String {
        serde_json::json!({
            "quote_hex": quote,
            "seal_public_key": seal,
            "signing_address": address,
        })
        .to_string()
    }

    #[test]
    fn an_attestation_response_parses_in_the_spec_shape() {
        let seal = format!("02{}", "11".repeat(32));
        let address = format!("0x{}", "ab".repeat(20));
        let json = attestation_json("00ff10", &seal, &address);
        let parsed = parse_judgement_attestation(json.as_bytes()).unwrap();
        assert_eq!(parsed.quote_hex(), "00ff10");
        assert_eq!(parsed.quote_bytes(), vec![0x00, 0xff, 0x10]);
        assert_eq!(parsed.seal_public_key(), seal);
        let mut seal_bytes = [0x11u8; 33];
        seal_bytes[0] = 0x02;
        assert_eq!(parsed.seal_public_key_bytes(), seal_bytes);
        assert_eq!(parsed.signing_address(), address);
        assert_eq!(parsed.signing_address_bytes(), [0xab; 20]);
        let value: serde_json::Value = serde_json::from_str(&parsed.to_json()).unwrap();
        assert_eq!(
            value,
            serde_json::from_str::<serde_json::Value>(&json).unwrap()
        );
        let rebuilt =
            JudgementAttestationResponse::new("00ff10".to_string(), seal.clone(), address.clone())
                .unwrap();
        assert_eq!(rebuilt, parsed);
        // Both compressed-point prefixes are accepted.
        let odd = format!("03{}", "11".repeat(32));
        assert!(
            parse_judgement_attestation(attestation_json("00", &odd, &address).as_bytes()).is_ok()
        );
    }

    #[test]
    fn a_malformed_attestation_response_is_refused() {
        let seal = format!("02{}", "11".repeat(32));
        let address = format!("0x{}", "ab".repeat(20));
        for (name, body) in [
            ("empty quote", attestation_json("", &seal, &address)),
            ("odd quote", attestation_json("abc", &seal, &address)),
            ("uppercase quote", attestation_json("AB", &seal, &address)),
            ("quote prefix", attestation_json("0xab", &seal, &address)),
            (
                "uncompressed seal key",
                attestation_json("ab", &format!("04{}", "11".repeat(64)), &address),
            ),
            (
                "seal key prefix",
                attestation_json("ab", &format!("05{}", "11".repeat(32)), &address),
            ),
            (
                "short seal key",
                attestation_json("ab", &format!("02{}", "11".repeat(31)), &address),
            ),
            (
                "uppercase seal key",
                attestation_json("ab", &format!("02{}", "AA".repeat(32)), &address),
            ),
            (
                "address without prefix",
                attestation_json("ab", &seal, &"ab".repeat(20)),
            ),
            (
                "short address",
                attestation_json("ab", &seal, &format!("0x{}", "ab".repeat(19))),
            ),
            (
                "uppercase address",
                attestation_json("ab", &seal, &format!("0x{}", "AB".repeat(20))),
            ),
            (
                "unknown field",
                r#"{"quote_hex":"ab","seal_public_key":"x","signing_address":"y","extra":1}"#
                    .to_string(),
            ),
            (
                "old shape without the seal key",
                format!(r#"{{"quote_hex":"ab","signing_address":"{address}"}}"#),
            ),
            ("not json", "nope".to_string()),
        ] {
            assert!(
                parse_judgement_attestation(body.as_bytes()).is_err(),
                "{name}"
            );
        }
    }

    // ---- request ----

    const CLAIM: &str = "eyJhbGciOiJFZERTQSJ9.eyJhY2NvdW50X3JlZiI6InQifQ.c2ln";

    fn request_nonce() -> JudgementRequestNonce {
        JudgementRequestNonce::from_bytes(std::array::from_fn(|i| 0xa0 ^ i as u8))
    }

    fn request_json(version: u32, digest: &[u8], judgements: &str) -> String {
        format!(
            r#"{{"wire_version":{version},"request_nonce":"{}","claim":"{CLAIM}","digest_b64":"{}","judgements":{judgements}}}"#,
            request_nonce().to_hex(),
            base64_encode(digest)
        )
    }

    fn all_kinds() -> Vec<JudgementKind> {
        vec![
            JudgementKind::Task,
            JudgementKind::EventKind,
            JudgementKind::Finished,
        ]
    }

    #[test]
    fn a_request_parses_in_the_spec_shape_and_hashes_its_decoded_digest() {
        let digest = b"{\"turns\":3}";
        let body = request_json(1, digest, r#"["task","event_kind","finished"]"#);
        let request = parse_judgement_request(body.as_bytes()).unwrap();
        assert_eq!(request.digest(), digest);
        assert_eq!(request.claim(), CLAIM);
        assert_eq!(request.request_nonce(), &request_nonce());
        assert_eq!(request.judgements(), all_kinds());
        use sha2::{Digest as _, Sha256};
        assert_eq!(request.digest_sha256(), hex::encode(Sha256::digest(digest)));

        let rebuilt = JudgementRequest::new(
            request_nonce(),
            CLAIM.to_string(),
            digest.to_vec(),
            all_kinds(),
        )
        .unwrap();
        assert_eq!(rebuilt, request);
        assert_eq!(rebuilt.to_json(), body);
        let value: serde_json::Value = serde_json::from_str(&rebuilt.to_json()).unwrap();
        let keys: Vec<_> = value.as_object().unwrap().keys().cloned().collect();
        let mut expected = [
            "claim",
            "digest_b64",
            "judgements",
            "request_nonce",
            "wire_version",
        ];
        expected.sort();
        let mut keys_sorted = keys.clone();
        keys_sorted.sort();
        assert_eq!(keys_sorted, expected);
        assert_eq!(value["digest_b64"], "eyJ0dXJucyI6M30=");
        assert_eq!(
            parse_judgement_request(rebuilt.to_json().as_bytes()).unwrap(),
            request
        );
    }

    #[test]
    fn the_sealed_request_cap_is_the_plaintext_cap_plus_the_seal_overhead() {
        assert_eq!(JUDGEMENT_REQUEST_MAX_BYTES, 16_384);
        // version byte, compressed ephemeral key, AEAD tag
        assert_eq!(JUDGEMENT_SEALED_REQUEST_MAX_BYTES, 16 * 1024 + 1 + 33 + 16);
        assert_eq!(JUDGEMENT_SEALED_REQUEST_MAX_BYTES, 16_434);
    }

    #[test]
    fn the_digest_cap_is_8192_decoded_bytes_and_refused_whole() {
        let at_cap = vec![b'a'; JUDGEMENT_DIGEST_MAX_BYTES];
        assert!(
            parse_judgement_request(request_json(1, &at_cap, r#"["task"]"#).as_bytes()).is_ok()
        );
        // The cap is on decoded bytes: an at-cap digest's base64 is longer
        // than 8192 characters and still parses.
        assert!(base64_encode(&at_cap).len() > JUDGEMENT_DIGEST_MAX_BYTES);
        let over = vec![b'a'; JUDGEMENT_DIGEST_MAX_BYTES + 1];
        assert_eq!(
            parse_judgement_request(request_json(1, &over, r#"["task"]"#).as_bytes()),
            Err(JudgementRefusal::DigestTooLarge)
        );
        assert_eq!(
            JudgementRequest::new(
                request_nonce(),
                CLAIM.to_string(),
                over,
                vec![JudgementKind::Task]
            ),
            Err(JudgementRefusal::DigestTooLarge)
        );
        // Multi-byte characters count as bytes, not characters.
        let multibyte = "\u{e9}"
            .repeat(JUDGEMENT_DIGEST_MAX_BYTES / 2 + 1)
            .into_bytes();
        assert_eq!(
            JudgementRequest::new(
                request_nonce(),
                CLAIM.to_string(),
                multibyte,
                vec![JudgementKind::Task]
            ),
            Err(JudgementRefusal::DigestTooLarge)
        );
        // A plaintext over the total cap is refused before it is parsed.
        let mut huge = request_json(1, b"x", r#"["task"]"#).into_bytes();
        huge.resize(JUDGEMENT_REQUEST_MAX_BYTES + 1, b' ');
        assert_eq!(
            parse_judgement_request(&huge),
            Err(JudgementRefusal::DigestTooLarge)
        );
    }

    /// The constructor refuses a request whose encoded plaintext is over the
    /// cap, so the daemon never builds one the service would refuse. An
    /// at-cap digest with a realistic claim fits; a claim whose JSON
    /// escaping multiplies its size does not, though its raw length does.
    #[test]
    fn a_request_whose_plaintext_is_over_the_cap_is_never_built() {
        let at_cap = vec![0xffu8; JUDGEMENT_DIGEST_MAX_BYTES];
        let realistic_claim = "a".repeat(1024);
        let fits = JudgementRequest::new(
            request_nonce(),
            realistic_claim,
            at_cap.clone(),
            all_kinds(),
        )
        .unwrap();
        assert!(fits.to_json().len() <= JUDGEMENT_REQUEST_MAX_BYTES);

        let escaped_claim = "\u{1}".repeat(1500);
        let raw_len = escaped_claim.len() + base64_encode(&at_cap).len();
        assert!(raw_len < JUDGEMENT_REQUEST_MAX_BYTES);
        assert_eq!(
            JudgementRequest::new(request_nonce(), escaped_claim, at_cap.clone(), all_kinds()),
            Err(JudgementRefusal::DigestTooLarge)
        );
        assert_eq!(
            JudgementRequest::new(
                request_nonce(),
                "a".repeat(JUDGEMENT_REQUEST_MAX_BYTES),
                b"x".to_vec(),
                all_kinds()
            ),
            Err(JudgementRefusal::DigestTooLarge)
        );
    }

    #[test]
    fn an_unknown_wire_version_is_unsupported_not_malformed() {
        assert_eq!(
            parse_judgement_request(request_json(2, b"x", r#"["task"]"#).as_bytes()),
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
        let nonce_hex = request_nonce().to_hex();
        let with = |field: &str, value: &str| {
            let mut v: serde_json::Value =
                serde_json::from_str(&request_json(1, b"x", r#"["task"]"#)).unwrap();
            v[field] = serde_json::from_str(value).unwrap();
            v.to_string()
        };
        let without = |field: &str| {
            let mut v: serde_json::Value =
                serde_json::from_str(&request_json(1, b"x", r#"["task"]"#)).unwrap();
            v.as_object_mut().unwrap().remove(field);
            v.to_string()
        };
        for body in [
            "not json".to_string(),
            without("wire_version"),
            without("request_nonce"),
            without("claim"),
            without("digest_b64"),
            without("judgements"),
            request_json(1, b"x", "[]"),
            request_json(1, b"x", r#"["task","task"]"#),
            request_json(1, b"x", r#"["mood"]"#),
            request_json(1, b"", r#"["task"]"#),
            with("extra", "1"),
            // the old v1 draft's plaintext digest field
            with("digest", r#""x""#),
            with("claim", r#""""#),
            with("digest_b64", r#""Zh==""#),
            with("digest_b64", r#""not base64!""#),
            with("digest_b64", r#""eA""#),
            with(
                "request_nonce",
                &format!(r#""{}""#, nonce_hex.to_uppercase()),
            ),
            with("request_nonce", &format!(r#""{}""#, &nonce_hex[..62])),
            with("request_nonce", r#"null"#),
        ] {
            assert_eq!(
                parse_judgement_request(body.as_bytes()),
                Err(JudgementRefusal::RequestMalformed),
                "{body}"
            );
        }
        assert_eq!(
            JudgementRequest::new(
                request_nonce(),
                String::new(),
                b"x".to_vec(),
                vec![JudgementKind::Task]
            ),
            Err(JudgementRefusal::RequestMalformed)
        );
    }

    /// The digest is scrubbed contributor text, the claim is a bearer
    /// credential naming the account, and the request nonce ties a response
    /// to one send: none of them is ever in `Debug`.
    #[test]
    fn request_debug_shows_only_the_digest_hash() {
        let request = JudgementRequest::new(
            request_nonce(),
            CLAIM.to_string(),
            b"secret contributor prompt".to_vec(),
            vec![JudgementKind::Task],
        )
        .unwrap();
        let debug = format!("{request:?}");
        assert!(!debug.contains("secret contributor prompt"));
        assert!(!debug.contains(&base64_encode(b"secret contributor prompt")));
        assert!(!debug.contains(CLAIM));
        assert!(!debug.contains(&request_nonce().to_hex()));
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
        assert_eq!(value["request_nonce"], nonce().to_hex());
        assert_eq!(response.request_nonce(), &nonce());
        assert_eq!(response.wire_version(), 1);
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
                "request nonce missing",
                Box::new(|v| {
                    v.as_object_mut().unwrap().remove("request_nonce");
                }),
            ),
            (
                "request nonce uppercase",
                Box::new(|v| v["request_nonce"] = "AB".repeat(32).into()),
            ),
            (
                "request nonce short",
                Box::new(|v| v["request_nonce"] = "ab".repeat(31).into()),
            ),
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

    /// A later wire version is reported as a version even when it also adds
    /// fields, so the client can tell "update the daemon" from garbage.
    #[test]
    fn a_later_response_version_is_unsupported_not_malformed() {
        let mut value: serde_json::Value = serde_json::from_str(&response().to_json()).unwrap();
        value["wire_version"] = 2.into();
        assert_eq!(
            parse_judgement_response(value.to_string().as_bytes()),
            Err(JudgementWireError::WireVersionUnsupported)
        );
        value["new_field"] = 1.into();
        assert_eq!(
            parse_judgement_response(value.to_string().as_bytes()),
            Err(JudgementWireError::WireVersionUnsupported)
        );
        // A v1 body with an unknown field is still malformed.
        value["wire_version"] = 1.into();
        assert_eq!(
            parse_judgement_response(value.to_string().as_bytes()),
            Err(JudgementWireError::Malformed("response"))
        );
        // No version at all is malformed, not unsupported.
        assert_eq!(
            parse_judgement_response(b"{}"),
            Err(JudgementWireError::Malformed("response"))
        );
    }

    // ---- response binding ----

    fn judged_event(index: u32) -> JudgementResult {
        JudgementResult::judged(
            JudgementKind::EventKind,
            Some(index),
            JudgementLabel::Redirect,
            JudgementConfidence {
                bucket: ConfidenceBucket::Medium,
                basis: ConfidenceBasis::Probability,
            },
            version("event-kind-v1+sha256:0123456789ab"),
            None,
        )
        .unwrap()
    }

    fn sent(judgements: Vec<JudgementKind>) -> JudgementRequest {
        JudgementRequest::new(
            request_nonce(),
            CLAIM.to_string(),
            b"{\"turns\":3}".to_vec(),
            judgements,
        )
        .unwrap()
    }

    /// Task, event marks `0..marks` (the first four judged, the rest
    /// unjudged), finished.
    fn full_results(marks: u32) -> Vec<JudgementResult> {
        let all = results();
        let mut out = vec![all[0].clone()];
        for index in 0..marks {
            out.push(if index < JUDGEMENT_EVENT_KIND_MAX {
                judged_event(index)
            } else {
                JudgementResult::unjudged_event(index).unwrap()
            });
        }
        out.push(all[2].clone());
        out
    }

    fn answer(request: &JudgementRequest, results: Vec<JudgementResult>) -> JudgementResponse {
        JudgementResponse::new(
            *request.request_nonce(),
            request.digest_sha256(),
            results,
            spend(SpendBucket::OverHalf),
            1_792_540_800,
            format!("0x{}", "ef".repeat(65)),
        )
        .unwrap()
    }

    #[test]
    fn a_response_bound_to_its_send_passes() {
        let request = sent(all_kinds());
        for marks in [1, 4, 6] {
            let response = answer(&request, full_results(marks));
            assert_eq!(response.check_binding(&request, marks), Ok(()), "{marks}");
        }
        // No marks in the digest: no event results, task and finished still
        // required.
        let no_marks = vec![results()[0].clone(), results()[2].clone()];
        assert_eq!(
            answer(&request, no_marks).check_binding(&request, 0),
            Ok(())
        );
        // Order of results is not part of the binding.
        let mut reordered = full_results(2);
        reordered.reverse();
        assert_eq!(
            answer(&request, reordered).check_binding(&request, 2),
            Ok(())
        );
        // Only what was asked for.
        let task_only = sent(vec![JudgementKind::Task]);
        assert_eq!(
            answer(&task_only, vec![results()[0].clone()]).check_binding(&task_only, 3),
            Ok(())
        );
    }

    #[test]
    fn a_response_for_another_send_is_refused() {
        let request = sent(all_kinds());
        let response = answer(&request, full_results(2));
        // Another request nonce, same digest.
        let mut other_nonce = *request_nonce().as_bytes();
        other_nonce[0] ^= 1;
        let other = JudgementRequest::new(
            JudgementRequestNonce::from_bytes(other_nonce),
            CLAIM.to_string(),
            request.digest().to_vec(),
            all_kinds(),
        )
        .unwrap();
        assert_eq!(
            response.check_binding(&other, 2),
            Err(JudgementBindingError::RequestNonceMismatch)
        );
        // Same nonce, another digest.
        let other = JudgementRequest::new(
            request_nonce(),
            CLAIM.to_string(),
            b"another session".to_vec(),
            all_kinds(),
        )
        .unwrap();
        assert_eq!(
            response.check_binding(&other, 2),
            Err(JudgementBindingError::DigestMismatch)
        );
        // A wire version other than 1, however it got there.
        let mut later = response.clone();
        later.wire_version = 2;
        assert_eq!(
            later.check_binding(&request, 2),
            Err(JudgementBindingError::WireVersionUnsupported)
        );
    }

    #[test]
    fn a_response_must_answer_exactly_what_was_asked() {
        let request = sent(all_kinds());
        let full = full_results(2);
        // Task missing.
        assert_eq!(
            answer(&request, full[1..].to_vec()).check_binding(&request, 2),
            Err(JudgementBindingError::JudgementMissing)
        );
        // Finished missing.
        assert_eq!(
            answer(&request, full[..full.len() - 1].to_vec()).check_binding(&request, 2),
            Err(JudgementBindingError::JudgementMissing)
        );
        // A judgement that was not asked for.
        let task_only = sent(vec![JudgementKind::Task]);
        assert_eq!(
            answer(&task_only, vec![results()[0].clone(), results()[2].clone()])
                .check_binding(&task_only, 0),
            Err(JudgementBindingError::JudgementUnrequested)
        );
        let no_events = sent(vec![JudgementKind::Task, JudgementKind::Finished]);
        assert_eq!(
            answer(&no_events, full_results(1)).check_binding(&no_events, 1),
            Err(JudgementBindingError::JudgementUnrequested)
        );
        // An event mark the digest does not have.
        assert_eq!(
            answer(&request, full_results(3)).check_binding(&request, 2),
            Err(JudgementBindingError::EventIndexOutOfRange)
        );
        assert_eq!(
            answer(&request, full_results(6)).check_binding(&request, 5),
            Err(JudgementBindingError::EventIndexOutOfRange)
        );
        // A mark the digest has, missing from the response.
        assert_eq!(
            answer(&request, full_results(2)).check_binding(&request, 3),
            Err(JudgementBindingError::EventMarkMissing)
        );
        assert_eq!(
            answer(&request, full_results(4)).check_binding(&request, 6),
            Err(JudgementBindingError::EventMarkMissing)
        );
        // A gap: marks 0 and 2 for a two-mark digest.
        let mut gap = full_results(1);
        gap.insert(2, judged_event(2));
        assert_eq!(
            answer(&request, gap).check_binding(&request, 2),
            Err(JudgementBindingError::EventIndexOutOfRange)
        );
    }

    // ---- signing bytes ----

    fn spend(bucket: SpendBucket) -> JudgementSpend {
        JudgementSpend {
            account_month_remaining_bucket: bucket,
        }
    }

    fn sign(
        wire_version: u32,
        request_nonce: &JudgementRequestNonce,
        digest_sha256: &str,
        results: &[JudgementResult],
        spend_bucket: SpendBucket,
        signed_at: i64,
    ) -> Vec<u8> {
        judgement_signing_bytes(
            wire_version,
            request_nonce,
            digest_sha256,
            results,
            spend(spend_bucket),
            signed_at,
        )
    }

    /// A `stuck` response, pinning tag 4 and the `stuck` label for a
    /// service not written in Rust.
    const FROZEN_STUCK_VECTOR: &str = concat!(
        // domain "trace_commons.insights_judgement.v1\n"
        "74726163655f636f6d6d6f6e732e696e7369676874735f6a756467656d656e742e76310a",
        // wire_version 1, u32 LE
        "01000000",
        // request_nonce 00..1f, 32 raw bytes
        "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
        // digest_sha256, length-prefixed hex
        "4000000000000000",
        "6162616261626162616261626162616261626162616261626162616261626162",
        "6162616261626162616261626162616261626162616261626162616261626162",
        // one result
        "0100000000000000",
        // stuck, no index, "stuck", judged, medium, self_reported
        "0400",
        "0500000000000000",
        "737475636b",
        "010202",
        "1c00000000000000",
        "737475636b2d76312b7368613235363a336363326639303537326265",
        "0100000000000000",
        "6d",
        // no receipt
        "00",
        // spend bucket under_tenth
        "03",
        // signed_at 1792540800, i64 LE
        "8000d86a00000000",
    );

    #[test]
    fn signing_bytes_frozen_stuck_vector() {
        let stuck = JudgementResult::judged(
            JudgementKind::Stuck,
            None,
            JudgementLabel::Stuck,
            JudgementConfidence {
                bucket: ConfidenceBucket::Medium,
                basis: ConfidenceBasis::SelfReported,
            },
            version("stuck-v1+sha256:3cc2f90572be"),
            None,
        )
        .unwrap();
        assert_eq!(
            hex::encode(sign(
                1,
                &nonce(),
                &"ab".repeat(32),
                &[stuck],
                SpendBucket::UnderTenth,
                1_792_540_800
            )),
            FROZEN_STUCK_VECTOR
        );
    }

    #[test]
    fn signing_bytes_frozen_vector() {
        assert_eq!(hex::encode(response().signing_bytes()), FROZEN_VECTOR);
        assert_eq!(
            sign(
                1,
                &nonce(),
                &"ab".repeat(32),
                &results(),
                SpendBucket::OverHalf,
                1_792_540_800
            ),
            response().signing_bytes()
        );
    }

    /// Changing any signed field changes the bytes.
    #[test]
    fn every_signed_field_moves_the_signing_bytes() {
        let original = response().signing_bytes();
        let digest = "ab".repeat(32);
        let at = 1_792_540_800;
        let over = SpendBucket::OverHalf;
        assert_eq!(original, sign(1, &nonce(), &digest, &results(), over, at));

        // wire_version, request_nonce and spend are signed (#1501 point 12).
        assert_ne!(original, sign(2, &nonce(), &digest, &results(), over, at));
        let mut other_nonce: [u8; 32] = *nonce().as_bytes();
        other_nonce[31] ^= 1;
        assert_ne!(
            original,
            sign(
                1,
                &JudgementRequestNonce::from_bytes(other_nonce),
                &digest,
                &results(),
                over,
                at
            )
        );
        for bucket in [SpendBucket::UnderHalf, SpendBucket::UnderTenth] {
            assert_ne!(
                original,
                sign(1, &nonce(), &digest, &results(), bucket, at),
                "{bucket:?}"
            );
        }
        assert_ne!(
            sign(1, &nonce(), &digest, &results(), SpendBucket::UnderHalf, at),
            sign(
                1,
                &nonce(),
                &digest,
                &results(),
                SpendBucket::UnderTenth,
                at
            )
        );

        assert_ne!(
            original,
            sign(1, &nonce(), &"ac".repeat(32), &results(), over, at)
        );
        assert_ne!(
            original,
            sign(1, &nonce(), &digest, &results(), over, at + 1)
        );
        assert_ne!(
            original,
            sign(1, &nonce(), &digest, &results()[..2], over, at)
        );
        let mut reordered = results();
        reordered.swap(0, 2);
        assert_ne!(original, sign(1, &nonce(), &digest, &reordered, over, at));

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
            sign(1, &nonce(), &digest, &r, over, at)
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
        assert_ne!(original, sign(1, &nonce(), &digest, &moved, over, at));
    }

    /// The signature is the only response field outside the signed bytes.
    #[test]
    fn only_the_signature_is_not_signed() {
        let mut other = response();
        other.signature = format!("0x{}", "01".repeat(65));
        assert_eq!(other.signing_bytes(), response().signing_bytes());
        let mut other = response();
        other.spend = spend(SpendBucket::UnderTenth);
        assert_ne!(other.signing_bytes(), response().signing_bytes());
    }

    #[test]
    fn adjacent_boundary_shifts_do_not_collide() {
        // The digest hash is the one variable field next to a fixed one; a
        // byte moved across its end must not read alike.
        let r = results();
        let over = SpendBucket::OverHalf;
        assert_ne!(
            sign(1, &nonce(), "abc", &r, over, 0),
            sign(1, &nonce(), "ab", &r, over, 0)
        );
        // An absent event index and a present one cannot read alike.
        let task_only = vec![results().remove(0)];
        let event = vec![JudgementResult::unjudged_event(4).unwrap()];
        assert_ne!(
            sign(1, &nonce(), &"ab".repeat(32), &task_only, over, 0),
            sign(1, &nonce(), &"ab".repeat(32), &event, over, 0)
        );
    }

    #[test]
    fn the_claim_route_and_schema_are_pinned() {
        assert_eq!(JUDGEMENT_CLAIM_PATH, "/v1/insights-judgement/claim");
        assert_eq!(
            JUDGEMENT_CLAIM_REQUEST_SCHEMA_VERSION,
            "trace_commons.insights_judgement_claim_request.v1"
        );
        assert_eq!(JUDGEMENT_CLAIM_MAX_SKEW_SECONDS, 300);
        assert_eq!(
            JUDGEMENT_CLAIM_REQUEST_STALE,
            "insights_judgement_claim_request_stale"
        );
    }

    /// The claim route's refusals are a closed set, and these strings are the
    /// wire: the daemon's re-mint logic matches them and nothing else.
    #[test]
    fn the_claim_refusal_labels_statuses_and_retry_classes_are_pinned() {
        use JudgementClaimRefusal as R;
        use JudgementClaimRetry as Retry;
        let expected: [(R, &str, u16, Retry); 10] = [
            (R::Malformed, "judgement_claim_malformed", 400, Retry::Never),
            (
                R::DeviceKeyMismatch,
                "judgement_claim_device_key_mismatch",
                400,
                Retry::Never,
            ),
            (
                R::Stale,
                "insights_judgement_claim_request_stale",
                400,
                Retry::WithFreshTimestamp,
            ),
            (
                R::SignatureInvalid,
                "judgement_claim_signature_invalid",
                403,
                Retry::Never,
            ),
            (
                R::DeviceKeyNotRegistered,
                "device_key_not_registered",
                403,
                Retry::Never,
            ),
            (R::DeviceKeyRevoked, "device_key_revoked", 403, Retry::Never),
            (
                R::AccountIneligible,
                "judgement_account_ineligible",
                403,
                Retry::Never,
            ),
            (
                R::GrantRequired,
                "judgement_claim_grant_required",
                403,
                Retry::Never,
            ),
            (
                R::RegistryUnavailable,
                "judgement_claim_registry_unavailable",
                503,
                Retry::AfterDelay,
            ),
            (
                R::IssueFailed,
                "judgement_claim_issue_failed",
                500,
                Retry::AfterDelay,
            ),
        ];
        assert_eq!(R::ALL.len(), expected.len());
        for (refusal, (want, label, status, retry)) in R::ALL.into_iter().zip(expected) {
            assert_eq!(refusal, want);
            assert_eq!(refusal.label(), label);
            assert_eq!(refusal.status(), status, "{label}");
            assert_eq!(refusal.retry(), retry, "{label}");
            assert_eq!(R::from_label(label), Some(refusal));
        }
        assert_eq!(JUDGEMENT_CLAIM_RETRY_AFTER_SECS, 30);
        assert_eq!(JUDGEMENT_CLAIM_REQUEST_STALE, R::Stale.label());
        // The ineligible label is the service's own, so a daemon reads it the
        // same way from either hop.
        assert_eq!(
            R::AccountIneligible.label(),
            JudgementRefusal::AccountIneligible.label()
        );
        // The upload-claim route's prose is not a member.
        for prose in [
            "device key not registered",
            "device key revoked",
            "failed to issue upload claim",
            "device key registry is not configured",
            "invalid device key signature",
        ] {
            assert_eq!(R::from_label(prose), None, "{prose}");
        }
        let mut labels: Vec<&str> = R::ALL.iter().map(|r| r.label()).collect();
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), R::ALL.len(), "labels are distinct");
    }

    #[test]
    fn claim_wire_types_round_trip_and_refuse_unknown_fields() {
        let request = JudgementClaimRequest {
            schema_version: JUDGEMENT_CLAIM_REQUEST_SCHEMA_VERSION.to_string(),
            tenant_id: format!("near-{}", "a".repeat(64)),
            device_key_id: "sha256:00".to_string(),
            issued_at: 1,
        };
        let value = serde_json::to_value(&request).unwrap();
        assert_eq!(
            serde_json::from_value::<JudgementClaimRequest>(value.clone()).unwrap(),
            request
        );
        let mut smuggled = value;
        smuggled["consent_scopes"] = serde_json::json!(["benchmark_only"]);
        assert!(serde_json::from_value::<JudgementClaimRequest>(smuggled).is_err());

        let response = JudgementClaimResponse {
            access_token: "a.b.c".to_string(),
            token_type: "Bearer".to_string(),
            expires_at: 2,
            expires_in: 1,
        };
        let value = serde_json::to_value(&response).unwrap();
        assert_eq!(
            serde_json::from_value::<JudgementClaimResponse>(value.clone()).unwrap(),
            response
        );
        let mut smuggled = value;
        smuggled["account_ref"] = serde_json::json!("near-00");
        assert!(serde_json::from_value::<JudgementClaimResponse>(smuggled).is_err());
    }
}
