// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, TimeZone, Utc};
use serde_json::Value;
use trace_commons_protocol::trace_contribution::{
    ConsentMetadata, ConsentScope, ContributorMetadata, DETERMINISTIC_REDACTION_PIPELINE_VERSION,
    Decimal, IronclawTraceMetadata, OutcomeMetadata, PrivacyMetadata, ReplayMetadata,
    ResidualPiiRisk, SideEffectLevel, SourceSessionIdentity, TRACE_CONTRIBUTION_POLICY_VERSION,
    TRACE_CONTRIBUTION_SCHEMA_VERSION, TokenCounts, TraceCard, TraceChannel,
    TraceContributionEnvelope, TraceContributionEvent, TraceContributionEventType, TraceValueCard,
    ValueMetadata,
};
use uuid::Uuid;

use super::*;

/// Evaluation time for every test: fixed, so nothing depends on the clock.
fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 21, 18, 0, 0).unwrap()
}

/// Turn gaps in seconds, cycled: irregular, with think time.
const GAPS: [i64; 7] = [3, 45, 9, 140, 22, 6, 700];
/// Tool latencies in milliseconds, cycled.
const LATENCIES: [u64; 5] = [40, 900, 3_000, 120, 15_000];

#[derive(Clone)]
struct Session {
    adapter: Option<&'static str>,
    model: Option<&'static str>,
    tool: &'static str,
    turns: usize,
    start: DateTime<Utc>,
    gaps: Vec<i64>,
    with_ids: bool,
    with_results: bool,
    with_failure: bool,
    with_tokens: bool,
    seed: u32,
}

impl Session {
    fn claude(start: DateTime<Utc>, seed: u32) -> Self {
        Self {
            adapter: Some("claude-code"),
            model: Some("claude-sonnet-4-5"),
            tool: "Bash",
            turns: 20,
            start,
            gaps: GAPS.to_vec(),
            with_ids: true,
            with_results: true,
            with_failure: true,
            with_tokens: true,
            seed,
        }
    }

    fn codex(start: DateTime<Utc>, seed: u32) -> Self {
        Self {
            adapter: Some("codex"),
            model: Some("gpt-5-codex"),
            tool: "shell",
            ..Self::claude(start, seed)
        }
    }

    fn envelope(&self) -> TraceContributionEnvelope {
        let mut events = Vec::new();
        let mut at = self.start;
        let mut input_tokens = 1_000u32;
        for turn in 0..self.turns {
            let gap = self.gaps[turn % self.gaps.len()];
            events.push(event(
                TraceContributionEventType::UserMessage,
                at,
                Some(format!("ask {} {}", self.seed, turn)),
            ));
            at += Duration::seconds(gap);
            let mut assistant = event(
                TraceContributionEventType::AssistantMessage,
                at,
                Some(format!("answer {} {}", self.seed, turn)),
            );
            if self.with_tokens {
                assistant.token_counts = Some(TokenCounts {
                    input_tokens,
                    output_tokens: 200,
                });
                assistant.cost_usd = Some(Decimal::new(12, 3));
                input_tokens += 750;
            }
            events.push(assistant);
            let call_id = format!("toolu_{}_{}", self.seed, turn);
            let mut call = event(TraceContributionEventType::ToolCall, at, None);
            call.tool_name = Some(self.tool.to_string());
            call.tool_call_id = self.with_ids.then(|| call_id.clone());
            call.latency_ms = Some(LATENCIES[turn % LATENCIES.len()]);
            events.push(call);
            if self.with_results {
                let mut result = event(TraceContributionEventType::ToolResult, at, None);
                result.tool_name = Some(self.tool.to_string());
                result.tool_call_id = self.with_ids.then_some(call_id);
                result.success = Some(!(self.with_failure && turn == 3));
                events.push(result);
            }
            at += Duration::seconds(gap);
        }
        let mut envelope = base_envelope(events);
        envelope.ironclaw.model_name = self.model.map(str::to_string);
        envelope.source_session = self.adapter.map(|adapter| SourceSessionIdentity {
            adapter: adapter.to_string(),
            native_id: Uuid::from_u128(u128::from(self.seed) + 1).to_string(),
        });
        envelope
    }

    fn features(&self) -> StarterEnvelopeFeatures {
        extract_features(&self.envelope(), now())
    }
}

fn event(
    event_type: TraceContributionEventType,
    timestamp: DateTime<Utc>,
    content: Option<String>,
) -> TraceContributionEvent {
    TraceContributionEvent {
        event_id: Uuid::new_v4(),
        parent_event_id: None,
        event_type,
        timestamp,
        redacted_content: content,
        structured_payload: Value::Null,
        tool_name: None,
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

fn base_envelope(events: Vec<TraceContributionEvent>) -> TraceContributionEnvelope {
    TraceContributionEnvelope {
        schema_version: TRACE_CONTRIBUTION_SCHEMA_VERSION.to_string(),
        trace_id: Uuid::new_v4(),
        submission_id: Uuid::new_v4(),
        created_at: now(),
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

/// Nine sessions over four days, the last ending a day before `now`.
fn plausible_week(make: fn(DateTime<Utc>, u32) -> Session) -> Vec<Session> {
    (0..9u32)
        .map(|i| {
            let day = now() - Duration::days(4) + Duration::days(i64::from(i / 3));
            make(day + Duration::hours(i64::from(9 + 2 * (i % 3))), i)
        })
        .collect()
}

fn features_of(sessions: &[Session]) -> Vec<StarterEnvelopeFeatures> {
    sessions.iter().map(Session::features).collect()
}

fn decision(sessions: &[Session]) -> StarterDecision {
    evaluate(&StarterPolicy::v1(), &features_of(sessions), now()).decision
}

#[test]
fn a_few_days_of_claude_code_qualifies() {
    let evaluation = evaluate(
        &StarterPolicy::v1(),
        &features_of(&plausible_week(Session::claude)),
        now(),
    );
    assert_eq!(evaluation.decision, StarterDecision::Qualified);
    assert_eq!(evaluation.rule_version, "starter-plausibility-v1");
    assert_eq!(evaluation.totals.sessions, 9);
    assert_eq!(evaluation.totals.tool_calls, 180);
    assert_eq!(evaluation.totals.distinct_days, 3);
    assert!(evaluation.totals.span_hours >= 48);
}

#[test]
fn a_few_days_of_codex_qualifies() {
    assert_eq!(
        decision(&plausible_week(Session::codex)),
        StarterDecision::Qualified
    );
}

#[test]
fn no_source_session_or_another_agent_is_not_counted() {
    let mut none = plausible_week(Session::claude);
    none.iter_mut().for_each(|s| s.adapter = None);
    assert_eq!(
        decision(&none),
        StarterDecision::NotYet(StarterRule::SourceAgent)
    );

    let mut other = plausible_week(Session::claude);
    other.iter_mut().for_each(|s| s.adapter = Some("opencode"));
    assert_eq!(
        decision(&other),
        StarterDecision::NotYet(StarterRule::SourceAgent)
    );

    assert_eq!(
        decision(&[]),
        StarterDecision::NotYet(StarterRule::SourceAgent)
    );
}

/// A source identity the protocol would refuse (here a Claude Code id that
/// is not a UUID) counts as no source, so it neither names an agent nor
/// gives a session hash.
#[test]
fn an_invalid_source_identity_is_not_counted() {
    let session = &plausible_week(Session::claude)[0];
    let mut envelope = session.envelope();
    envelope.source_session.as_mut().unwrap().native_id = "not-a-uuid".to_string();
    let features = extract_features(&envelope, now());
    assert_eq!(features.agent, StarterAgent::None);
    assert!(features.session_hash.is_none());

    let valid = extract_features(&session.envelope(), now());
    assert_ne!(valid.agent, StarterAgent::None);
    assert!(valid.session_hash.is_some());
}

#[test]
fn too_few_sessions_or_tool_calls_is_volume() {
    let few = plausible_week(Session::claude)[..5].to_vec();
    assert_eq!(decision(&few), StarterDecision::NotYet(StarterRule::Volume));

    let mut short = plausible_week(Session::claude);
    short.iter_mut().for_each(|s| s.turns = 10);
    assert_eq!(
        decision(&short),
        StarterDecision::NotYet(StarterRule::Volume)
    );
}

#[test]
fn one_session_id_split_across_envelopes_counts_once() {
    let mut same = plausible_week(Session::claude);
    same.iter_mut().for_each(|s| {
        // Distinct content, one session id.
        s.seed += 100;
    });
    let features: Vec<StarterEnvelopeFeatures> = same
        .iter()
        .map(|s| {
            let mut envelope = s.envelope();
            envelope.source_session.as_mut().unwrap().native_id = Uuid::from_u128(7).to_string();
            extract_features(&envelope, now())
        })
        .collect();
    let evaluation = evaluate(&StarterPolicy::v1(), &features, now());
    assert_eq!(evaluation.totals.sessions, 1);
    assert_eq!(
        evaluation.decision,
        StarterDecision::NotYet(StarterRule::Volume)
    );
}

#[test]
fn one_day_of_work_is_span() {
    let mut one_day = plausible_week(Session::claude);
    one_day
        .iter_mut()
        .enumerate()
        .for_each(|(i, s)| s.start = now() - Duration::days(1) + Duration::minutes(40 * i as i64));
    assert_eq!(
        decision(&one_day),
        StarterDecision::NotYet(StarterRule::Span)
    );
}

#[test]
fn a_future_dated_envelope_is_span() {
    let mut sessions = plausible_week(Session::claude);
    sessions.push(Session::claude(now() + Duration::days(2), 50));
    assert_eq!(
        decision(&sessions),
        StarterDecision::NotYet(StarterRule::Span)
    );
}

#[test]
fn small_clock_skew_is_not_future_dated() {
    let features = Session {
        turns: 1,
        ..Session::claude(now() + Duration::minutes(10), 1)
    }
    .features();
    assert!(!features.future_dated);
}

#[test]
fn envelopes_older_than_the_age_limit_are_not_counted() {
    let mut sessions = plausible_week(Session::claude);
    sessions
        .iter_mut()
        .for_each(|s| s.start -= Duration::days(200));
    assert_eq!(
        decision(&sessions),
        StarterDecision::NotYet(StarterRule::SourceAgent)
    );
}

#[test]
fn machine_regular_turns_are_rhythm() {
    let mut regular = plausible_week(Session::claude);
    regular.iter_mut().for_each(|s| s.gaps = vec![2]);
    assert_eq!(
        decision(&regular),
        StarterDecision::NotYet(StarterRule::Rhythm)
    );
}

#[test]
fn a_mismatched_model_or_unknown_tools_is_agent_signature() {
    let mut mismatched = plausible_week(Session::codex);
    mismatched
        .iter_mut()
        .for_each(|s| s.model = Some("claude-opus-4"));
    assert_eq!(
        decision(&mismatched),
        StarterDecision::NotYet(StarterRule::AgentSignature)
    );

    let mut unknown = plausible_week(Session::claude);
    unknown.iter_mut().for_each(|s| s.tool = "run_terminal_cmd");
    assert_eq!(
        decision(&unknown),
        StarterDecision::NotYet(StarterRule::AgentSignature)
    );
}

#[test]
fn no_model_name_does_not_fail_agent_signature() {
    let mut unnamed = plausible_week(Session::claude);
    unnamed.iter_mut().for_each(|s| s.model = None);
    assert_eq!(decision(&unnamed), StarterDecision::Qualified);
}

#[test]
fn mcp_tools_count_for_either_agent() {
    let mut mcp = plausible_week(Session::codex);
    mcp.iter_mut().for_each(|s| s.tool = "mcp__github__get_me");
    assert_eq!(decision(&mcp), StarterDecision::Qualified);
}

#[test]
fn unanswered_calls_or_no_failures_is_tool_pairing() {
    let mut unanswered = plausible_week(Session::claude);
    unanswered.iter_mut().for_each(|s| s.with_results = false);
    assert_eq!(
        decision(&unanswered),
        StarterDecision::NotYet(StarterRule::ToolPairing)
    );

    let mut flawless = plausible_week(Session::claude);
    flawless.iter_mut().for_each(|s| s.with_failure = false);
    assert_eq!(
        decision(&flawless),
        StarterDecision::NotYet(StarterRule::ToolPairing)
    );
}

#[test]
fn calls_without_ids_pair_by_count() {
    let mut no_ids = plausible_week(Session::claude);
    no_ids.iter_mut().for_each(|s| s.with_ids = false);
    let features = features_of(&no_ids);
    assert!(features.iter().all(|f| f.paired_tool_calls == f.tool_calls));
    assert_eq!(
        evaluate(&StarterPolicy::v1(), &features, now()).decision,
        StarterDecision::Qualified
    );
}

#[test]
fn no_token_counts_is_usage() {
    let mut untokened = plausible_week(Session::claude);
    untokened.iter_mut().for_each(|s| s.with_tokens = false);
    assert_eq!(
        decision(&untokened),
        StarterDecision::NotYet(StarterRule::Usage)
    );
}

#[test]
fn a_repeated_envelope_is_not_copied() {
    let mut sessions = plausible_week(Session::claude);
    let mut copy = sessions[0].clone();
    // Same content, different session id: the content hash still repeats.
    let mut envelope = copy.envelope();
    envelope.source_session.as_mut().unwrap().native_id = Uuid::from_u128(999).to_string();
    let mut features = features_of(&sessions);
    features.push(extract_features(&envelope, now()));
    assert_eq!(
        evaluate(&StarterPolicy::v1(), &features, now()).decision,
        StarterDecision::NotYet(StarterRule::NotCopied)
    );
    // And the original set alone still qualifies.
    copy.seed = 500;
    sessions.push(copy);
    assert_eq!(decision(&sessions), StarterDecision::Qualified);
}

#[test]
fn features_are_hash_only() {
    let session = Session::claude(now() - Duration::days(1), 42);
    let envelope = session.envelope();
    let features = extract_features(&envelope, now());
    let stored = serde_json::to_string(&features).unwrap();
    let native_id = &envelope.source_session.as_ref().unwrap().native_id;
    assert!(!stored.contains(native_id.as_str()));
    assert!(!stored.contains("ask 42"));
    assert!(!stored.contains("claude-sonnet"));
    assert!(!stored.contains("toolu_"));
    assert!(
        features
            .session_hash
            .as_deref()
            .unwrap()
            .starts_with("sha256:")
    );
    assert!(features.content_hash.starts_with("sha256:"));
    let first = features.first_event_hour.unwrap();
    assert_eq!(
        first.timestamp() % 3600,
        0,
        "times are truncated to the hour"
    );
}

#[test]
fn features_round_trip_and_refuse_unknown_fields() {
    let features = Session::claude(now() - Duration::days(1), 3).features();
    let json = serde_json::to_value(&features).unwrap();
    let back: StarterEnvelopeFeatures = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(back, features);

    let mut extra = json;
    extra
        .as_object_mut()
        .unwrap()
        .insert("model_name".to_string(), Value::from("claude"));
    assert!(serde_json::from_value::<StarterEnvelopeFeatures>(extra).is_err());
}

#[test]
fn an_unknown_feature_version_is_not_counted() {
    let mut features = features_of(&plausible_week(Session::claude));
    features
        .iter_mut()
        .for_each(|f| f.version = "starter-features-v0".to_string());
    assert_eq!(
        evaluate(&StarterPolicy::v1(), &features, now()).decision,
        StarterDecision::NotYet(StarterRule::SourceAgent)
    );
}

#[test]
fn model_families() {
    assert_eq!(
        ModelFamily::of(Some("claude-opus-4-1")),
        ModelFamily::Anthropic
    );
    assert_eq!(
        ModelFamily::of(Some("anthropic/claude-3-5-sonnet")),
        ModelFamily::Anthropic
    );
    assert_eq!(ModelFamily::of(Some("gpt-5-codex")), ModelFamily::OpenAi);
    assert_eq!(ModelFamily::of(Some("o3")), ModelFamily::OpenAi);
    assert_eq!(ModelFamily::of(Some("openai/gpt-4.1")), ModelFamily::OpenAi);
    assert_eq!(ModelFamily::of(Some("qwen3-coder")), ModelFamily::Other);
    assert_eq!(ModelFamily::of(Some("  ")), ModelFamily::None);
    assert_eq!(ModelFamily::of(None), ModelFamily::None);
}

#[test]
fn bucket_bits() {
    assert_eq!(bucket_bit(0), 1);
    assert_eq!(bucket_bit(1), 1);
    assert_eq!(bucket_bit(2), 1 << 1);
    assert_eq!(bucket_bit(3), 1 << 1);
    assert_eq!(bucket_bit(1_000), 1 << 9);
    assert_eq!(bucket_bit(u64::MAX), 1 << 31);
}

#[test]
fn rule_labels_are_distinct() {
    let labels: std::collections::BTreeSet<&str> =
        StarterRule::ALL.iter().map(|rule| rule.label()).collect();
    assert_eq!(labels.len(), StarterRule::ALL.len());
    assert!(!labels.contains("qualified"));
}

#[test]
fn dry_run_report_counts_by_first_failed_rule() {
    let policy = StarterPolicy::v1();
    let mut report = StarterDryRunReport::new();
    report.record(&evaluate(
        &policy,
        &features_of(&plausible_week(Session::claude)),
        now(),
    ));
    report.record(&evaluate(
        &policy,
        &features_of(&plausible_week(Session::claude)[..2]),
        now(),
    ));
    report.record(&evaluate(&policy, &[], now()));
    assert_eq!(report.rule_version, "starter-plausibility-v1");
    assert_eq!(report.accounts_with_features, 3);
    assert_eq!(report.would_grant, 1);
    assert_eq!(report.not_yet_by_rule.get("volume"), Some(&1));
    assert_eq!(report.not_yet_by_rule.get("source_agent"), Some(&1));
}
