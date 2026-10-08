//! A local estimate of the credit a waiting session is likely to be shown,
//! made on the contributor's device before anything is sent.
//!
//! What a contributor is shown once the commons scores a session is
//! `round(10 * q, 2)`, where `q` is the gate's credit quality. `q` is driven by
//! perplexity under the scoring model and novelty against the tenant's vector
//! index, and neither exists on a device. So this module does not compute `q`
//! and does not port the server's credit-quality function or its constants
//! (OWNER DECISION E14). It extracts a few content-free numbers from a session
//! ([`LocalEstimateFeatures`], version `lef1`) and looks them up in a published
//! calibration table ([`LocalEstimateTable`]) whose tiers carry the p10-p90
//! band of displayed credit observed for sessions that fell in each tier.
//!
//! Names are kept apart from the submit-time `CreditEstimate` in
//! [`crate::trace_contribution`]: that one is a different computation, and the
//! server no longer shows it to contributors.
//!
//! Rules this module holds:
//!
//! - **Never 0.** [`estimate`] returns `None` when it cannot say something
//!   true; a table whose band could display as 0 is refused.
//! - **One definition, two callers.** The daemon feeds [`LocalEstimateAccumulator`]
//!   from its parsed transcript; the server feeds it from a stored envelope
//!   through [`LocalEstimateFeatures::from_envelope`]. Both go through the same
//!   [`LocalEstimateAccumulator::accumulate`].
//! - **Bounded cost.** The entropy pass reads at most
//!   `ESTIMATE_SAMPLE_WINDOWS * ESTIMATE_SAMPLE_WINDOW_BYTES` bytes whatever
//!   the session's size.
//! - **No content.** Features are counts and ratios only.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::trace_contribution::{TraceContributionEnvelope, TraceContributionEventType};

/// The feature definitions this module computes. A table fit against other
/// definitions does not apply to these features, and [`estimate`] says so by
/// returning `None`.
pub const LOCAL_ESTIMATE_FEATURES_VERSION: &str = "lef1";

/// The table schema this build reads. A newer schema is refused rather than
/// half-read.
pub const ESTIMATE_TABLE_SCHEMA_VERSION: u32 = 1;

/// Number of windows in the strided entropy sample.
///
/// OWNER DECISION (spec section 4.2, serving E5 and E15): mirrors the gate's
/// chunk cap so a long session is sampled across its length, never only from
/// its opening.
pub const ESTIMATE_SAMPLE_WINDOWS: usize = 16;

/// Bytes per window in the strided entropy sample.
///
/// OWNER DECISION (spec section 4.2, serving E5 and E15). With
/// [`ESTIMATE_SAMPLE_WINDOWS`] this bounds the entropy pass at 256 KiB.
pub const ESTIMATE_SAMPLE_WINDOW_BYTES: usize = 16 * 1024;

/// The most bytes the entropy pass reads, whatever the session's size.
pub const ESTIMATE_SAMPLE_BYTES: usize = ESTIMATE_SAMPLE_WINDOWS * ESTIMATE_SAMPLE_WINDOW_BYTES;

/// Displayed bands are rounded outward to this step: `low` floored, `high`
/// ceiled. Never a single number.
///
/// OWNER DECISION E3.
pub const ESTIMATE_DISPLAY_STEP: f64 = 0.5;

/// The most tiers a table may carry. The whole p10-p90 range of displayed
/// credit is about 1.3 to 2.9, so anything finer is false precision.
///
/// OWNER DECISION E4.
pub const ESTIMATE_MAX_TIERS: usize = 3;

/// `user_messages` is capped here before it enters a weighted term, so one
/// very long conversation cannot dominate the score.
///
/// OWNER DECISION E1 (the option (a) feature transform, spec section 4.3).
pub const ESTIMATE_USER_MESSAGES_CAP: u32 = 20;

/// The most weighted terms a table may carry. One per [`EstimateTerm`] today;
/// a table naming more is refused.
pub const ESTIMATE_MAX_WEIGHTS: usize = 16;

/// The largest absolute weight or cut-off a table may carry. A larger one is a
/// malformed table, not a calibration.
pub const ESTIMATE_MAX_ABS_WEIGHT: f64 = 1.0e6;

/// The largest displayed credit a band may name: displayed credit is
/// `10 * q` and `q` is at most 1.
pub const ESTIMATE_MAX_DISPLAYED_CREDIT: f64 = 10.0;

/// The longest version or calibration label a table may carry.
pub const ESTIMATE_MAX_LABEL_LEN: usize = 32;

/// Upper bound on each of a table's chunk parameters (`bytes_per_token`,
/// `chunk_target_tokens`, `chunk_cap`). Zero is refused too.
pub const ESTIMATE_MAX_CHUNK_PARAMETER: u32 = 1 << 20;

/// Built-in table version label.
///
/// OWNER DECISION E2.
pub const BUILT_IN_ESTIMATE_TABLE_VERSION: &str = "t1";

/// The credit-quality calibration the built-in band was read from.
///
/// OWNER DECISION E2.
pub const BUILT_IN_ESTIMATE_CREDIT_QUALITY_CALIBRATION: &str = "cq3";

/// Built-in band low end, in displayed-credit units: ten times the V3
/// calibration report's credit-quality p10 of 0.127. Displays as 1.
///
/// OWNER DECISION E2.
pub const BUILT_IN_ESTIMATE_BAND_LOW: f64 = 1.27;

/// Built-in band high end, in displayed-credit units: ten times the V3
/// calibration report's credit-quality p90 of 0.290. Displays as 3.
///
/// OWNER DECISION E2.
pub const BUILT_IN_ESTIMATE_BAND_HIGH: f64 = 2.90;

/// Bytes per token for the chunk estimate, copying the gate chunker's
/// characters-per-token figure as table data.
///
/// OWNER DECISION E2 (spec section 4.8).
pub const BUILT_IN_ESTIMATE_BYTES_PER_TOKEN: u32 = 4;

/// Chunk target in tokens, copying the server default as table data.
///
/// OWNER DECISION E2 (spec section 4.8).
pub const BUILT_IN_ESTIMATE_CHUNK_TARGET_TOKENS: u32 = 2048;

/// Chunk cap, copying the server default as table data.
///
/// OWNER DECISION E2 (spec section 4.8).
pub const BUILT_IN_ESTIMATE_CHUNK_CAP: u32 = 16;

/// Slack used when rounding a band outward, so `6.000000000000001` steps do
/// not ceil a 3.0 up to 3.5.
const ROUNDING_SLACK: f64 = 1.0e-9;

/// What an event is, as far as the estimate cares.
///
/// Mapping from an envelope's [`TraceContributionEventType`]:
/// `UserMessage` -> `User`, `AssistantMessage` -> `Assistant`, `ToolResult`
/// -> `ToolResult`, `RoutingDecision` -> `Routing`, and `Reasoning`,
/// `ToolCall`, `Feedback`, `HttpExchange` -> `Other`. Reasoning counts as
/// other, as the gate's per-author attribution does.
///
/// The daemon maps its own event kinds the same way, and an event kind whose
/// text the envelope builder drops (an opaque record) must be fed with no
/// text, so both sides count the same bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EstimateRole {
    User,
    Assistant,
    ToolResult,
    Other,
    /// Attribution metadata about which backend served a request, not
    /// conversation content. Excluded from every feature.
    Routing,
}

impl EstimateRole {
    #[must_use]
    pub fn of_event_type(event_type: &TraceContributionEventType) -> Self {
        match event_type {
            TraceContributionEventType::UserMessage => Self::User,
            TraceContributionEventType::AssistantMessage => Self::Assistant,
            TraceContributionEventType::ToolResult => Self::ToolResult,
            TraceContributionEventType::RoutingDecision => Self::Routing,
            TraceContributionEventType::Reasoning
            | TraceContributionEventType::ToolCall
            | TraceContributionEventType::Feedback
            | TraceContributionEventType::HttpExchange => Self::Other,
        }
    }
}

/// Content-free numbers about one session, version `lef1`.
///
/// Byte counts are stored rather than ratios so the struct serializes
/// deterministically; the shares are derived. Chunk count and the cap flag are
/// derived against a table at estimate time, because they depend on the
/// table's chunk parameters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalEstimateFeatures {
    /// Always [`LOCAL_ESTIMATE_FEATURES_VERSION`] when built here.
    pub version: String,
    /// Content bytes over every event except routing rows.
    pub content_bytes: u64,
    /// Content bytes of tool results.
    pub tool_result_bytes: u64,
    /// Content bytes of assistant messages (reasoning excluded).
    pub agent_prose_bytes: u64,
    /// Events with role user whose text is not empty after trimming.
    pub user_messages: u32,
    /// Distinct non-empty tool names on non-routing events.
    pub distinct_tools: u32,
    /// Order-0 byte entropy in bits per byte, times 1000, over the strided
    /// sample. `None` when there is no content to measure, never 0 for that.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub byte_entropy_milli: Option<u32>,
}

impl LocalEstimateFeatures {
    /// Features of a stored envelope, using each event's (redacted) content.
    #[must_use]
    pub fn from_envelope(envelope: &TraceContributionEnvelope) -> Self {
        let mut acc = LocalEstimateAccumulator::new();
        for event in &envelope.events {
            acc.accumulate(
                EstimateRole::of_event_type(&event.event_type),
                event.redacted_content.as_deref(),
                event.tool_name.as_deref(),
            );
        }
        acc.finish()
    }

    /// Tool-result bytes as a share of content bytes; `None` with no content.
    #[must_use]
    pub fn tool_result_share(&self) -> Option<f64> {
        share(self.tool_result_bytes, self.content_bytes)
    }

    /// Assistant bytes as a share of content bytes; `None` with no content.
    #[must_use]
    pub fn agent_prose_share(&self) -> Option<f64> {
        share(self.agent_prose_bytes, self.content_bytes)
    }

    /// `ceil(content_bytes / (bytes_per_token * chunk_target_tokens))`.
    #[must_use]
    pub fn est_chunks(&self, table: &LocalEstimateTable) -> u64 {
        let per_chunk = u64::from(table.bytes_per_token) * u64::from(table.chunk_target_tokens);
        if per_chunk == 0 {
            // Refused by `validate`; a table built around it gets no chunks.
            return 0;
        }
        self.content_bytes.div_ceil(per_chunk)
    }

    /// Whether the session would be scored on a strided sample.
    #[must_use]
    pub fn capped(&self, table: &LocalEstimateTable) -> bool {
        self.est_chunks(table) > u64::from(table.chunk_cap)
    }

    /// The transformed value of one weighted term, or `None` when the
    /// feature behind it is unknown. Public so a fit computes exactly the
    /// values [`estimate_score`] weights.
    #[must_use]
    pub fn term_value(&self, term: EstimateTerm, table: &LocalEstimateTable) -> Option<f64> {
        match term {
            EstimateTerm::LnContentBytes => Some((self.content_bytes as f64).ln_1p()),
            EstimateTerm::Capped => Some(if self.capped(table) { 1.0 } else { 0.0 }),
            EstimateTerm::ToolResultShare => self.tool_result_share(),
            EstimateTerm::AgentProseShare => self.agent_prose_share(),
            EstimateTerm::ByteEntropy => self.byte_entropy_milli.map(|m| f64::from(m) / 1000.0),
            EstimateTerm::UserMessagesCapped => Some(f64::from(
                self.user_messages.min(ESTIMATE_USER_MESSAGES_CAP),
            )),
            EstimateTerm::LnDistinctTools => Some(f64::from(self.distinct_tools).ln_1p()),
        }
    }
}

/// Collects a session's events, then computes [`LocalEstimateFeatures`].
///
/// Two phases because the strided entropy sample needs the total length before
/// it can place its windows: [`Self::accumulate`] borrows each event's text
/// and sums byte counts; [`Self::finish`] places the windows.
#[derive(Debug, Default)]
pub struct LocalEstimateAccumulator<'a> {
    segments: Vec<&'a str>,
    content_bytes: u64,
    tool_result_bytes: u64,
    agent_prose_bytes: u64,
    user_messages: u32,
    tools: BTreeSet<&'a str>,
}

impl<'a> LocalEstimateAccumulator<'a> {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The single entry point both callers use, one call per event.
    pub fn accumulate(
        &mut self,
        role: EstimateRole,
        text: Option<&'a str>,
        tool_name: Option<&'a str>,
    ) {
        if role == EstimateRole::Routing {
            return;
        }
        if let Some(name) = tool_name.filter(|name| !name.trim().is_empty()) {
            self.tools.insert(name);
        }
        let Some(text) = text.filter(|text| !text.is_empty()) else {
            return;
        };
        let bytes = text.len() as u64;
        self.content_bytes = self.content_bytes.saturating_add(bytes);
        match role {
            EstimateRole::User => {
                if !text.trim().is_empty() {
                    self.user_messages = self.user_messages.saturating_add(1);
                }
            }
            EstimateRole::Assistant => {
                self.agent_prose_bytes = self.agent_prose_bytes.saturating_add(bytes);
            }
            EstimateRole::ToolResult => {
                self.tool_result_bytes = self.tool_result_bytes.saturating_add(bytes);
            }
            EstimateRole::Other | EstimateRole::Routing => {}
        }
        self.segments.push(text);
    }

    #[must_use]
    pub fn finish(self) -> LocalEstimateFeatures {
        let total: usize = self.segments.iter().map(|s| s.len()).sum();
        let (histogram, read) = sample_histogram(&self.segments, total);
        LocalEstimateFeatures {
            version: LOCAL_ESTIMATE_FEATURES_VERSION.to_string(),
            content_bytes: self.content_bytes,
            tool_result_bytes: self.tool_result_bytes,
            agent_prose_bytes: self.agent_prose_bytes,
            user_messages: self.user_messages,
            distinct_tools: u32::try_from(self.tools.len()).unwrap_or(u32::MAX),
            byte_entropy_milli: entropy_milli(&histogram, read),
        }
    }
}

fn share(part: u64, whole: u64) -> Option<f64> {
    (whole > 0).then(|| part as f64 / whole as f64)
}

/// Order-0 byte histogram over the endpoint-inclusive strided sample of the
/// concatenation of `segments` (whose lengths sum to `total`), and how many
/// bytes it read.
///
/// Up to [`ESTIMATE_SAMPLE_BYTES`] everything is read. Beyond it,
/// [`ESTIMATE_SAMPLE_WINDOWS`] windows of [`ESTIMATE_SAMPLE_WINDOW_BYTES`] are
/// placed evenly from the first byte to the last, so the opening and the end
/// are both in the sample and the windows never overlap.
fn sample_histogram(segments: &[&str], total: usize) -> ([u64; 256], usize) {
    let mut histogram = [0u64; 256];
    if total <= ESTIMATE_SAMPLE_BYTES {
        let mut read = 0;
        for segment in segments {
            for &byte in segment.as_bytes() {
                histogram[usize::from(byte)] += 1;
            }
            read += segment.len();
        }
        return (histogram, read);
    }

    // Start offset of each segment in the concatenation.
    let mut starts = Vec::with_capacity(segments.len());
    let mut offset = 0usize;
    for segment in segments {
        starts.push(offset);
        offset += segment.len();
    }

    let span = (total - ESTIMATE_SAMPLE_WINDOW_BYTES) as u128;
    let steps = (ESTIMATE_SAMPLE_WINDOWS - 1) as u128;
    let mut read = 0;
    for window in 0..ESTIMATE_SAMPLE_WINDOWS {
        let from = (window as u128 * span / steps) as usize;
        let to = from + ESTIMATE_SAMPLE_WINDOW_BYTES;
        // The last segment starting at or before `from`.
        let mut index = starts.partition_point(|&start| start <= from) - 1;
        let mut position = from;
        while position < to && index < segments.len() {
            let bytes = segments[index].as_bytes();
            let local_from = position - starts[index];
            let local_to = (to - starts[index]).min(bytes.len());
            for &byte in &bytes[local_from..local_to] {
                histogram[usize::from(byte)] += 1;
            }
            read += local_to - local_from;
            position = starts[index] + local_to;
            index += 1;
        }
    }
    (histogram, read)
}

/// Order-0 entropy in bits per byte, times 1000. `None` with nothing read.
fn entropy_milli(histogram: &[u64; 256], read: usize) -> Option<u32> {
    if read == 0 {
        return None;
    }
    let n = read as f64;
    let bits: f64 = histogram
        .iter()
        .filter(|&&count| count > 0)
        .map(|&count| {
            let p = count as f64 / n;
            -p * p.log2()
        })
        .sum();
    // At most 8 bits per byte, so this fits comfortably.
    Some((bits * 1000.0).round().max(0.0) as u32)
}

/// A weighted term of the estimate score, over transformed features.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstimateTerm {
    /// `ln(1 + content_bytes)`.
    LnContentBytes,
    /// 1 when the session would be scored on a strided sample, else 0.
    Capped,
    /// Tool-result bytes over content bytes.
    ToolResultShare,
    /// Assistant bytes over content bytes.
    AgentProseShare,
    /// Byte entropy in bits per byte.
    ByteEntropy,
    /// `min(user_messages, ESTIMATE_USER_MESSAGES_CAP)`.
    UserMessagesCapped,
    /// `ln(1 + distinct_tools)`.
    LnDistinctTools,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EstimateWeight {
    pub term: EstimateTerm,
    pub weight: f64,
}

/// The p10-p90 band of displayed credit for one tier, in displayed-credit
/// units (ten times credit quality), before display rounding.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EstimateBand {
    pub low: f64,
    pub high: f64,
}

/// A tier label. A two-tier table uses `Lower` and `Higher`; a one-tier table
/// reports no tier at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstimateTier {
    Lower,
    Middle,
    Higher,
}

/// A published calibration table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalEstimateTable {
    pub schema_version: u32,
    /// Must equal [`LOCAL_ESTIMATE_FEATURES_VERSION`].
    pub features_version: String,
    /// This table's own version label, such as `t1`.
    pub version: String,
    /// The credit-quality calibration the table was fit against, such as `cq3`.
    pub credit_quality_calibration: String,
    pub bytes_per_token: u32,
    pub chunk_target_tokens: u32,
    pub chunk_cap: u32,
    #[serde(default)]
    pub weights: Vec<EstimateWeight>,
    /// Ascending score cut-offs, one fewer than `bands`. A score at or above a
    /// cut-off is in the tier above it.
    #[serde(default)]
    pub cut_offs: Vec<f64>,
    /// One band per tier, lowest tier first.
    pub bands: Vec<EstimateBand>,
    /// Share of the fit window's decisions displayed as 0 by the duplicate
    /// and hold paths, which the bands exclude. Omitted when unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub withheld_share: Option<f64>,
}

/// Why a table was refused.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum EstimateTableError {
    #[error("estimate table is not a valid table document")]
    Malformed,
    #[error("estimate table schema version is not supported")]
    UnsupportedSchema,
    #[error("estimate table is for other feature definitions")]
    FeaturesVersion,
    #[error("estimate table label is invalid")]
    Label,
    #[error("estimate table chunk parameters are invalid")]
    ChunkParameters,
    #[error("estimate table has too many or too few tiers")]
    Tiers,
    #[error("estimate table cut-offs are invalid")]
    CutOffs,
    #[error("estimate table has too many or repeated weights")]
    Weights,
    #[error("estimate table carries a number out of range")]
    OutOfRange,
}

impl LocalEstimateTable {
    /// The table in force until a fitted one is accepted: one tier, zero
    /// weights, the V3 band. Labelled `lef1.t1/cq3`.
    #[must_use]
    pub fn built_in() -> Self {
        Self {
            schema_version: ESTIMATE_TABLE_SCHEMA_VERSION,
            features_version: LOCAL_ESTIMATE_FEATURES_VERSION.to_string(),
            version: BUILT_IN_ESTIMATE_TABLE_VERSION.to_string(),
            credit_quality_calibration: BUILT_IN_ESTIMATE_CREDIT_QUALITY_CALIBRATION.to_string(),
            bytes_per_token: BUILT_IN_ESTIMATE_BYTES_PER_TOKEN,
            chunk_target_tokens: BUILT_IN_ESTIMATE_CHUNK_TARGET_TOKENS,
            chunk_cap: BUILT_IN_ESTIMATE_CHUNK_CAP,
            weights: Vec::new(),
            cut_offs: Vec::new(),
            bands: vec![EstimateBand {
                low: BUILT_IN_ESTIMATE_BAND_LOW,
                high: BUILT_IN_ESTIMATE_BAND_HIGH,
            }],
            withheld_share: None,
        }
    }

    /// Parse and validate a table document. Anything this build cannot read
    /// completely is refused.
    pub fn from_value(value: &Value) -> Result<Self, EstimateTableError> {
        // Read the schema first: a newer table may not have this shape at all,
        // and "unsupported" is the truer refusal than "malformed".
        let schema = value
            .get("schema_version")
            .and_then(Value::as_u64)
            .ok_or(EstimateTableError::Malformed)?;
        if schema != u64::from(ESTIMATE_TABLE_SCHEMA_VERSION) {
            return Err(EstimateTableError::UnsupportedSchema);
        }
        // Bound the arrays before deserializing them.
        if value
            .get("weights")
            .and_then(Value::as_array)
            .is_some_and(|weights| weights.len() > ESTIMATE_MAX_WEIGHTS)
        {
            return Err(EstimateTableError::Weights);
        }
        if value
            .get("bands")
            .and_then(Value::as_array)
            .is_some_and(|bands| bands.len() > ESTIMATE_MAX_TIERS)
        {
            return Err(EstimateTableError::Tiers);
        }
        let table: Self =
            serde_json::from_value(value.clone()).map_err(|_| EstimateTableError::Malformed)?;
        table.validate()?;
        Ok(table)
    }

    /// The checks [`Self::from_value`] applies, for a table built in code.
    pub fn validate(&self) -> Result<(), EstimateTableError> {
        if self.schema_version != ESTIMATE_TABLE_SCHEMA_VERSION {
            return Err(EstimateTableError::UnsupportedSchema);
        }
        if self.features_version != LOCAL_ESTIMATE_FEATURES_VERSION {
            return Err(EstimateTableError::FeaturesVersion);
        }
        if !valid_label(&self.version) || !valid_label(&self.credit_quality_calibration) {
            return Err(EstimateTableError::Label);
        }
        let chunk_parameter_ok = |value: u32| (1..=ESTIMATE_MAX_CHUNK_PARAMETER).contains(&value);
        if !chunk_parameter_ok(self.bytes_per_token)
            || !chunk_parameter_ok(self.chunk_target_tokens)
            || !chunk_parameter_ok(self.chunk_cap)
        {
            return Err(EstimateTableError::ChunkParameters);
        }
        let tiers = self.bands.len();
        if tiers == 0 || tiers > ESTIMATE_MAX_TIERS {
            return Err(EstimateTableError::Tiers);
        }
        if self.weights.len() > ESTIMATE_MAX_WEIGHTS {
            return Err(EstimateTableError::Weights);
        }
        let mut terms = BTreeSet::new();
        for weight in &self.weights {
            if !terms.insert(weight.term) {
                return Err(EstimateTableError::Weights);
            }
            if !bounded(weight.weight) {
                return Err(EstimateTableError::OutOfRange);
            }
        }
        if self.cut_offs.len() != tiers - 1 {
            return Err(EstimateTableError::CutOffs);
        }
        if !self.cut_offs.iter().all(|&cut| bounded(cut)) {
            return Err(EstimateTableError::OutOfRange);
        }
        if !self.cut_offs.windows(2).all(|pair| pair[0] < pair[1]) {
            return Err(EstimateTableError::CutOffs);
        }
        for band in &self.bands {
            let in_range = band.low.is_finite()
                && band.high.is_finite()
                && band.low >= ESTIMATE_DISPLAY_STEP
                && band.high <= ESTIMATE_MAX_DISPLAYED_CREDIT
                && band.low <= band.high;
            if !in_range {
                return Err(EstimateTableError::OutOfRange);
            }
        }
        if let Some(withheld) = self.withheld_share {
            if !(withheld.is_finite() && (0.0..=1.0).contains(&withheld)) {
                return Err(EstimateTableError::OutOfRange);
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn tier_count(&self) -> usize {
        self.bands.len()
    }

    /// `features_version.version/credit_quality_calibration`, such as
    /// `lef1.t1/cq3`.
    #[must_use]
    pub fn calibration_label(&self) -> String {
        format!(
            "{}.{}/{}",
            self.features_version, self.version, self.credit_quality_calibration
        )
    }

    /// The tier labels in use, lowest first. Empty for a one-tier table.
    fn tier_labels(&self) -> &'static [EstimateTier] {
        match self.bands.len() {
            2 => &[EstimateTier::Lower, EstimateTier::Higher],
            3 => &[
                EstimateTier::Lower,
                EstimateTier::Middle,
                EstimateTier::Higher,
            ],
            _ => &[],
        }
    }
}

fn valid_label(label: &str) -> bool {
    !label.is_empty()
        && label.len() <= ESTIMATE_MAX_LABEL_LEN
        && label
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn bounded(value: f64) -> bool {
    value.is_finite() && value.abs() <= ESTIMATE_MAX_ABS_WEIGHT
}

/// What a shell may show: a band, never a single number, and never 0.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LocalCreditEstimate {
    /// Band low end, floored to [`ESTIMATE_DISPLAY_STEP`]. Always above 0.
    pub low: f64,
    /// Band high end, ceiled to [`ESTIMATE_DISPLAY_STEP`].
    pub high: f64,
    /// Omitted for a one-tier table, which carries no ordering information.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tier: Option<EstimateTier>,
    /// The table's [`LocalEstimateTable::calibration_label`].
    pub calibration: String,
}

/// The weighted sum the table's cut-offs are placed on, or `None` when a
/// weighted feature is unknown or the sum is not finite. A zero weight is
/// skipped, so an unknown feature behind it does not matter.
#[must_use]
pub fn estimate_score(features: &LocalEstimateFeatures, table: &LocalEstimateTable) -> Option<f64> {
    let mut score = 0.0;
    for weight in &table.weights {
        if weight.weight == 0.0 {
            continue;
        }
        score += weight.weight * features.term_value(weight.term, table)?;
    }
    score.is_finite().then_some(score)
}

/// The estimate for one session under one table, or `None` when it cannot be
/// made honestly: features of another version, no content, a weighted feature
/// that is unknown, or a band that would display as 0.
#[must_use]
pub fn estimate(
    features: &LocalEstimateFeatures,
    table: &LocalEstimateTable,
) -> Option<LocalCreditEstimate> {
    if features.version != table.features_version || features.content_bytes == 0 {
        return None;
    }
    let labels = table.tier_labels();
    let (index, tier) = if labels.is_empty() {
        // One tier: no score is needed, and none is reported.
        (0, None)
    } else {
        let score = estimate_score(features, table)?;
        let index = table.cut_offs.iter().filter(|&&cut| score >= cut).count();
        (index, Some(*labels.get(index)?))
    };
    let band = table.bands.get(index)?;
    let low = (band.low / ESTIMATE_DISPLAY_STEP + ROUNDING_SLACK).floor() * ESTIMATE_DISPLAY_STEP;
    let high = (band.high / ESTIMATE_DISPLAY_STEP - ROUNDING_SLACK).ceil() * ESTIMATE_DISPLAY_STEP;
    if !(low.is_finite() && high.is_finite() && low > 0.0 && high >= low) {
        return None;
    }
    Some(LocalCreditEstimate {
        low,
        high,
        tier,
        calibration: table.calibration_label(),
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::Utc;
    use serde_json::json;
    use uuid::Uuid;

    use super::*;
    use crate::trace_contribution::{
        ConsentMetadata, ConsentScope, ContributorMetadata,
        DETERMINISTIC_REDACTION_PIPELINE_VERSION, IronclawTraceMetadata, OutcomeMetadata,
        PrivacyMetadata, ReplayMetadata, ResidualPiiRisk, SideEffectLevel,
        TRACE_CONTRIBUTION_POLICY_VERSION, TRACE_CONTRIBUTION_SCHEMA_VERSION, TraceCard,
        TraceChannel, TraceContributionEvent, TraceValueCard, ValueMetadata,
    };

    fn event(
        event_type: TraceContributionEventType,
        content: Option<&str>,
        tool_name: Option<&str>,
    ) -> TraceContributionEvent {
        TraceContributionEvent {
            event_id: Uuid::new_v4(),
            parent_event_id: None,
            event_type,
            timestamp: Utc::now(),
            redacted_content: content.map(str::to_string),
            structured_payload: Value::Null,
            tool_name: tool_name.map(str::to_string),
            tool_category: None,
            tool_call_id: None,
            latency_ms: None,
            token_counts: None,
            cost_usd: None,
            success: None,
            failure_modes: Vec::new(),
            side_effect: SideEffectLevel::None,
        }
    }

    fn envelope(events: Vec<TraceContributionEvent>) -> TraceContributionEnvelope {
        let now = Utc::now();
        TraceContributionEnvelope {
            schema_version: TRACE_CONTRIBUTION_SCHEMA_VERSION.to_string(),
            trace_id: Uuid::new_v4(),
            submission_id: Uuid::new_v4(),
            created_at: now,
            ironclaw: IronclawTraceMetadata {
                version: "1".to_string(),
                engine_version: None,
                feature_flags: BTreeMap::new(),
                channel: TraceChannel::Cli,
                model_name: None,
            },
            consent: ConsentMetadata {
                policy_version: TRACE_CONTRIBUTION_POLICY_VERSION.to_string(),
                scopes: vec![ConsentScope::DebuggingEvaluation],
                message_text_included: true,
                tool_payloads_included: false,
                correction_included: false,
                routing_metadata_included: false,
                revocable: true,
            },
            contributor: ContributorMetadata {
                pseudonymous_contributor_id: None,
                tenant_scope_ref: None,
                credit_account_ref: None,
                revocation_handle: Uuid::new_v4(),
            },
            privacy: PrivacyMetadata {
                redaction_pipeline_version: DETERMINISTIC_REDACTION_PIPELINE_VERSION.to_string(),
                redaction_counts: BTreeMap::new(),
                redaction_distinct_counts: BTreeMap::new(),
                privacy_filter_summary: None,
                pii_labels_present: Vec::new(),
                residual_pii_risk: ResidualPiiRisk::Low,
                redaction_hash: "sha256:placeholder".to_string(),
                warnings: Vec::new(),
            },
            events,
            outcome: OutcomeMetadata::default(),
            replay: ReplayMetadata {
                replayable: false,
                required_tools: Vec::new(),
                tool_manifest_hashes: BTreeMap::new(),
                expected_assertions: Vec::new(),
                replay_notes: Vec::new(),
            },
            embedding_analysis: None,
            value: ValueMetadata::default(),
            conversation_id: None,
            source_session: None,
            trace_card: TraceCard::default(),
            value_card: TraceValueCard::default(),
            hindsight: None,
            training_dynamics: None,
            process_evaluation: None,
        }
    }

    fn features_of(events: &[(EstimateRole, Option<&str>, Option<&str>)]) -> LocalEstimateFeatures {
        let mut acc = LocalEstimateAccumulator::new();
        for (role, text, tool) in events {
            acc.accumulate(*role, *text, *tool);
        }
        acc.finish()
    }

    fn some_features() -> LocalEstimateFeatures {
        features_of(&[
            (
                EstimateRole::User,
                Some("fix the failing build please"),
                None,
            ),
            (
                EstimateRole::Assistant,
                Some("Looking at the build log now."),
                None,
            ),
            (EstimateRole::Other, Some("cargo build"), Some("bash")),
            (
                EstimateRole::ToolResult,
                Some("error[E0308]: mismatched types"),
                Some("bash"),
            ),
        ])
    }

    fn three_tier_table_value() -> Value {
        json!({
            "schema_version": 1,
            "features_version": "lef1",
            "version": "t7",
            "credit_quality_calibration": "cq3",
            "bytes_per_token": 4,
            "chunk_target_tokens": 2048,
            "chunk_cap": 16,
            "weights": [
                {"term": "ln_content_bytes", "weight": 1.0},
                {"term": "user_messages_capped", "weight": 0.5}
            ],
            "cut_offs": [4.0, 6.0],
            "bands": [
                {"low": 0.9, "high": 1.9},
                {"low": 1.3, "high": 2.4},
                {"low": 1.8, "high": 3.1}
            ],
            "withheld_share": 0.12
        })
    }

    // -- features -----------------------------------------------------------

    #[test]
    fn accumulate_counts_bytes_by_role() {
        let f = features_of(&[
            (EstimateRole::User, Some("abcd"), None),
            (EstimateRole::Assistant, Some("efghij"), None),
            (EstimateRole::Other, Some("kl"), Some("bash")),
            (EstimateRole::ToolResult, Some("mnopqrst"), Some("bash")),
            (EstimateRole::Other, Some("uv"), Some("read")),
        ]);
        assert_eq!(f.version, LOCAL_ESTIMATE_FEATURES_VERSION);
        assert_eq!(f.content_bytes, 22);
        assert_eq!(f.agent_prose_bytes, 6);
        assert_eq!(f.tool_result_bytes, 8);
        assert_eq!(f.user_messages, 1);
        assert_eq!(f.distinct_tools, 2);
        assert!(f.byte_entropy_milli.is_some());
        let share = f.tool_result_share().expect("content present");
        assert!((share - 8.0 / 22.0).abs() < 1e-12);
        let prose = f.agent_prose_share().expect("content present");
        assert!((prose - 6.0 / 22.0).abs() < 1e-12);
    }

    #[test]
    fn reasoning_counts_as_other_not_agent_prose() {
        let mut env = envelope(vec![
            event(
                TraceContributionEventType::Reasoning,
                Some("thinking hard"),
                None,
            ),
            event(
                TraceContributionEventType::AssistantMessage,
                Some("done"),
                None,
            ),
        ]);
        let f = LocalEstimateFeatures::from_envelope(&env);
        assert_eq!(f.content_bytes, 17);
        assert_eq!(f.agent_prose_bytes, 4);
        env.events.clear();
        assert_eq!(LocalEstimateFeatures::from_envelope(&env).content_bytes, 0);
    }

    #[test]
    fn user_messages_need_non_empty_text() {
        let f = features_of(&[
            (EstimateRole::User, Some("hello"), None),
            (EstimateRole::User, Some("   \n"), None),
            (EstimateRole::User, None, None),
            (EstimateRole::User, Some("again"), None),
        ]);
        assert_eq!(f.user_messages, 2);
    }

    #[test]
    fn routing_rows_are_excluded_from_every_feature() {
        let with_routing = LocalEstimateFeatures::from_envelope(&envelope(vec![
            event(TraceContributionEventType::UserMessage, Some("aaaa"), None),
            event(
                TraceContributionEventType::RoutingDecision,
                Some("ZZZZZZZZZZZZZZZZZZZZ"),
                Some("router"),
            ),
        ]));
        let without = LocalEstimateFeatures::from_envelope(&envelope(vec![event(
            TraceContributionEventType::UserMessage,
            Some("aaaa"),
            None,
        )]));
        assert_eq!(with_routing, without);
        assert_eq!(with_routing.content_bytes, 4);
        assert_eq!(with_routing.distinct_tools, 0);
        // All-'a' content has zero entropy; the routing text would raise it.
        assert_eq!(with_routing.byte_entropy_milli, Some(0));
    }

    #[test]
    fn from_envelope_matches_the_accumulator_the_daemon_uses() {
        let env = envelope(vec![
            event(
                TraceContributionEventType::UserMessage,
                Some("fix the failing build please"),
                None,
            ),
            event(
                TraceContributionEventType::AssistantMessage,
                Some("Looking at the build log now."),
                None,
            ),
            event(
                TraceContributionEventType::ToolCall,
                Some("cargo build"),
                Some("bash"),
            ),
            event(
                TraceContributionEventType::ToolResult,
                Some("error[E0308]: mismatched types"),
                Some("bash"),
            ),
        ]);
        assert_eq!(LocalEstimateFeatures::from_envelope(&env), some_features());
    }

    #[test]
    fn empty_content_has_unknown_entropy_and_no_shares() {
        let f = features_of(&[(EstimateRole::User, None, None)]);
        assert_eq!(f.content_bytes, 0);
        assert_eq!(f.byte_entropy_milli, None);
        assert_eq!(f.tool_result_share(), None);
        assert_eq!(f.agent_prose_share(), None);
    }

    #[test]
    fn entropy_of_two_equiprobable_bytes_is_one_bit() {
        let f = features_of(&[(EstimateRole::User, Some("abababab"), None)]);
        assert_eq!(f.byte_entropy_milli, Some(1000));
    }

    #[test]
    fn entropy_sample_reads_everything_under_the_bound() {
        let a = "x".repeat(1000);
        let b = "y".repeat(500);
        let (hist, read) = sample_histogram(&[a.as_str(), b.as_str()], 1500);
        assert_eq!(read, 1500);
        assert_eq!(hist[usize::from(b'x')], 1000);
        assert_eq!(hist[usize::from(b'y')], 500);
    }

    #[test]
    fn entropy_reads_at_most_the_sample_bound_on_a_64_mib_input() {
        let mut big = vec![b'a'; 64 * 1024 * 1024];
        // A byte only at the very end: the endpoint-inclusive stride must
        // reach it.
        let last = big.len() - 1;
        big[last] = b'z';
        let big = String::from_utf8(big).expect("ascii");
        // Split across segments so windows straddle a boundary.
        let (left, right) = big.split_at(big.len() / 3 + 7);
        let (hist, read) = sample_histogram(&[left, right], big.len());
        assert_eq!(read, ESTIMATE_SAMPLE_BYTES);
        assert_eq!(hist.iter().sum::<u64>(), ESTIMATE_SAMPLE_BYTES as u64);
        assert_eq!(
            hist[usize::from(b'z')],
            1,
            "the final window reaches the end"
        );

        let f = features_of(&[
            (EstimateRole::User, Some(left), None),
            (EstimateRole::ToolResult, Some(right), None),
        ]);
        assert_eq!(f.content_bytes, big.len() as u64);
        assert!(f.byte_entropy_milli.is_some());
    }

    #[test]
    fn entropy_sample_starts_at_the_opening_and_spans_the_length() {
        // A distinct byte at the very start and one in the middle region.
        let total = ESTIMATE_SAMPLE_BYTES * 8;
        let mut bytes = vec![b'a'; total];
        bytes[0] = b'b';
        let s = String::from_utf8(bytes).expect("ascii");
        let (hist, read) = sample_histogram(&[s.as_str()], total);
        assert_eq!(read, ESTIMATE_SAMPLE_BYTES);
        assert_eq!(hist[usize::from(b'b')], 1);
    }

    #[test]
    fn chunk_estimate_uses_the_table_parameters() {
        let table = LocalEstimateTable::built_in();
        let per_chunk = u64::from(table.bytes_per_token) * u64::from(table.chunk_target_tokens);
        let mut f = some_features();
        f.content_bytes = per_chunk;
        assert_eq!(f.est_chunks(&table), 1);
        assert!(!f.capped(&table));
        f.content_bytes = per_chunk + 1;
        assert_eq!(f.est_chunks(&table), 2);
        f.content_bytes = per_chunk * u64::from(table.chunk_cap) + 1;
        assert_eq!(f.est_chunks(&table), u64::from(table.chunk_cap) + 1);
        assert!(f.capped(&table));
    }

    #[test]
    fn features_round_trip_through_json() {
        let f = some_features();
        let back: LocalEstimateFeatures =
            serde_json::from_value(serde_json::to_value(&f).unwrap()).unwrap();
        assert_eq!(back, f);
    }

    #[test]
    fn features_are_deterministic() {
        assert_eq!(some_features(), some_features());
        let a = serde_json::to_string(&some_features()).unwrap();
        let b = serde_json::to_string(&some_features()).unwrap();
        assert_eq!(a, b);
    }

    // -- estimate -----------------------------------------------------------

    #[test]
    fn built_in_table_gives_about_one_to_three_with_no_tier() {
        let table = LocalEstimateTable::built_in();
        assert_eq!(table.validate(), Ok(()));
        assert_eq!(table.tier_count(), 1);
        assert_eq!(table.calibration_label(), "lef1.t1/cq3");
        let e = estimate(&some_features(), &table).expect("built-in table gives a band");
        assert_eq!(e.low, 1.0);
        assert_eq!(e.high, 3.0);
        assert_eq!(e.tier, None);
        assert_eq!(e.calibration, "lef1.t1/cq3");
        let wire = serde_json::to_value(&e).unwrap();
        assert!(wire.get("tier").is_none(), "one-tier table omits tier");
    }

    #[test]
    fn built_in_table_round_trips_through_from_value() {
        let table = LocalEstimateTable::built_in();
        let value = serde_json::to_value(&table).unwrap();
        assert_eq!(LocalEstimateTable::from_value(&value), Ok(table));
    }

    #[test]
    fn no_content_gives_none_never_zero() {
        let f = features_of(&[(EstimateRole::User, None, None)]);
        assert_eq!(estimate(&f, &LocalEstimateTable::built_in()), None);
    }

    #[test]
    fn other_feature_version_gives_none() {
        let mut f = some_features();
        f.version = "lef0".to_string();
        assert_eq!(estimate(&f, &LocalEstimateTable::built_in()), None);
    }

    #[test]
    fn unknown_weighted_feature_gives_none() {
        let mut table = LocalEstimateTable::from_value(&three_tier_table_value()).unwrap();
        table.weights.push(EstimateWeight {
            term: EstimateTerm::ByteEntropy,
            weight: 1.0,
        });
        let mut f = some_features();
        f.byte_entropy_milli = None;
        assert_eq!(estimate(&f, &table), None);
    }

    #[test]
    fn a_band_that_would_display_as_zero_gives_none() {
        let mut table = LocalEstimateTable::built_in();
        table.bands[0].low = 0.2;
        assert_eq!(estimate(&some_features(), &table), None);
    }

    #[test]
    fn estimate_is_never_zero_across_many_shapes() {
        let table = LocalEstimateTable::from_value(&three_tier_table_value()).unwrap();
        for n in 0..200usize {
            let text = "q".repeat(n * 37 + 1);
            let users = n % 25;
            let mut acc = LocalEstimateAccumulator::new();
            for _ in 0..users {
                acc.accumulate(EstimateRole::User, Some("ok"), None);
            }
            acc.accumulate(EstimateRole::ToolResult, Some(text.as_str()), Some("t"));
            let f = acc.finish();
            for t in [&table, &LocalEstimateTable::built_in()] {
                let e = estimate(&f, t).expect("content present");
                assert!(e.low > 0.0, "low {} at n {n}", e.low);
                assert!(e.high >= e.low);
            }
        }
    }

    #[test]
    fn three_tier_table_places_tiers_by_cut_off() {
        let table = LocalEstimateTable::from_value(&three_tier_table_value()).unwrap();
        assert_eq!(table.calibration_label(), "lef1.t7/cq3");
        // score = ln(1 + bytes) + 0.5 * min(users, 20)
        let small = features_of(&[(EstimateRole::ToolResult, Some("abc"), None)]); // ~1.39
        let middle = features_of(&[(EstimateRole::ToolResult, Some(&"a".repeat(200)), None)]); // ~5.3
        let large = features_of(&[(EstimateRole::ToolResult, Some(&"a".repeat(2000)), None)]); // ~7.6
        let lo = estimate(&small, &table).unwrap();
        let mid = estimate(&middle, &table).unwrap();
        let hi = estimate(&large, &table).unwrap();
        assert_eq!(lo.tier, Some(EstimateTier::Lower));
        assert_eq!((lo.low, lo.high), (0.5, 2.0));
        assert_eq!(mid.tier, Some(EstimateTier::Middle));
        assert_eq!((mid.low, mid.high), (1.0, 2.5));
        assert_eq!(hi.tier, Some(EstimateTier::Higher));
        assert_eq!((hi.low, hi.high), (1.5, 3.5));
    }

    #[test]
    fn estimate_score_is_the_weighted_sum_the_tiers_are_cut_on() {
        let table = LocalEstimateTable::from_value(&three_tier_table_value()).unwrap();
        let f = features_of(&[
            (EstimateRole::User, Some("a"), None),
            (EstimateRole::ToolResult, Some(&"a".repeat(200)), None),
        ]);
        let expected = (f.content_bytes as f64).ln_1p() + 0.5;
        let score = estimate_score(&f, &table).unwrap();
        assert!((score - expected).abs() < 1e-12, "{score} vs {expected}");
        // The same per-term values the server fits against.
        assert_eq!(
            f.term_value(EstimateTerm::UserMessagesCapped, &table),
            Some(1.0)
        );
        // A weighted term whose feature is unknown leaves no score, never 0.
        let empty = features_of(&[]);
        let mut shares = table.clone();
        shares.weights = vec![EstimateWeight {
            term: EstimateTerm::ToolResultShare,
            weight: 1.0,
        }];
        assert_eq!(estimate_score(&empty, &shares), None);
    }

    #[test]
    fn a_score_exactly_on_a_cut_off_goes_to_the_tier_above() {
        let mut table = LocalEstimateTable::from_value(&three_tier_table_value()).unwrap();
        table.weights = vec![EstimateWeight {
            term: EstimateTerm::UserMessagesCapped,
            weight: 1.0,
        }];
        table.cut_offs = vec![2.0, 30.0];
        let f = features_of(&[
            (EstimateRole::User, Some("a"), None),
            (EstimateRole::User, Some("b"), None),
        ]);
        assert_eq!(
            estimate(&f, &table).unwrap().tier,
            Some(EstimateTier::Middle)
        );
    }

    #[test]
    fn user_messages_are_capped_in_the_score() {
        let mut table = LocalEstimateTable::from_value(&three_tier_table_value()).unwrap();
        table.weights = vec![EstimateWeight {
            term: EstimateTerm::UserMessagesCapped,
            weight: 1.0,
        }];
        table.cut_offs = vec![10.0, f64::from(ESTIMATE_USER_MESSAGES_CAP) + 0.5];
        let mut f = some_features();
        f.user_messages = 500;
        assert_eq!(
            estimate(&f, &table).unwrap().tier,
            Some(EstimateTier::Middle)
        );
    }

    #[test]
    fn two_tier_table_uses_lower_and_higher() {
        let mut v = three_tier_table_value();
        v["cut_offs"] = json!([4.0]);
        v["bands"] = json!([{"low": 1.0, "high": 2.0}, {"low": 1.5, "high": 3.0}]);
        let table = LocalEstimateTable::from_value(&v).unwrap();
        let small = features_of(&[(EstimateRole::ToolResult, Some("abc"), None)]);
        let large = features_of(&[(EstimateRole::ToolResult, Some(&"a".repeat(2000)), None)]);
        assert_eq!(
            estimate(&small, &table).unwrap().tier,
            Some(EstimateTier::Lower)
        );
        assert_eq!(
            estimate(&large, &table).unwrap().tier,
            Some(EstimateTier::Higher)
        );
    }

    #[test]
    fn estimate_is_deterministic() {
        let table = LocalEstimateTable::from_value(&three_tier_table_value()).unwrap();
        let f = some_features();
        assert_eq!(estimate(&f, &table), estimate(&f, &table));
    }

    #[test]
    fn display_rounding_goes_outward_and_tolerates_float_noise() {
        let mut table = LocalEstimateTable::built_in();
        table.bands[0] = EstimateBand {
            low: 1.0000000000000002,
            high: 2.9999999999999996,
        };
        let e = estimate(&some_features(), &table).unwrap();
        assert_eq!((e.low, e.high), (1.0, 3.0));
        table.bands[0] = EstimateBand {
            low: 0.1 * 30.0,
            high: 0.1 * 30.0,
        };
        let e = estimate(&some_features(), &table).unwrap();
        assert_eq!((e.low, e.high), (3.0, 3.0));
    }

    // -- refusals -----------------------------------------------------------

    fn refused(edit: impl FnOnce(&mut Value)) -> EstimateTableError {
        let mut v = three_tier_table_value();
        edit(&mut v);
        LocalEstimateTable::from_value(&v).expect_err("table must be refused")
    }

    #[test]
    fn the_seeded_table_is_accepted() {
        let table = LocalEstimateTable::from_value(&three_tier_table_value()).unwrap();
        assert_eq!(table.tier_count(), 3);
        assert_eq!(table.withheld_share, Some(0.12));
    }

    #[test]
    fn refuses_a_newer_schema() {
        assert_eq!(
            refused(|v| v["schema_version"] = json!(ESTIMATE_TABLE_SCHEMA_VERSION + 1)),
            EstimateTableError::UnsupportedSchema
        );
    }

    #[test]
    fn refuses_other_feature_definitions() {
        assert_eq!(
            refused(|v| v["features_version"] = json!("lef2")),
            EstimateTableError::FeaturesVersion
        );
    }

    #[test]
    fn refuses_too_many_tiers() {
        assert_eq!(
            refused(|v| {
                v["cut_offs"] = json!([1.0, 2.0, 3.0]);
                v["bands"] = json!([
                    {"low": 1.0, "high": 2.0},
                    {"low": 1.0, "high": 2.0},
                    {"low": 1.0, "high": 2.0},
                    {"low": 1.0, "high": 2.0}
                ]);
            }),
            EstimateTableError::Tiers
        );
        assert_eq!(
            refused(|v| v["bands"] = json!([])),
            EstimateTableError::Tiers
        );
    }

    #[test]
    fn refuses_mismatched_or_unordered_cut_offs() {
        assert_eq!(
            refused(|v| v["cut_offs"] = json!([4.0])),
            EstimateTableError::CutOffs
        );
        assert_eq!(
            refused(|v| v["cut_offs"] = json!([6.0, 4.0])),
            EstimateTableError::CutOffs
        );
        assert_eq!(
            refused(|v| v["cut_offs"] = json!([4.0, 4.0])),
            EstimateTableError::CutOffs
        );
    }

    #[test]
    fn refuses_too_many_or_repeated_weights() {
        assert_eq!(
            refused(|v| {
                let many: Vec<Value> = (0..=ESTIMATE_MAX_WEIGHTS)
                    .map(|_| json!({"term": "capped", "weight": 1.0}))
                    .collect();
                v["weights"] = Value::Array(many);
            }),
            EstimateTableError::Weights
        );
        assert_eq!(
            refused(|v| v["weights"] = json!([
                {"term": "capped", "weight": 1.0},
                {"term": "capped", "weight": 2.0}
            ])),
            EstimateTableError::Weights
        );
    }

    #[test]
    fn refuses_an_unknown_term_or_field() {
        assert_eq!(
            refused(|v| v["weights"] = json!([{"term": "perplexity", "weight": 1.0}])),
            EstimateTableError::Malformed
        );
        assert_eq!(
            refused(|v| v["extra"] = json!(1)),
            EstimateTableError::Malformed
        );
        assert_eq!(
            LocalEstimateTable::from_value(&json!("not a table")),
            Err(EstimateTableError::Malformed)
        );
    }

    #[test]
    fn refuses_out_of_range_numbers() {
        assert_eq!(
            refused(|v| v["weights"][0]["weight"] = json!(ESTIMATE_MAX_ABS_WEIGHT * 2.0)),
            EstimateTableError::OutOfRange
        );
        assert_eq!(
            refused(|v| v["bands"][0]["low"] = json!(0.2)),
            EstimateTableError::OutOfRange,
            "a band low that would display as 0 is refused"
        );
        assert_eq!(
            refused(|v| v["bands"][2]["high"] = json!(ESTIMATE_MAX_DISPLAYED_CREDIT + 1.0)),
            EstimateTableError::OutOfRange
        );
        assert_eq!(
            refused(|v| v["bands"][1] = json!({"low": 2.5, "high": 2.0})),
            EstimateTableError::OutOfRange
        );
        assert_eq!(
            refused(|v| v["withheld_share"] = json!(1.5)),
            EstimateTableError::OutOfRange
        );
    }

    #[test]
    fn refuses_non_finite_numbers() {
        let base = LocalEstimateTable::from_value(&three_tier_table_value()).unwrap();
        for poison in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let mut t = base.clone();
            t.weights[0].weight = poison;
            assert_eq!(t.validate(), Err(EstimateTableError::OutOfRange));
            let mut t = base.clone();
            t.cut_offs[0] = poison;
            assert!(t.validate().is_err());
            let mut t = base.clone();
            t.bands[0].high = poison;
            assert_eq!(t.validate(), Err(EstimateTableError::OutOfRange));
            let mut t = base.clone();
            t.withheld_share = Some(poison);
            assert_eq!(t.validate(), Err(EstimateTableError::OutOfRange));
        }
    }

    #[test]
    fn refuses_bad_labels_and_chunk_parameters() {
        assert_eq!(
            refused(|v| v["version"] = json!("")),
            EstimateTableError::Label
        );
        assert_eq!(
            refused(|v| v["version"] = json!("t1/../x")),
            EstimateTableError::Label
        );
        assert_eq!(
            refused(
                |v| v["credit_quality_calibration"] = json!("c".repeat(ESTIMATE_MAX_LABEL_LEN + 1))
            ),
            EstimateTableError::Label
        );
        assert_eq!(
            refused(|v| v["bytes_per_token"] = json!(0)),
            EstimateTableError::ChunkParameters
        );
        assert_eq!(
            refused(|v| v["chunk_cap"] = json!(0)),
            EstimateTableError::ChunkParameters
        );
    }
}
