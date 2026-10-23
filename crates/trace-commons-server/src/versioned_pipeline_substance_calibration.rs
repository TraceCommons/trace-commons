// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Offline calibration checks of the substance Score: the pairwise regression
//! guardrail of spec section 7.1 (check 1) on synthetic bases, and the
//! monotonicity sweep (check 2).
//!
//! The committed fixture, the corpus bases and `dataset generate` of
//! section 7.1 are in the later plan. Nothing here runs a model, an embedder
//! or the index, and the generator never runs the score.

use std::collections::BTreeMap;

use base64::Engine;
use chrono::{DateTime, TimeZone, Utc};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use trace_commons_gate_api::pipeline::ScoreWitnessClass;
use trace_commons_protocol::trace_contribution::{
    ConsentMetadata, ConsentScope, ContributorMetadata, DETERMINISTIC_REDACTION_PIPELINE_VERSION,
    IronclawTraceMetadata, OutcomeMetadata, PrivacyMetadata, ReplayMetadata, ResidualPiiRisk,
    SideEffectLevel, TRACE_CONTRIBUTION_POLICY_VERSION, TRACE_CONTRIBUTION_SCHEMA_VERSION,
    TaskSuccess, TraceCard, TraceChannel, TraceContributionEnvelope, TraceContributionEvent,
    TraceContributionEventType, TraceValueCard, ValueMetadata,
};
use uuid::Uuid;

use crate::versioned_pipeline_substance::{
    SubstanceParams, composite, project_tool_calls, substance_terms, to_micros,
};

/// The fixed paragraph of the template filler.
const TEMPLATE_PARAGRAPH: &str = "I have reviewed the request and I will proceed with the \
    requested change. The work is complete and the result matches the request. ";

/// The seed of [`run_synthetic_guardrail`].
const SYNTHETIC_SEED: u64 = 1;

/// The paired-call counts of the synthetic bases.
const SYNTHETIC_BASE_CALLS: [usize; 8] = [1, 2, 3, 5, 8, 13, 20, 30];

pub const GUARDRAIL_BASE_INELIGIBLE_LABEL: &str = "guardrail_base_ineligible";

/// The seven transforms of spec section 7.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardrailTransform {
    AppendNoise,
    TemplateFiller,
    AddScaffold,
    TruncateHalf,
    DuplicateEvents,
    StripResults,
    AssertSuccessNoWork,
}

/// What a variant must do against its base.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardrailExpectation {
    Equal,
    NotAbove,
    Below,
}

impl GuardrailTransform {
    pub const ALL: [Self; 7] = [
        Self::AppendNoise,
        Self::TemplateFiller,
        Self::AddScaffold,
        Self::TruncateHalf,
        Self::DuplicateEvents,
        Self::StripResults,
        Self::AssertSuccessNoWork,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::AppendNoise => "append_noise",
            Self::TemplateFiller => "template_filler",
            Self::AddScaffold => "add_scaffold",
            Self::TruncateHalf => "truncate_half",
            Self::DuplicateEvents => "duplicate_events",
            Self::StripResults => "strip_results",
            Self::AssertSuccessNoWork => "assert_success_no_work",
        }
    }

    /// The `Expect` column of spec section 7.1.
    pub fn expectation(self) -> GuardrailExpectation {
        match self {
            Self::AppendNoise | Self::TemplateFiller => GuardrailExpectation::Equal,
            Self::AddScaffold | Self::TruncateHalf | Self::DuplicateEvents => {
                GuardrailExpectation::NotAbove
            }
            Self::StripResults | Self::AssertSuccessNoWork => GuardrailExpectation::Below,
        }
    }
}

/// The parameters of the recipe. A change to any of them is a new recipe id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GuardrailRecipe {
    pub noise_bytes: usize,
    pub scaffold_calls: usize,
}

impl GuardrailRecipe {
    pub fn v1() -> Self {
        Self {
            noise_bytes: 4096,
            scaffold_calls: 20,
        }
    }

    pub fn recipe_id(&self) -> String {
        let canonical = json!({
            "noise_bytes": self.noise_bytes,
            "scaffold_calls": self.scaffold_calls,
        })
        .to_string();
        format!(
            "sha256:{}",
            hex::encode(Sha256::digest(canonical.as_bytes()))
        )
    }
}

/// Random-looking bytes from SHA-256 blocks over `(seed, trace id, counter)`.
fn noise(seed: u64, trace_id: Uuid, len: usize) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(len + 32);
    let mut counter = 0u64;
    while bytes.len() < len {
        let mut hasher = Sha256::new();
        hasher.update(seed.to_le_bytes());
        hasher.update(trace_id.as_bytes());
        hasher.update(counter.to_le_bytes());
        bytes.extend_from_slice(&hasher.finalize());
        counter += 1;
    }
    bytes.truncate(len);
    bytes
}

pub fn apply_transform(
    transform: GuardrailTransform,
    base: &TraceContributionEnvelope,
    recipe: &GuardrailRecipe,
    seed: u64,
) -> TraceContributionEnvelope {
    let mut variant = base.clone();
    match transform {
        GuardrailTransform::AppendNoise => {
            let encoded = base64::engine::general_purpose::STANDARD.encode(noise(
                seed,
                base.trace_id,
                recipe.noise_bytes,
            ));
            if let Some(last) = variant
                .events
                .iter_mut()
                .rev()
                .find(|event| event.event_type == TraceContributionEventType::AssistantMessage)
            {
                last.redacted_content
                    .get_or_insert_with(String::new)
                    .push_str(&encoded);
            }
        }
        GuardrailTransform::TemplateFiller => {
            for event in variant
                .events
                .iter_mut()
                .filter(|event| event.event_type == TraceContributionEventType::AssistantMessage)
            {
                if let Some(content) = event.redacted_content.as_mut() {
                    let wanted = content.len();
                    let mut filler =
                        TEMPLATE_PARAGRAPH.repeat(wanted / TEMPLATE_PARAGRAPH.len() + 1);
                    filler.truncate(wanted);
                    *content = filler;
                }
            }
        }
        GuardrailTransform::AddScaffold => {
            let at = variant.created_at;
            for index in 0..recipe.scaffold_calls {
                variant.events.push(TraceContributionEvent {
                    event_id: Uuid::new_v5(&base.trace_id, format!("scaffold:{index}").as_bytes()),
                    parent_event_id: None,
                    event_type: TraceContributionEventType::ToolCall,
                    timestamp: at,
                    redacted_content: None,
                    structured_payload: Value::Null,
                    tool_name: Some("scaffold".to_string()),
                    tool_category: None,
                    tool_call_id: None,
                    latency_ms: None,
                    token_counts: None,
                    cost_usd: None,
                    success: None,
                    failure_modes: Vec::new(),
                    side_effect: SideEffectLevel::None,
                });
            }
        }
        GuardrailTransform::TruncateHalf => {
            variant.events.truncate(base.events.len() / 2);
        }
        GuardrailTransform::DuplicateEvents => {
            let renamed = |id: Uuid| Uuid::new_v5(&id, b"duplicate");
            let copies: Vec<TraceContributionEvent> = base
                .events
                .iter()
                .map(|event| {
                    let mut copy = event.clone();
                    copy.event_id = renamed(event.event_id);
                    copy.parent_event_id = event.parent_event_id.map(renamed);
                    copy
                })
                .collect();
            variant.events.extend(copies);
        }
        GuardrailTransform::StripResults => {
            for event in variant
                .events
                .iter_mut()
                .filter(|event| event.event_type == TraceContributionEventType::ToolResult)
            {
                event.redacted_content = None;
                event.structured_payload = Value::Null;
            }
        }
        GuardrailTransform::AssertSuccessNoWork => {
            variant.events.retain(|event| {
                !matches!(
                    event.event_type,
                    TraceContributionEventType::ToolCall | TraceContributionEventType::ToolResult
                )
            });
            variant.outcome.task_success = TaskSuccess::Success;
        }
    }
    variant
}

/// A base has at least one paired call, and every tool call in it is paired
/// (spec section 7.1).
pub fn guardrail_base_eligible(envelope: &TraceContributionEnvelope) -> bool {
    let projection = project_tool_calls(envelope);
    projection.paired_calls >= 1 && projection.tool_calls == projection.paired_calls
}

#[derive(Debug, Clone)]
pub struct GuardrailPair {
    pub base_label: String,
    pub transform: GuardrailTransform,
    pub expectation: GuardrailExpectation,
    pub base: TraceContributionEnvelope,
    pub variant: TraceContributionEnvelope,
}

fn calibration_time() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 23, 12, 0, 0).unwrap()
}

fn calibration_event(
    key: &str,
    event_type: TraceContributionEventType,
    content: Option<String>,
) -> TraceContributionEvent {
    TraceContributionEvent {
        event_id: Uuid::new_v5(&Uuid::NAMESPACE_OID, key.as_bytes()),
        parent_event_id: None,
        event_type,
        timestamp: calibration_time(),
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

/// One paired call: a call with arguments and a result that names it.
fn paired_events(label: &str, index: usize, tool: &str) -> [TraceContributionEvent; 2] {
    let call_id = format!("c{index}");
    let mut call = calibration_event(
        &format!("{label}:call:{index}"),
        TraceContributionEventType::ToolCall,
        None,
    );
    call.tool_name = Some(tool.to_string());
    call.structured_payload = json!({ "arguments": { "n": index } });
    call.tool_call_id = Some(call_id.clone());
    let mut result = calibration_event(
        &format!("{label}:result:{index}"),
        TraceContributionEventType::ToolResult,
        Some("ok".to_string()),
    );
    result.parent_event_id = Some(call.event_id);
    result.tool_call_id = Some(call_id);
    [call, result]
}

fn calibration_envelope(
    label: &str,
    events: Vec<TraceContributionEvent>,
) -> TraceContributionEnvelope {
    let id = |part: &str| Uuid::new_v5(&Uuid::NAMESPACE_OID, format!("{label}:{part}").as_bytes());
    TraceContributionEnvelope {
        schema_version: TRACE_CONTRIBUTION_SCHEMA_VERSION.to_string(),
        trace_id: id("trace"),
        submission_id: id("submission"),
        created_at: calibration_time(),
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

/// Eight labelled bases with 1, 2, 3, 5, 8, 13, 20 and 30 paired calls over
/// `min(n, 8)` tool names, a prompt and two assistant messages. Every call is
/// paired. Ids and timestamps derive from the label, so the bases are fixed.
pub fn synthetic_guardrail_bases() -> Vec<(String, TraceContributionEnvelope)> {
    SYNTHETIC_BASE_CALLS
        .iter()
        .map(|&calls| {
            let label = format!("synthetic_{calls:02}_calls");
            let tools = calls.min(8);
            let mut events = vec![
                calibration_event(
                    &format!("{label}:prompt"),
                    TraceContributionEventType::UserMessage,
                    Some("fix the failing test in the parser".to_string()),
                ),
                calibration_event(
                    &format!("{label}:plan"),
                    TraceContributionEventType::AssistantMessage,
                    Some("I will read the parser and run the failing test.".to_string()),
                ),
            ];
            for index in 0..calls {
                events.extend(paired_events(
                    &label,
                    index,
                    &format!("tool{}", index % tools),
                ));
            }
            events.push(calibration_event(
                &format!("{label}:summary"),
                TraceContributionEventType::AssistantMessage,
                Some("The test passes after the change to the parser.".to_string()),
            ));
            let envelope = calibration_envelope(&label, events);
            (label, envelope)
        })
        .collect()
}

/// Build every pair. The generator checks the structure of each base and
/// never runs the score.
pub fn generate_guardrail_pairs(
    bases: &[(String, TraceContributionEnvelope)],
    recipe: &GuardrailRecipe,
    seed: u64,
) -> Result<Vec<GuardrailPair>, &'static str> {
    if bases.iter().any(|(_, base)| !guardrail_base_eligible(base)) {
        return Err(GUARDRAIL_BASE_INELIGIBLE_LABEL);
    }
    let mut pairs = Vec::with_capacity(bases.len() * GuardrailTransform::ALL.len());
    for (label, base) in bases {
        for transform in GuardrailTransform::ALL {
            pairs.push(GuardrailPair {
                base_label: label.clone(),
                transform,
                expectation: transform.expectation(),
                base: base.clone(),
                variant: apply_transform(transform, base, recipe, seed),
            });
        }
    }
    Ok(pairs)
}

#[derive(Debug, Clone, Serialize)]
pub struct GuardrailPairResult {
    pub met: bool,
    pub base_micros: u64,
    pub variant_micros: u64,
    /// The term with the larger change: `work`, `replay` or `none`.
    pub most_moved: &'static str,
}

/// Score both sides as `Unattested` and compare them in micros.
pub fn check_guardrail_pair(pair: &GuardrailPair, params: &SubstanceParams) -> GuardrailPairResult {
    let score = |envelope: &TraceContributionEnvelope| {
        let terms = substance_terms(
            &project_tool_calls(envelope),
            ScoreWitnessClass::Unattested,
            params,
        );
        (
            to_micros(composite(&terms, params)),
            to_micros(terms.work),
            to_micros(terms.replay),
        )
    };
    let (base_micros, base_work, base_replay) = score(&pair.base);
    let (variant_micros, variant_work, variant_replay) = score(&pair.variant);
    let expectation_met = match pair.expectation {
        GuardrailExpectation::Equal => variant_micros == base_micros,
        GuardrailExpectation::NotAbove => variant_micros <= base_micros,
        GuardrailExpectation::Below => variant_micros < base_micros,
    };
    let work_moved = base_work.abs_diff(variant_work);
    let replay_moved = base_replay.abs_diff(variant_replay);
    let most_moved = if work_moved == 0 && replay_moved == 0 {
        "none"
    } else if work_moved >= replay_moved {
        "work"
    } else {
        "replay"
    };
    GuardrailPairResult {
        met: expectation_met && variant_micros <= base_micros,
        base_micros,
        variant_micros,
        most_moved,
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct GuardrailTransformSummary {
    pub transform: GuardrailTransform,
    pub pair_count: u64,
    pub met_count: u64,
    pub most_moved: &'static str,
}

/// Check 1 on the synthetic bases: seed 1, [`GuardrailRecipe::v1`].
pub fn run_synthetic_guardrail(params: &SubstanceParams) -> Vec<GuardrailTransformSummary> {
    let pairs = generate_guardrail_pairs(
        &synthetic_guardrail_bases(),
        &GuardrailRecipe::v1(),
        SYNTHETIC_SEED,
    )
    .expect("the synthetic bases are eligible");
    GuardrailTransform::ALL
        .iter()
        .map(|&transform| {
            let mut summary = GuardrailTransformSummary {
                transform,
                pair_count: 0,
                met_count: 0,
                most_moved: "none",
            };
            let mut moved: BTreeMap<&'static str, u64> = BTreeMap::new();
            for pair in pairs.iter().filter(|pair| pair.transform == transform) {
                let result = check_guardrail_pair(pair, params);
                summary.pair_count += 1;
                summary.met_count += u64::from(result.met);
                *moved.entry(result.most_moved).or_default() += 1;
            }
            // The term that moved most often among the pairs; `none` on a tie
            // with nothing else.
            summary.most_moved = moved
                .iter()
                .filter(|(term, _)| **term != "none")
                .max_by_key(|(_, count)| **count)
                .map(|(term, _)| *term)
                .unwrap_or("none");
            summary
        })
        .collect()
}

/// Check 2: more paired calls never lower substance, over 0 to 40 calls.
pub fn monotonicity_sweep(params: &SubstanceParams) -> bool {
    let label = "monotonicity_sweep";
    let substance = |events: &[TraceContributionEvent]| {
        let envelope = calibration_envelope(label, events.to_vec());
        let terms = substance_terms(
            &project_tool_calls(&envelope),
            ScoreWitnessClass::Unattested,
            params,
        );
        composite(&terms, params)
    };
    let mut events = vec![calibration_event(
        &format!("{label}:prompt"),
        TraceContributionEventType::UserMessage,
        Some("fix the failing test".to_string()),
    )];
    let mut previous = substance(&events);
    for index in 1..=40usize {
        events.extend(paired_events(label, index, &format!("tool{}", index / 3)));
        let current = substance(&events);
        if current < previous {
            return false;
        }
        previous = current;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> SubstanceParams {
        SubstanceParams::rlvr_v1()
    }

    fn v1_pairs(seed: u64) -> Vec<GuardrailPair> {
        generate_guardrail_pairs(&synthetic_guardrail_bases(), &GuardrailRecipe::v1(), seed)
            .expect("eligible bases")
    }

    fn count(envelope: &TraceContributionEnvelope, kind: TraceContributionEventType) -> usize {
        envelope
            .events
            .iter()
            .filter(|event| event.event_type == kind)
            .count()
    }

    fn last_assistant_len(envelope: &TraceContributionEnvelope) -> usize {
        envelope
            .events
            .iter()
            .rev()
            .find(|event| event.event_type == TraceContributionEventType::AssistantMessage)
            .and_then(|event| event.redacted_content.as_ref())
            .map_or(0, |text| text.len())
    }

    fn pair_of(transform: GuardrailTransform, calls: usize) -> GuardrailPair {
        v1_pairs(1)
            .into_iter()
            .find(|pair| {
                pair.transform == transform
                    && pair.base_label == format!("synthetic_{calls:02}_calls")
            })
            .expect("pair")
    }

    #[test]
    fn synthetic_guardrail_pairs_meet_their_expectations() {
        let pairs = v1_pairs(1);
        assert_eq!(pairs.len(), 56);
        for pair in &pairs {
            let result = check_guardrail_pair(pair, &params());
            assert!(
                result.met,
                "{} on {} did not meet {:?}: base {} variant {}",
                pair.transform.label(),
                pair.base_label,
                pair.expectation,
                result.base_micros,
                result.variant_micros
            );
        }
        let summaries = run_synthetic_guardrail(&params());
        assert_eq!(summaries.len(), 7);
        for summary in summaries {
            assert_eq!(summary.pair_count, 8);
            assert_eq!(summary.met_count, 8);
        }
    }

    #[test]
    fn transforms_have_the_section_7_1_shape() {
        let calls = 5;
        let scaffold = pair_of(GuardrailTransform::AddScaffold, calls);
        let added = scaffold.variant.events.len() - scaffold.base.events.len();
        assert_eq!(added, 20);
        assert!(
            scaffold.variant.events[scaffold.base.events.len()..]
                .iter()
                .all(
                    |event| event.event_type == TraceContributionEventType::ToolCall
                        && event.tool_name.as_deref() == Some("scaffold")
                        && event.structured_payload.is_null()
                )
        );

        let truncated = pair_of(GuardrailTransform::TruncateHalf, calls);
        assert_eq!(
            truncated.variant.events.len(),
            truncated.base.events.len() / 2
        );

        let duplicated = pair_of(GuardrailTransform::DuplicateEvents, calls);
        assert_eq!(
            duplicated.variant.events.len(),
            duplicated.base.events.len() * 2
        );
        let mut ids: Vec<Uuid> = duplicated
            .variant
            .events
            .iter()
            .map(|event| event.event_id)
            .collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), duplicated.variant.events.len());
        let copy_ids: Vec<Uuid> = duplicated.variant.events[duplicated.base.events.len()..]
            .iter()
            .map(|event| event.event_id)
            .collect();
        assert!(
            duplicated.variant.events[duplicated.base.events.len()..]
                .iter()
                .filter_map(|event| event.parent_event_id)
                .all(|parent| copy_ids.contains(&parent))
        );

        let stripped = pair_of(GuardrailTransform::StripResults, calls);
        assert!(
            stripped
                .variant
                .events
                .iter()
                .filter(|event| event.event_type == TraceContributionEventType::ToolResult)
                .all(|event| event.redacted_content.is_none()
                    && event.structured_payload.is_null())
        );

        let asserted = pair_of(GuardrailTransform::AssertSuccessNoWork, calls);
        assert_eq!(
            count(&asserted.variant, TraceContributionEventType::ToolCall),
            0
        );
        assert_eq!(
            count(&asserted.variant, TraceContributionEventType::ToolResult),
            0
        );
        assert_eq!(asserted.variant.outcome.task_success, TaskSuccess::Success);

        let noisy = pair_of(GuardrailTransform::AppendNoise, calls);
        assert_eq!(
            last_assistant_len(&noisy.variant) - last_assistant_len(&noisy.base),
            4096usize.div_ceil(3) * 4
        );
    }

    #[test]
    fn a_base_with_an_unpaired_call_is_refused() {
        let mut bases = synthetic_guardrail_bases();
        let bare = apply_transform(
            GuardrailTransform::AddScaffold,
            &bases[0].1,
            &GuardrailRecipe::v1(),
            1,
        );
        assert!(!guardrail_base_eligible(&bare));
        bases[0].1 = bare;
        let refused = generate_guardrail_pairs(&bases, &GuardrailRecipe::v1(), 1);
        assert_eq!(refused.unwrap_err(), "guardrail_base_ineligible");
    }

    #[test]
    fn a_recipe_change_is_a_new_recipe_id() {
        let changed = GuardrailRecipe {
            scaffold_calls: 21,
            ..GuardrailRecipe::v1()
        };
        assert_ne!(changed.recipe_id(), GuardrailRecipe::v1().recipe_id());
        assert_eq!(
            GuardrailRecipe::v1().recipe_id(),
            GuardrailRecipe::v1().recipe_id()
        );
    }

    #[test]
    fn generation_is_deterministic() {
        let first = v1_pairs(1);
        let second = v1_pairs(1);
        assert_eq!(first.len(), second.len());
        for (a, b) in first.iter().zip(&second) {
            assert_eq!(a.variant, b.variant);
        }
        for (a, b) in first.iter().zip(&v1_pairs(2)) {
            if a.transform == GuardrailTransform::AppendNoise {
                assert_ne!(a.variant, b.variant);
            } else {
                assert_eq!(a.variant, b.variant);
            }
        }
    }

    #[test]
    fn the_monotonicity_sweep_passes_at_the_spec_values() {
        assert!(monotonicity_sweep(&SubstanceParams::rlvr_v1()));
    }
}
