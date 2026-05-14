//! Integration tests for the per-dataset translators that the
//! `tracedao-pilot-bootstrap` binary uses. Modules under `src/bin/` aren't
//! reachable through the library crate, so we pull them in here via
//! `#[path]` to keep the production binary the single source of truth.

#[path = "../src/bin/pilot_bootstrap/hf_dataset.rs"]
mod hf_dataset;

#[path = "../src/bin/pilot_bootstrap/translators.rs"]
mod translators;

use std::collections::BTreeMap;

use hf_dataset::Row;
use serde_json::{json, Value};
use translators::{
    submission_id_from_body, DeepSeekAgentTranslator, PiMonoTranslator, SubmissionDraft,
    SwivalTranslator, Translator, SWIVAL_SOURCE_CODE_CAP,
};

fn row_from(fields: &[(&str, Value)]) -> Row {
    let mut map = BTreeMap::new();
    for (k, v) in fields {
        map.insert((*k).to_string(), v.clone());
    }
    Row { fields: map }
}

#[test]
fn swival_translator_produces_deterministic_id_for_same_input() {
    let t = SwivalTranslator::new();
    let row = row_from(&[
        ("title", json!("Reentrancy in withdraw")),
        ("severity", json!("high")),
        ("finding_type", json!("reentrancy")),
        ("proof", json!(["external call before state update", "ETH balance left mutable"])),
        ("fix_outline", json!("apply checks-effects-interactions")),
        ("source_code", json!("function withdraw() { ... }")),
    ]);

    let a = t.translate(&row).expect("translate ok");
    let b = t.translate(&row).expect("translate ok");
    assert_eq!(a, b, "same row must yield identical draft");
    assert_eq!(
        a.submission_id,
        submission_id_from_body(&a.trace_body),
        "id is the body content hash prefix"
    );
    assert_eq!(a.source_dataset, "jedisct1/agent-traces-swival");
    assert_eq!(a.source_domain_tag, "security-audit/reentrancy");
}

#[test]
fn swival_translator_handles_missing_fields_gracefully() {
    let t = SwivalTranslator::new();
    let row = row_from(&[("title", json!("Only a title"))]);

    let draft: SubmissionDraft = t.translate(&row).expect("translate ok");
    assert!(draft.trace_body.contains("Only a title"));
    // Missing finding_type collapses to `security-audit/`.
    assert_eq!(draft.source_domain_tag, "security-audit/");
    // ID is still a deterministic hex prefix of fixed length.
    assert_eq!(draft.submission_id.len(), 32);
}

#[test]
fn swival_translator_truncates_long_source_code() {
    let t = SwivalTranslator::new();
    let long = "a".repeat(SWIVAL_SOURCE_CODE_CAP * 4);
    let row = row_from(&[
        ("title", json!("long")),
        ("severity", json!("low")),
        ("finding_type", json!("style")),
        ("source_code", json!(long)),
    ]);

    let draft = t.translate(&row).expect("translate ok");
    // The body header + truncated source_code; the source_code portion must
    // not exceed the cap.
    let trailing_a = draft.trace_body.chars().filter(|c| *c == 'a').count();
    assert!(
        trailing_a <= SWIVAL_SOURCE_CODE_CAP,
        "source_code truncation honored: {} <= {}",
        trailing_a,
        SWIVAL_SOURCE_CODE_CAP
    );
}

#[test]
fn pi_mono_translator_is_stubbed_in_slice_3() {
    let t = PiMonoTranslator::new();
    let row = row_from(&[("messages", json!([]))]);
    assert!(t.translate(&row).is_err(), "stub returns Err in Slice 3");
}

#[test]
fn deepseek_agent_translator_is_stubbed_in_slice_3() {
    let t = DeepSeekAgentTranslator::new();
    let row = row_from(&[("messages", json!([]))]);
    assert!(t.translate(&row).is_err(), "stub returns Err in Slice 3");
}
