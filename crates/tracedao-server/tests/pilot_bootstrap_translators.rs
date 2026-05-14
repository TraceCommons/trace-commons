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
use serde_json::{Value, json};
use translators::{
    DeepSeekAgentTranslator, PiMonoTranslator, SWIVAL_SOURCE_CODE_CAP, SubmissionDraft,
    SwivalTranslator, Translator, submission_id_from_body,
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
        (
            "proof",
            json!([
                "external call before state update",
                "ETH balance left mutable"
            ]),
        ),
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
fn pi_mono_translator_picks_longest_chain() {
    let t = PiMonoTranslator::new();
    // Tree:
    //   root1
    //     a (deep chain ends here)
    //   root2
    //     b
    //       c
    //         d (longest chain: root2 -> b -> c -> d)
    let row = row_from(&[
        (
            "messages",
            json!([
                {"id": "root1", "content": "root1-text"},
                {"id": "a", "parentId": "root1", "content": "a-text"},
                {"id": "root2", "content": "root2-text"},
                {"id": "b", "parentId": "root2", "content": "b-text"},
                {"id": "c", "parentId": "b", "content": "c-text"},
                {"id": "d", "parentId": "c", "content": "d-text"},
            ]),
        ),
        ("session_id", json!("S1")),
    ]);

    let draft = t.translate(&row).expect("translate ok");
    assert!(draft.trace_body.contains("root2-text"));
    assert!(draft.trace_body.contains("d-text"));
    assert!(!draft.trace_body.contains("a-text"));
    assert_eq!(draft.source_dataset, "badlogicgames/pi-mono");
    assert_eq!(draft.source_row_id, "S1");
    assert!(draft.source_domain_tag.starts_with("coding-session/"));
}

#[test]
fn pi_mono_translator_handles_flat_messages() {
    let t = PiMonoTranslator::new();
    // No parent ids — all roots, fallback to linear concatenation.
    let row = row_from(&[(
        "messages",
        json!([
            {"id": "1", "content": "first"},
            {"id": "2", "content": "second"},
        ]),
    )]);
    let draft = t.translate(&row).expect("translate ok");
    // All-root case: longest single-node chain is one message; the rest are
    // dropped. This is acceptable for v1 and matches the spec's "longest
    // top-level session" framing.
    assert!(draft.trace_body.contains("first") || draft.trace_body.contains("second"));
}

#[test]
fn pi_mono_translator_errors_on_missing_messages() {
    let t = PiMonoTranslator::new();
    let row = row_from(&[("session_id", json!("S1"))]);
    assert!(t.translate(&row).is_err());
}

#[test]
fn deepseek_agent_translator_concatenates_assistant_text() {
    let t = DeepSeekAgentTranslator::new();
    let row = row_from(&[(
        "messages",
        json!([
            {"role": "user", "content": "user-prompt"},
            {"role": "assistant", "content": "first-assistant"},
            {"role": "tool", "content": "tool-output"},
            {"role": "assistant", "content": "second-assistant"},
        ]),
    )]);
    let draft = t.translate(&row).expect("translate ok");
    assert!(draft.trace_body.contains("first-assistant"));
    assert!(draft.trace_body.contains("second-assistant"));
    assert!(!draft.trace_body.contains("user-prompt"));
    assert!(!draft.trace_body.contains("tool-output"));
    assert_eq!(draft.source_dataset, "TeichAI/DeepSeek-v4-Pro-Agent");
    assert!(draft.source_domain_tag.starts_with("agent-reasoning/"));
}

#[test]
fn deepseek_agent_translator_handles_array_content() {
    let t = DeepSeekAgentTranslator::new();
    let row = row_from(&[(
        "messages",
        json!([
            {"role": "assistant", "content": [
                {"type": "text", "text": "part-1"},
                {"type": "text", "text": "part-2"},
            ]},
        ]),
    )]);
    let draft = t.translate(&row).expect("translate ok");
    assert!(draft.trace_body.contains("part-1"));
    assert!(draft.trace_body.contains("part-2"));
}

#[test]
fn deepseek_agent_translator_errors_on_no_assistant_text() {
    let t = DeepSeekAgentTranslator::new();
    let row = row_from(&[(
        "messages",
        json!([
            {"role": "user", "content": "only user"},
        ]),
    )]);
    assert!(t.translate(&row).is_err());
}
