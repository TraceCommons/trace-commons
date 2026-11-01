// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The NEAR AI starter advance's plausibility check, `starter-plausibility-v1`.
//! See `docs/superpowers/specs/2026-10-21-nearai-starter-advance-design.md`,
//! section 3.
//!
//! Two pure steps, no database and no clock:
//!
//! 1. [`extract_features`] reduces one accepted envelope to a hash-only
//!    [`StarterEnvelopeFeatures`] record. It keeps counts, coarse
//!    hour-truncated times, bucket masks and `sha256:` hashes. It never keeps
//!    trace text, a tool argument, a model name or a session id.
//! 2. [`evaluate`] folds an account's records and applies the rules in order.
//!    The first rule that is not met is reported; there is no weighted score.
//!
//! Every input is declared by the client. The envelope marks
//! `source_session` as attribution only; this check reads it for one capped
//! decision that pays no Trace Credit (spec section 3, "Self-declared
//! input"). Nothing here feeds a gate, a score, Trace Credit or tenant
//! scoping. The rules are a speed bump; the identity and money caps in spec
//! section 4 bound the loss.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use chrono::{DateTime, Duration, DurationRound, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use trace_commons_protocol::trace_contribution::{
    TraceContributionEnvelope, TraceContributionEventType,
};

/// The rule set this module implements.
pub const STARTER_PLAUSIBILITY_RULE_VERSION: &str = "starter-plausibility-v1";
/// The shape of a stored [`StarterEnvelopeFeatures`] record.
pub const STARTER_FEATURES_VERSION: &str = "starter-features-v1";

/// Envelopes one evaluation reads at most. An account past this has more
/// than enough uploads; the oldest beyond it are ignored.
pub const MAX_EVALUATED_ENVELOPES: usize = 2_000;

/// A turn gap at least this long counts as human think time.
const THINK_TIME_SECONDS: i64 = 20;

/// Which agent the client says produced the trace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StarterAgent {
    ClaudeCode,
    Codex,
    /// A supported adapter outside the starter scope (OpenCode, Cline,
    /// Gemini CLI, ...). Not counted.
    Other,
    /// No `source_session`, or one the protocol refuses. Not counted.
    None,
}

impl StarterAgent {
    fn from_adapter(adapter: Option<&str>) -> Self {
        match adapter {
            Some("claude-code") => Self::ClaudeCode,
            Some("codex") => Self::Codex,
            Some(_) => Self::Other,
            None => Self::None,
        }
    }

    fn in_scope(self) -> bool {
        matches!(self, Self::ClaudeCode | Self::Codex)
    }

    fn expected_model_family(self) -> Option<ModelFamily> {
        match self {
            Self::ClaudeCode => Some(ModelFamily::Anthropic),
            Self::Codex => Some(ModelFamily::OpenAi),
            Self::Other | Self::None => None,
        }
    }

    /// The agent's own tool names. MCP tools (`mcp__...`) count for both.
    fn knows_tool(self, name: &str) -> bool {
        if name.starts_with("mcp__") || name.starts_with("mcp.") {
            return true;
        }
        let known: &[&str] = match self {
            Self::ClaudeCode => &[
                "Read",
                "Write",
                "Edit",
                "MultiEdit",
                "Bash",
                "BashOutput",
                "KillShell",
                "KillBash",
                "Glob",
                "Grep",
                "LS",
                "Task",
                "TodoWrite",
                "TodoRead",
                "WebFetch",
                "WebSearch",
                "NotebookEdit",
                "NotebookRead",
                "ExitPlanMode",
                "Skill",
                "SlashCommand",
                "AskUserQuestion",
            ],
            Self::Codex => &[
                "shell",
                "local_shell",
                "container.exec",
                "exec_command",
                "write_stdin",
                "apply_patch",
                "update_plan",
                "view_image",
                "web_search",
                "read_file",
                "list_dir",
                "grep_files",
                "unified_exec",
            ],
            Self::Other | Self::None => &[],
        };
        known.contains(&name)
    }
}

/// The provider family a model name belongs to. The name itself is never
/// stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelFamily {
    Anthropic,
    OpenAi,
    Other,
    None,
}

impl ModelFamily {
    fn of(model: Option<&str>) -> Self {
        let Some(model) = model.map(str::trim).filter(|m| !m.is_empty()) else {
            return Self::None;
        };
        let lower = model.to_ascii_lowercase();
        let bare = lower.rsplit('/').next().unwrap_or(&lower);
        if bare.starts_with("claude") || lower.starts_with("anthropic") {
            Self::Anthropic
        } else if bare.starts_with("gpt-")
            || bare.starts_with("o1")
            || bare.starts_with("o3")
            || bare.starts_with("o4")
            || bare.starts_with("codex")
            || lower.starts_with("openai")
        {
            Self::OpenAi
        } else {
            Self::Other
        }
    }
}

/// One accepted envelope, reduced to what the rules read. Hash-only: no
/// trace text, tool argument, model name, session id or exact timestamp.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StarterEnvelopeFeatures {
    pub version: String,
    pub agent: StarterAgent,
    /// `sha256:` of the adapter and its native session id; `None` without a
    /// `source_session`.
    pub session_hash: Option<String>,
    /// `sha256:` over the events' types, tool names and redacted content:
    /// two envelopes with the same value carry the same trace.
    pub content_hash: String,
    /// Earliest and latest event time, truncated to the hour.
    pub first_event_hour: Option<DateTime<Utc>>,
    pub last_event_hour: Option<DateTime<Utc>>,
    /// Some event is later than the moment the server received the envelope
    /// (with an hour of clock skew allowed).
    pub future_dated: bool,
    pub model_family: ModelFamily,
    pub tool_calls: u32,
    pub tool_results: u32,
    /// Tool calls answered by a result: matched by `tool_call_id` when the
    /// envelope carries ids, otherwise `min(calls, results)`.
    pub paired_tool_calls: u32,
    /// Tool results that report failure.
    pub failed_tool_results: u32,
    /// Tool calls whose name is in the agent's own tool set.
    pub known_tool_calls: u32,
    /// Gaps between consecutive message events (user, assistant).
    pub turn_gaps: u32,
    /// Of those, gaps of at least `THINK_TIME_SECONDS`.
    pub think_time_gaps: u32,
    /// Bit `n` set when some turn gap fell in `[2^n, 2^(n+1))` seconds
    /// (bit 0 also holds gaps under a second).
    pub gap_bucket_mask: u32,
    /// The same, over tool-call latencies in milliseconds.
    pub latency_bucket_mask: u32,
    /// Assistant message events, and those of them that report token counts.
    pub assistant_events: u32,
    pub token_events: u32,
    /// Consecutive token-reporting events, and how many of those steps had
    /// input tokens at least as large as the step before (context growth).
    pub input_token_steps: u32,
    pub input_token_rises: u32,
    /// Events that carry a list-price cost.
    pub cost_events: u32,
}

/// Reduces one envelope to its features. `received_at` is when the server
/// received it, used only to flag events dated in the future.
pub fn extract_features(
    envelope: &TraceContributionEnvelope,
    received_at: DateTime<Utc>,
) -> StarterEnvelopeFeatures {
    // Only a source identity the protocol accepts counts; anything else is
    // treated as no source at all.
    let source = envelope.source_session.as_ref().filter(|source| {
        trace_commons_protocol::trace_contribution::validate_source_session_identity(source).is_ok()
    });
    let agent = StarterAgent::from_adapter(source.map(|source| source.adapter.as_str()));
    let session_hash = source.map(|source| {
        sha256_prefixed(&[
            b"trace-starter-session.v1\n",
            source.adapter.as_bytes(),
            b"\n",
            source.native_id.as_bytes(),
        ])
    });

    let mut content = Sha256::new();
    content.update(b"trace-starter-content.v1\n");
    let mut first: Option<DateTime<Utc>> = None;
    let mut last: Option<DateTime<Utc>> = None;
    let mut tool_calls = 0u32;
    let mut tool_results = 0u32;
    let mut failed_tool_results = 0u32;
    let mut known_tool_calls = 0u32;
    let mut call_ids: Vec<&str> = Vec::new();
    let mut result_ids: BTreeSet<&str> = BTreeSet::new();
    let mut message_times: Vec<DateTime<Utc>> = Vec::new();
    let mut latency_bucket_mask = 0u32;
    let mut assistant_events = 0u32;
    let mut token_events = 0u32;
    let mut input_token_steps = 0u32;
    let mut input_token_rises = 0u32;
    let mut previous_input: Option<u32> = None;
    let mut cost_events = 0u32;

    for event in &envelope.events {
        content.update(event_type_label(event.event_type).as_bytes());
        content.update(b"\x1f");
        content.update(event.tool_name.as_deref().unwrap_or("").as_bytes());
        content.update(b"\x1f");
        content.update(event.redacted_content.as_deref().unwrap_or("").as_bytes());
        content.update(b"\x1e");

        first = Some(first.map_or(event.timestamp, |f| f.min(event.timestamp)));
        last = Some(last.map_or(event.timestamp, |l| l.max(event.timestamp)));
        if event.cost_usd.is_some() {
            cost_events += 1;
        }
        if let Some(tokens) = &event.token_counts {
            token_events +=
                u32::from(event.event_type == TraceContributionEventType::AssistantMessage);
            if let Some(previous) = previous_input {
                input_token_steps += 1;
                if tokens.input_tokens >= previous {
                    input_token_rises += 1;
                }
            }
            previous_input = Some(tokens.input_tokens);
        }
        match event.event_type {
            TraceContributionEventType::ToolCall => {
                tool_calls += 1;
                if event
                    .tool_name
                    .as_deref()
                    .is_some_and(|name| agent.knows_tool(name))
                {
                    known_tool_calls += 1;
                }
                if let Some(id) = event.tool_call_id.as_deref() {
                    call_ids.push(id);
                }
                if let Some(latency) = event.latency_ms {
                    latency_bucket_mask |= bucket_bit(latency);
                }
            }
            TraceContributionEventType::ToolResult => {
                tool_results += 1;
                if event.success == Some(false) || !event.failure_modes.is_empty() {
                    failed_tool_results += 1;
                }
                if let Some(id) = event.tool_call_id.as_deref() {
                    result_ids.insert(id);
                }
            }
            TraceContributionEventType::UserMessage => message_times.push(event.timestamp),
            TraceContributionEventType::AssistantMessage => {
                assistant_events += 1;
                message_times.push(event.timestamp);
            }
            _ => {}
        }
    }

    // In an envelope that carries ids, a call without one cannot match and
    // counts as unpaired.
    let paired_tool_calls = if call_ids.is_empty() {
        tool_calls.min(tool_results)
    } else {
        call_ids
            .iter()
            .filter(|id| result_ids.contains(*id))
            .count() as u32
    };

    message_times.sort();
    let mut turn_gaps = 0u32;
    let mut think_time_gaps = 0u32;
    let mut gap_bucket_mask = 0u32;
    for pair in message_times.windows(2) {
        let seconds = (pair[1] - pair[0]).num_seconds().max(0);
        turn_gaps += 1;
        if seconds >= THINK_TIME_SECONDS {
            think_time_gaps += 1;
        }
        gap_bucket_mask |= bucket_bit(seconds as u64);
    }

    let future_dated = last.is_some_and(|l| l > received_at + Duration::hours(1));

    StarterEnvelopeFeatures {
        version: STARTER_FEATURES_VERSION.to_string(),
        agent,
        session_hash,
        content_hash: format!("sha256:{}", hex::encode(content.finalize())),
        first_event_hour: first.map(truncate_to_hour),
        last_event_hour: last.map(truncate_to_hour),
        future_dated,
        model_family: ModelFamily::of(envelope.ironclaw.model_name.as_deref()),
        tool_calls,
        tool_results,
        paired_tool_calls,
        failed_tool_results,
        known_tool_calls,
        turn_gaps,
        think_time_gaps,
        gap_bucket_mask,
        latency_bucket_mask,
        assistant_events,
        token_events,
        input_token_steps,
        input_token_rises,
        cost_events,
    }
}

/// The thresholds of one rule set. [`StarterPolicy::v1`] is
/// `starter-plausibility-v1`; the values are proposals to tune in dry run.
#[derive(Debug, Clone, PartialEq)]
pub struct StarterPolicy {
    pub min_sessions: usize,
    pub min_tool_calls: u64,
    pub min_distinct_days: usize,
    pub min_span_hours: i64,
    /// Envelopes whose last event is older than this, relative to the
    /// evaluation time, are not counted.
    pub max_age_days: i64,
    pub min_think_time_share: f64,
    pub min_gap_buckets: u32,
    /// Applies only when some tool call reports a latency.
    pub min_latency_buckets: u32,
    pub min_model_match_share: f64,
    pub min_known_tool_share: f64,
    pub min_paired_share: f64,
    pub min_failed_tool_results: u64,
    pub min_token_share: f64,
    pub min_input_rise_share: f64,
    pub min_cost_events: u64,
}

impl StarterPolicy {
    pub fn v1() -> Self {
        Self {
            min_sessions: 8,
            min_tool_calls: 150,
            min_distinct_days: 3,
            min_span_hours: 48,
            max_age_days: 180,
            min_think_time_share: 0.10,
            min_gap_buckets: 4,
            min_latency_buckets: 3,
            min_model_match_share: 0.90,
            min_known_tool_share: 0.60,
            min_paired_share: 0.95,
            min_failed_tool_results: 1,
            min_token_share: 0.50,
            min_input_rise_share: 0.50,
            min_cost_events: 1,
        }
    }

    pub fn version(&self) -> &'static str {
        STARTER_PLAUSIBILITY_RULE_VERSION
    }
}

/// The rules, in the order they are checked. The label is what reports and
/// logs carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StarterRule {
    SourceAgent,
    Volume,
    Span,
    Rhythm,
    AgentSignature,
    ToolPairing,
    Usage,
    NotCopied,
}

impl StarterRule {
    pub const ALL: [StarterRule; 8] = [
        Self::SourceAgent,
        Self::Volume,
        Self::Span,
        Self::Rhythm,
        Self::AgentSignature,
        Self::ToolPairing,
        Self::Usage,
        Self::NotCopied,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::SourceAgent => "source_agent",
            Self::Volume => "volume",
            Self::Span => "span",
            Self::Rhythm => "rhythm",
            Self::AgentSignature => "agent_signature",
            Self::ToolPairing => "tool_pairing",
            Self::Usage => "usage",
            Self::NotCopied => "not_copied",
        }
    }
}

/// The outcome of one evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "decision", content = "rule", rename_all = "snake_case")]
pub enum StarterDecision {
    /// Every rule passed: the account would get an advance.
    Qualified,
    /// The first rule not met. More uploads can still qualify the account,
    /// except for a future-dated envelope (`span`) or a copied one
    /// (`not_copied`), which stay counted until withdrawn.
    NotYet(StarterRule),
}

impl StarterDecision {
    pub fn label(self) -> &'static str {
        match self {
            Self::Qualified => "qualified",
            Self::NotYet(rule) => rule.label(),
        }
    }
}

/// Label-only totals over an account's counted envelopes, reported beside
/// the decision so a dry run shows how close an account came.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct StarterTotals {
    pub envelopes_counted: u64,
    pub sessions: u64,
    pub tool_calls: u64,
    pub distinct_days: u64,
    pub span_hours: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StarterEvaluation {
    pub rule_version: &'static str,
    pub decision: StarterDecision,
    pub totals: StarterTotals,
}

/// Applies `policy` to an account's envelope features at `now`. Envelopes
/// outside the starter scope (another agent, no source session, older than
/// the age limit, or an unknown feature version) are not counted.
pub fn evaluate(
    policy: &StarterPolicy,
    features: &[StarterEnvelopeFeatures],
    now: DateTime<Utc>,
) -> StarterEvaluation {
    let oldest = now - Duration::days(policy.max_age_days);
    let counted: Vec<&StarterEnvelopeFeatures> = features
        .iter()
        .take(MAX_EVALUATED_ENVELOPES)
        .filter(|f| f.version == STARTER_FEATURES_VERSION)
        .filter(|f| f.agent.in_scope() && f.session_hash.is_some())
        .filter(|f| f.last_event_hour.is_some_and(|last| last >= oldest))
        .collect();

    let sessions: BTreeSet<&str> = counted
        .iter()
        .filter_map(|f| f.session_hash.as_deref())
        .collect();
    let days: BTreeSet<chrono::NaiveDate> = counted
        .iter()
        .flat_map(|f| [f.first_event_hour, f.last_event_hour])
        .flatten()
        .map(|hour| hour.date_naive())
        .collect();
    let first = counted.iter().filter_map(|f| f.first_event_hour).min();
    let last = counted.iter().filter_map(|f| f.last_event_hour).max();
    let span_hours = match (first, last) {
        (Some(first), Some(last)) => (last - first).num_hours(),
        _ => 0,
    };
    let sum = |field: fn(&StarterEnvelopeFeatures) -> u32| -> u64 {
        counted.iter().map(|f| u64::from(field(f))).sum()
    };
    let tool_calls = sum(|f| f.tool_calls);
    let totals = StarterTotals {
        envelopes_counted: counted.len() as u64,
        sessions: sessions.len() as u64,
        tool_calls,
        distinct_days: days.len() as u64,
        span_hours,
    };
    let decide = |decision| StarterEvaluation {
        rule_version: policy.version(),
        decision,
        totals: totals.clone(),
    };
    let not_yet = |rule| decide(StarterDecision::NotYet(rule));

    // Source agent: at least one counted Claude Code or Codex envelope.
    if counted.is_empty() {
        return not_yet(StarterRule::SourceAgent);
    }

    // Volume.
    if sessions.len() < policy.min_sessions || tool_calls < policy.min_tool_calls {
        return not_yet(StarterRule::Volume);
    }

    // Span: a few days, nothing dated in the future.
    if counted.iter().any(|f| f.future_dated)
        || days.len() < policy.min_distinct_days
        || span_hours < policy.min_span_hours
    {
        return not_yet(StarterRule::Span);
    }

    // Rhythm: irregular turn gaps with some human think time; tool latencies
    // not all alike when any are reported.
    let turn_gaps = sum(|f| f.turn_gaps);
    let think_time_gaps = sum(|f| f.think_time_gaps);
    let gap_buckets = counted
        .iter()
        .fold(0u32, |mask, f| mask | f.gap_bucket_mask)
        .count_ones();
    let latency_mask = counted
        .iter()
        .fold(0u32, |mask, f| mask | f.latency_bucket_mask);
    if !share_at_least(think_time_gaps, turn_gaps, policy.min_think_time_share)
        || gap_buckets < policy.min_gap_buckets
        || (latency_mask != 0 && latency_mask.count_ones() < policy.min_latency_buckets)
    {
        return not_yet(StarterRule::Rhythm);
    }

    // Agent signature: model family matches the adapter where a model is
    // named, and tool names fit the agent's tool set.
    let with_model: Vec<&&StarterEnvelopeFeatures> = counted
        .iter()
        .filter(|f| f.model_family != ModelFamily::None)
        .collect();
    let model_matches = with_model
        .iter()
        .filter(|f| f.agent.expected_model_family() == Some(f.model_family))
        .count() as u64;
    let models_match = with_model.is_empty()
        || share_at_least(
            model_matches,
            with_model.len() as u64,
            policy.min_model_match_share,
        );
    if !models_match
        || !share_at_least(
            sum(|f| f.known_tool_calls),
            tool_calls,
            policy.min_known_tool_share,
        )
    {
        return not_yet(StarterRule::AgentSignature);
    }

    // Tool pairing: calls are answered, and some fail.
    if !share_at_least(
        sum(|f| f.paired_tool_calls),
        tool_calls,
        policy.min_paired_share,
    ) || sum(|f| f.failed_tool_results) < policy.min_failed_tool_results
    {
        return not_yet(StarterRule::ToolPairing);
    }

    // Usage: token counts reported and context growing; some list price.
    if !share_at_least(
        sum(|f| f.token_events),
        sum(|f| f.assistant_events),
        policy.min_token_share,
    ) || !share_at_least(
        sum(|f| f.input_token_rises),
        sum(|f| f.input_token_steps),
        policy.min_input_rise_share,
    ) || sum(|f| f.cost_events) < policy.min_cost_events
    {
        return not_yet(StarterRule::Usage);
    }

    // Not copied: no envelope repeats another's content. Cross-account
    // duplicates are checked when the advance is sent (spec section 4).
    let mut seen: HashMap<&str, u32> = HashMap::new();
    for f in &counted {
        *seen.entry(f.content_hash.as_str()).or_default() += 1;
    }
    if seen.values().any(|count| *count > 1) {
        return not_yet(StarterRule::NotCopied);
    }

    decide(StarterDecision::Qualified)
}

/// Label-only counts from a dry-run report over many accounts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct StarterDryRunReport {
    pub rule_version: &'static str,
    pub accounts_scanned: u64,
    /// Accounts with at least one stored feature record.
    pub accounts_with_features: u64,
    pub would_grant: u64,
    /// Accounts with features that did not qualify, by the first rule not met.
    pub not_yet_by_rule: BTreeMap<&'static str, u64>,
    /// Accounts whose features could not be read; the run continues.
    pub failed: u64,
    pub limit_reached: bool,
}

impl StarterDryRunReport {
    pub fn new() -> Self {
        Self {
            rule_version: STARTER_PLAUSIBILITY_RULE_VERSION,
            ..Self::default()
        }
    }

    pub fn record(&mut self, evaluation: &StarterEvaluation) {
        self.accounts_with_features += 1;
        match evaluation.decision {
            StarterDecision::Qualified => self.would_grant += 1,
            StarterDecision::NotYet(rule) => {
                *self.not_yet_by_rule.entry(rule.label()).or_default() += 1;
            }
        }
    }
}

fn share_at_least(part: u64, whole: u64, share: f64) -> bool {
    whole > 0 && (part as f64) >= share * (whole as f64)
}

/// The bit for `value` in a power-of-two bucket mask, capped at bit 31.
fn bucket_bit(value: u64) -> u32 {
    let bucket = if value <= 1 {
        0
    } else {
        (63 - value.leading_zeros()).min(31)
    };
    1u32 << bucket
}

fn truncate_to_hour(at: DateTime<Utc>) -> DateTime<Utc> {
    at.duration_trunc(Duration::hours(1)).unwrap_or(at)
}

fn event_type_label(event_type: TraceContributionEventType) -> &'static str {
    match event_type {
        TraceContributionEventType::UserMessage => "user_message",
        TraceContributionEventType::AssistantMessage => "assistant_message",
        TraceContributionEventType::Reasoning => "reasoning",
        TraceContributionEventType::ToolCall => "tool_call",
        TraceContributionEventType::ToolResult => "tool_result",
        TraceContributionEventType::RoutingDecision => "routing_decision",
        TraceContributionEventType::Feedback => "feedback",
        TraceContributionEventType::HttpExchange => "http_exchange",
    }
}

fn sha256_prefixed(parts: &[&[u8]]) -> String {
    let mut hash = Sha256::new();
    for part in parts {
        hash.update(part);
    }
    format!("sha256:{}", hex::encode(hash.finalize()))
}

#[cfg(test)]
#[path = "starter_plausibility_tests.rs"]
mod tests;
