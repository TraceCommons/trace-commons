// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The model-free substance formula: a deduplicated tool-call projection of an
//! envelope, and the work, replay and composite terms over it.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use trace_commons_gate_api::pipeline::ScoreWitnessClass;
use trace_commons_protocol::canonical_json::to_canonical_string;
use trace_commons_protocol::trace_contribution::{
    TraceContributionEnvelope, TraceContributionEventType, event_carries_result, replay_call_id,
    replay_sufficiency, tool_call_arguments,
};
use uuid::Uuid;

pub const SUBSTANCE_SCORE_IMPLEMENTATION: &str = "trace_commons.score.substance.v1";
pub const SUBSTANCE_SCORE_RULE: &str = "substance_novelty_v1";
pub const SUBSTANCE_VERSION: u32 = 1;

const MICROS: u32 = 1_000_000;

/// The set parameters of the formula, in micros where fractional.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstanceParams {
    pub pc_ceil: u32,
    pub dt_ceil: u32,
    pub w_pc_micros: u32,
    pub w_dt_micros: u32,
    pub w_hp_micros: u32,
    pub w_sa_micros: u32,
    pub w_sr_micros: u32,
    pub m_work_micros: u32,
    pub m_rep_micros: u32,
    pub att_floor_mult_micros: u32,
}

impl SubstanceParams {
    /// The values of the RLVR persona (spec section 6).
    pub fn rlvr_v1() -> Self {
        Self {
            pc_ceil: 20,
            dt_ceil: 8,
            w_pc_micros: 600_000,
            w_dt_micros: 400_000,
            w_hp_micros: 400_000,
            w_sa_micros: 300_000,
            w_sr_micros: 300_000,
            m_work_micros: 300_000,
            m_rep_micros: 100_000,
            att_floor_mult_micros: 900_000,
        }
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        let micros = [
            self.w_pc_micros,
            self.w_dt_micros,
            self.w_hp_micros,
            self.w_sa_micros,
            self.w_sr_micros,
            self.m_work_micros,
            self.m_rep_micros,
            self.att_floor_mult_micros,
        ];
        let floors = [
            self.m_work_micros,
            self.m_rep_micros,
            self.att_floor_mult_micros,
        ];
        if self.pc_ceil == 0
            || self.dt_ceil == 0
            || micros.iter().any(|value| *value > MICROS)
            || floors.contains(&0)
        {
            return Err("substance_params_invalid");
        }
        Ok(())
    }
}

/// Counts over the deduplicated tool calls of one envelope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ToolCallProjection {
    pub tool_calls: u64,
    pub paired_calls: u64,
    pub distinct_tools: u64,
    pub calls_with_arguments: u64,
    pub calls_with_results: u64,
    pub has_initial_prompt: bool,
}

struct ResultSlot<'a> {
    call_id: Option<&'a str>,
    parent_event_id: Option<Uuid>,
    used: bool,
}

#[derive(Default)]
struct CallGroup {
    has_arguments: bool,
    has_result: bool,
}

/// Project an envelope onto deduplicated tool calls (SQ-4 and SQ-5).
pub fn project_tool_calls(envelope: &TraceContributionEnvelope) -> ToolCallProjection {
    let mut results: Vec<ResultSlot<'_>> = envelope
        .events
        .iter()
        .filter(|event| event.event_type == TraceContributionEventType::ToolResult)
        .filter(|event| event_carries_result(event))
        .map(|event| ResultSlot {
            call_id: replay_call_id(event),
            parent_event_id: event.parent_event_id,
            used: false,
        })
        .collect();

    let mut groups: BTreeMap<(String, String), CallGroup> = BTreeMap::new();
    for call in envelope
        .events
        .iter()
        .filter(|event| event.event_type == TraceContributionEventType::ToolCall)
    {
        let name = call.tool_name.clone().unwrap_or_default();
        let arguments = tool_call_arguments(call);
        let canonical = arguments
            .map(|value| to_canonical_string(value).unwrap_or_else(|_| value.to_string()))
            .unwrap_or_default();
        let call_id = replay_call_id(call);
        let slot = results.iter_mut().find(|slot| {
            if slot.used {
                return false;
            }
            match (call_id, slot.call_id) {
                (Some(call_id), Some(result_id)) => call_id == result_id,
                _ => slot.parent_event_id == Some(call.event_id),
            }
        });
        let matched = slot.is_some_and(|slot| {
            slot.used = true;
            true
        });
        let group = groups.entry((name, canonical)).or_default();
        group.has_arguments = arguments.is_some();
        group.has_result |= matched;
    }

    let mut projection = ToolCallProjection {
        tool_calls: groups.len() as u64,
        has_initial_prompt: replay_sufficiency(envelope).has_initial_prompt,
        ..ToolCallProjection::default()
    };
    let mut paired_tools: BTreeSet<&str> = BTreeSet::new();
    for ((name, _), group) in &groups {
        projection.calls_with_arguments += u64::from(group.has_arguments);
        projection.calls_with_results += u64::from(group.has_result);
        if group.has_arguments && group.has_result {
            projection.paired_calls += 1;
            paired_tools.insert(name.as_str());
        }
    }
    projection.distinct_tools = paired_tools.len() as u64;
    projection
}

/// Raw terms, each clamped to `[0, 1]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SubstanceTerms {
    pub work: f64,
    pub replay: f64,
    pub v: f64,
}

/// `ln(1 + x) / ln(1 + ceil)`, clamped to `[0, 1]`.
pub fn sat(x: f64, ceil: f64) -> f64 {
    (x.ln_1p() / ceil.ln_1p()).clamp(0.0, 1.0)
}

/// `m + (1 - m) * v`: lifts `[0, 1]` onto `[m, 1]`.
pub fn graded(v: f64, m: f64) -> f64 {
    m + (1.0 - m) * v
}

/// Terms in micros: clamp to `[0, 1]`, then round.
pub fn to_micros(x: f64) -> u64 {
    (x.clamp(0.0, 1.0) * f64::from(MICROS)).round() as u64
}

fn weight(micros: u32) -> f64 {
    f64::from(micros) / f64::from(MICROS)
}

pub fn substance_terms(
    projection: &ToolCallProjection,
    witness: ScoreWitnessClass,
    params: &SubstanceParams,
) -> SubstanceTerms {
    let work = weight(params.w_pc_micros)
        * sat(projection.paired_calls as f64, f64::from(params.pc_ceil))
        + weight(params.w_dt_micros)
            * sat(projection.distinct_tools as f64, f64::from(params.dt_ceil));
    let (share_args, share_results) = if projection.tool_calls == 0 {
        (0.0, 0.0)
    } else {
        let calls = projection.tool_calls as f64;
        (
            projection.calls_with_arguments as f64 / calls,
            projection.calls_with_results as f64 / calls,
        )
    };
    let replay = weight(params.w_hp_micros) * f64::from(u8::from(projection.has_initial_prompt))
        + weight(params.w_sa_micros) * share_args
        + weight(params.w_sr_micros) * share_results;
    SubstanceTerms {
        work: work.clamp(0.0, 1.0),
        replay: replay.clamp(0.0, 1.0),
        v: f64::from(u8::from(witness.is_attested_final_call())),
    }
}

pub fn composite(terms: &SubstanceTerms, params: &SubstanceParams) -> f64 {
    graded(terms.work, weight(params.m_work_micros))
        * graded(terms.replay, weight(params.m_rep_micros))
        * graded(terms.v, weight(params.att_floor_mult_micros))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::{DateTime, TimeZone, Utc};
    use serde_json::{Value, json};
    use trace_commons_protocol::trace_contribution::{
        ConsentMetadata, ConsentScope, ContributorMetadata,
        DETERMINISTIC_REDACTION_PIPELINE_VERSION, IronclawTraceMetadata, OutcomeMetadata,
        PrivacyMetadata, ReplayMetadata, ResidualPiiRisk, SideEffectLevel,
        TRACE_CONTRIBUTION_POLICY_VERSION, TRACE_CONTRIBUTION_SCHEMA_VERSION, TraceCard,
        TraceChannel, TraceContributionEnvelope, TraceContributionEvent,
        TraceContributionEventType, TraceValueCard, ValueMetadata,
    };

    use super::*;

    const CLASSES: [ScoreWitnessClass; 4] = [
        ScoreWitnessClass::ProviderTeeFinalCall,
        ScoreWitnessClass::GatewayFinalCall,
        ScoreWitnessClass::Unattested,
        ScoreWitnessClass::LegacyV1,
    ];

    fn at() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 23, 12, 0, 0).unwrap()
    }

    fn id(label: &str) -> Uuid {
        Uuid::new_v5(&Uuid::NAMESPACE_OID, label.as_bytes())
    }

    fn event(label: &str, event_type: TraceContributionEventType) -> TraceContributionEvent {
        TraceContributionEvent {
            event_id: id(label),
            parent_event_id: None,
            event_type,
            timestamp: at(),
            redacted_content: None,
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

    fn prompt() -> TraceContributionEvent {
        let mut e = event("prompt", TraceContributionEventType::UserMessage);
        e.redacted_content = Some("fix the failing test".to_string());
        e
    }

    fn call(
        label: &str,
        tool: &str,
        payload: Value,
        call_id: Option<&str>,
    ) -> TraceContributionEvent {
        let mut e = event(label, TraceContributionEventType::ToolCall);
        e.tool_name = Some(tool.to_string());
        e.structured_payload = payload;
        e.tool_call_id = call_id.map(str::to_string);
        e
    }

    fn result(label: &str, call_id: Option<&str>, text: Option<&str>) -> TraceContributionEvent {
        let mut e = event(label, TraceContributionEventType::ToolResult);
        e.tool_call_id = call_id.map(str::to_string);
        e.redacted_content = text.map(str::to_string);
        e
    }

    fn args(value: Value) -> Value {
        json!({ "arguments": value })
    }

    fn envelope(events: Vec<TraceContributionEvent>) -> TraceContributionEnvelope {
        TraceContributionEnvelope {
            schema_version: TRACE_CONTRIBUTION_SCHEMA_VERSION.to_string(),
            trace_id: id("trace"),
            submission_id: id("submission"),
            created_at: at(),
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
                revocation_handle: id("handle"),
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

    fn paired(n: usize, tool: &str, arguments: Value) -> Vec<TraceContributionEvent> {
        let call_id = format!("c{n}");
        vec![
            call(
                &format!("call{n}"),
                tool,
                args(arguments),
                Some(call_id.as_str()),
            ),
            result(&format!("result{n}"), Some(call_id.as_str()), Some("ok")),
        ]
    }

    fn one_paired() -> TraceContributionEnvelope {
        let mut events = vec![prompt()];
        events.extend(paired(0, "read", json!({"path": "a"})));
        envelope(events)
    }

    fn twenty_paired() -> TraceContributionEnvelope {
        let mut events = vec![prompt()];
        for n in 0..20 {
            events.extend(paired(n, &format!("tool{}", n % 8), json!({ "n": n })));
        }
        envelope(events)
    }

    fn withheld() -> TraceContributionEnvelope {
        let marker = json!({"has_arguments": true, "has_result": true, "has_error": false});
        let mut events = vec![prompt()];
        for n in 0..3 {
            let call_id = format!("c{n}");
            events.push(call(
                &format!("call{n}"),
                "read",
                marker.clone(),
                Some(call_id.as_str()),
            ));
            events.push(result(&format!("result{n}"), Some(call_id.as_str()), None));
        }
        envelope(events)
    }

    fn one_paired_one_bare() -> TraceContributionEnvelope {
        let mut events = vec![prompt()];
        events.extend(paired(0, "read", json!({"path": "a"})));
        events.push(call("bare", "grep", Value::Null, None));
        envelope(events)
    }

    fn prompt_only() -> TraceContributionEnvelope {
        envelope(vec![prompt()])
    }

    fn goldens() -> Vec<TraceContributionEnvelope> {
        vec![
            envelope(Vec::new()),
            prompt_only(),
            one_paired(),
            twenty_paired(),
            withheld(),
            one_paired_one_bare(),
        ]
    }

    fn golden(envelope: &TraceContributionEnvelope, class: ScoreWitnessClass) -> (u64, u64, u64) {
        let params = SubstanceParams::rlvr_v1();
        let terms = substance_terms(&project_tool_calls(envelope), class, &params);
        (
            to_micros(terms.work),
            to_micros(terms.replay),
            to_micros(composite(&terms, &params)),
        )
    }

    fn substance(envelope: &TraceContributionEnvelope, class: ScoreWitnessClass) -> f64 {
        let params = SubstanceParams::rlvr_v1();
        let terms = substance_terms(&project_tool_calls(envelope), class, &params);
        composite(&terms, &params)
    }

    #[test]
    fn golden_empty_unattested() {
        assert_eq!(
            golden(&envelope(Vec::new()), ScoreWitnessClass::Unattested),
            (0, 0, 27_000)
        );
    }

    #[test]
    fn golden_prompt_only_unattested() {
        assert_eq!(
            golden(&prompt_only(), ScoreWitnessClass::Unattested),
            (0, 400_000, 124_200)
        );
    }

    #[test]
    fn golden_one_paired_call_unattested() {
        assert_eq!(
            golden(&one_paired(), ScoreWitnessClass::Unattested),
            (262_788, 1_000_000, 435_557)
        );
    }

    #[test]
    fn golden_one_paired_call_provider_tee() {
        assert_eq!(
            golden(&one_paired(), ScoreWitnessClass::ProviderTeeFinalCall),
            (262_788, 1_000_000, 483_952)
        );
    }

    #[test]
    fn golden_twenty_paired_eight_tools_gateway() {
        assert_eq!(
            golden(&twenty_paired(), ScoreWitnessClass::GatewayFinalCall),
            (1_000_000, 1_000_000, 1_000_000)
        );
    }

    #[test]
    fn golden_withheld_markers_unattested() {
        let envelope = withheld();
        assert_eq!(project_tool_calls(&envelope).tool_calls, 1);
        assert_eq!(
            golden(&envelope, ScoreWitnessClass::Unattested),
            (0, 400_000, 124_200)
        );
    }

    #[test]
    fn golden_one_paired_one_bare_unattested() {
        assert_eq!(
            golden(&one_paired_one_bare(), ScoreWitnessClass::Unattested),
            (262_788, 700_000, 317_956)
        );
    }

    #[test]
    fn golden_legacy_v1_is_unattested() {
        assert_eq!(
            golden(&one_paired(), ScoreWitnessClass::LegacyV1),
            (262_788, 1_000_000, 435_557)
        );
    }

    #[test]
    fn dedup_ignores_argument_key_order() {
        let mut events = vec![prompt()];
        events.extend(paired(0, "read", json!({"a": 1, "b": 2})));
        events.extend(paired(1, "read", json!({"b": 2, "a": 1})));
        let projection = project_tool_calls(&envelope(events));
        assert_eq!(projection.tool_calls, 1);
        assert_eq!(projection.paired_calls, 1);
    }

    #[test]
    fn a_varied_argument_still_counts() {
        let mut events = vec![prompt()];
        events.extend(paired(0, "read", json!({"n": 1})));
        events.extend(paired(1, "read", json!({"n": 2})));
        let projection = project_tool_calls(&envelope(events));
        assert_eq!(projection.paired_calls, 2);
        assert_eq!(projection.distinct_tools, 1);
    }

    #[test]
    fn orphan_and_repeated_results_pair_once() {
        let target = call("call", "read", args(json!({"path": "a"})), Some("c1"));
        let mut early = result("early", None, Some("early"));
        early.parent_event_id = Some(target.event_id);
        let events = vec![
            prompt(),
            early,
            target,
            result("r1", Some("c1"), Some("one")),
            result("r2", Some("c1"), Some("two")),
            result("r9", Some("c9"), Some("orphan")),
        ];
        let projection = project_tool_calls(&envelope(events));
        assert_eq!(projection.paired_calls, 1);
        assert_eq!(projection.calls_with_results, 1);
    }

    #[test]
    fn results_match_by_parent_event_id_without_ids() {
        let target = call("call", "read", args(json!({"path": "a"})), None);
        let mut reply = result("reply", None, Some("ok"));
        reply.parent_event_id = Some(target.event_id);
        let projection = project_tool_calls(&envelope(vec![prompt(), target, reply]));
        assert_eq!(projection.paired_calls, 1);
        assert_eq!(projection.calls_with_results, 1);
    }

    #[test]
    fn a_tool_free_trace_earns_at_most_the_prompt_share() {
        let params = SubstanceParams::rlvr_v1();
        let projection = project_tool_calls(&prompt_only());
        let terms = substance_terms(&projection, ScoreWitnessClass::Unattested, &params);
        assert!((terms.replay - 0.4).abs() < 1e-12);
    }

    #[test]
    fn more_paired_calls_never_lower_substance() {
        let mut events = vec![prompt()];
        let mut previous = substance(&envelope(events.clone()), ScoreWitnessClass::Unattested);
        for n in 1..=40usize {
            events.extend(paired(n, &format!("tool{}", n / 3), json!({ "n": n })));
            let current = substance(&envelope(events.clone()), ScoreWitnessClass::Unattested);
            assert!(current >= previous, "substance fell at n = {n}");
            previous = current;
        }
    }

    #[test]
    fn no_term_vetoes() {
        let params = SubstanceParams::rlvr_v1();
        for envelope in goldens() {
            for class in CLASSES {
                let terms = substance_terms(&project_tool_calls(&envelope), class, &params);
                assert!(composite(&terms, &params) >= 0.3 * 0.1 * 0.9 - 1e-12);
            }
        }
    }

    #[test]
    fn rlvr_v1_matches_the_spec_table() {
        let params = SubstanceParams::rlvr_v1();
        assert_eq!(params.pc_ceil, 20);
        assert_eq!(params.dt_ceil, 8);
        assert_eq!(params.w_pc_micros, 600_000);
        assert_eq!(params.w_dt_micros, 400_000);
        assert_eq!(params.w_hp_micros, 400_000);
        assert_eq!(params.w_sa_micros, 300_000);
        assert_eq!(params.w_sr_micros, 300_000);
        assert_eq!(params.m_work_micros, 300_000);
        assert_eq!(params.m_rep_micros, 100_000);
        assert_eq!(params.att_floor_mult_micros, 900_000);
        assert_eq!(params.validate(), Ok(()));
    }

    #[test]
    fn params_refuse_a_zero_floor() {
        let mut params = SubstanceParams::rlvr_v1();
        params.m_rep_micros = 0;
        assert_eq!(params.validate(), Err("substance_params_invalid"));
    }
}
