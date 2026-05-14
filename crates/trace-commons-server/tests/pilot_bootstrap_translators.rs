//! Integration tests for the per-dataset translators that the
//! `trace-commons-pilot-bootstrap` binary uses. Modules under `src/bin/` aren't
//! reachable through the library crate, so we pull them in here via
//! `#[path]` to keep the production binary the single source of truth.

#[path = "../src/bin/pilot_bootstrap/hf_dataset.rs"]
mod hf_dataset;

#[path = "../src/bin/pilot_bootstrap/translators.rs"]
mod translators;

use hf_dataset::list_local_jsonl_sessions;
use serde_json::{Value, json};
use translators::{
    DeepSeekAgentTranslator, PiMonoTranslator, SubmissionDraft, SwivalTranslator, Translator,
    passes_word_filter, submission_id_from_body,
};

fn make_session(events: &[Value]) -> Vec<u8> {
    let mut out = Vec::new();
    for e in events {
        out.extend_from_slice(e.to_string().as_bytes());
        out.push(b'\n');
    }
    out
}

#[test]
fn swival_translator_concats_session_content_in_order() {
    let t = SwivalTranslator::new();
    let bytes = make_session(&[
        json!({"type":"session","id":"sess-1","timestamp":"2026-01-01T00:00:00Z"}),
        json!({
            "type":"message","id":"m1",
            "message":{"role":"user","content":[
                {"type":"text","text":"hello there"}
            ]}
        }),
        json!({
            "type":"message","id":"m2","parentId":"m1",
            "message":{"role":"assistant","content":"hi back"}
        }),
        json!({"type":"system","content":"system notice"}),
    ]);

    let a: SubmissionDraft = t.translate("sess-1.jsonl", &bytes).unwrap();
    let b: SubmissionDraft = t.translate("sess-1.jsonl", &bytes).unwrap();
    assert_eq!(a, b, "same session bytes must yield identical draft");
    assert_eq!(a.trace_body, "hello there\n\nhi back\n\nsystem notice");
    assert_eq!(a.source_dataset, "jedisct1/agent-traces-swival");
    assert_eq!(a.source_row_id, "sess-1");
    assert_eq!(a.source_domain_tag, "agent-traces/swival");
    assert_eq!(a.submission_id, submission_id_from_body(&a.trace_body));
}

#[test]
fn swival_translator_handles_empty_session() {
    let t = SwivalTranslator::new();
    let bytes = make_session(&[
        json!({"type":"model_change","id":"a","provider":"x"}),
        json!({"type":"thinking_level_change","id":"b"}),
    ]);
    let err = t.translate("empty.jsonl", &bytes).err().expect("err");
    assert!(
        err.to_string().contains("no textual content"),
        "unexpected error: {err}"
    );
}

#[test]
fn swival_translator_skips_malformed_lines() {
    let t = SwivalTranslator::new();
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"not json\n");
    bytes.extend_from_slice(b"{broken\n");
    bytes.extend_from_slice(
        json!({"type":"m","content":"good content"})
            .to_string()
            .as_bytes(),
    );
    bytes.push(b'\n');
    let draft = t.translate("mixed.jsonl", &bytes).unwrap();
    assert!(draft.trace_body.contains("good content"));
}

#[test]
fn submission_id_is_stable_across_runs() {
    let t = SwivalTranslator::new();
    let bytes = make_session(&[
        json!({"type":"session","id":"S"}),
        json!({"type":"message","message":{"role":"user","content":"hello"}}),
        json!({"type":"message","message":{"role":"assistant","content":"world"}}),
    ]);
    let a = t.translate("a.jsonl", &bytes).unwrap();
    let b = t.translate("a.jsonl", &bytes).unwrap();
    let c = t.translate("different-name.jsonl", &bytes).unwrap();
    assert_eq!(a.submission_id, b.submission_id);
    assert_eq!(a.submission_id, c.submission_id);
}

#[test]
fn jsonl_shard_discovery_returns_session_files_in_deterministic_order() {
    let tmp = tempfile::tempdir().unwrap();
    for name in &["20.jsonl", "01.jsonl", "10.jsonl", "ignore.txt"] {
        std::fs::write(tmp.path().join(name), b"{}\n").unwrap();
    }
    let sessions = list_local_jsonl_sessions(tmp.path()).unwrap();
    let names: Vec<_> = sessions.iter().map(|s| s.sibling_name.clone()).collect();
    assert_eq!(names, vec!["01.jsonl", "10.jsonl", "20.jsonl"]);
}

#[test]
fn pi_mono_translator_uses_pi_mono_labels() {
    let t = PiMonoTranslator::new();
    let bytes = make_session(&[
        json!({"type":"session","id":"pm-1"}),
        json!({"type":"message","message":{"role":"user","content":"pi prompt"}}),
    ]);
    let draft = t.translate("pm-1.jsonl", &bytes).unwrap();
    assert_eq!(draft.source_dataset, "badlogicgames/pi-mono");
    assert_eq!(draft.source_domain_tag, "agent-traces/pi-mono");
    assert_eq!(draft.source_row_id, "pm-1");
    assert!(draft.trace_body.contains("pi prompt"));
}

#[test]
fn deepseek_agent_translator_uses_deepseek_labels() {
    let t = DeepSeekAgentTranslator::new();
    let bytes = make_session(&[
        json!({"type":"session","id":"ds-1"}),
        json!({
            "type":"message",
            "message":{"role":"assistant","content":[
                {"type":"text","text":"part 1"},
                {"type":"text","text":"part 2"}
            ]}
        }),
    ]);
    let draft = t.translate("ds-1.jsonl", &bytes).unwrap();
    assert_eq!(draft.source_dataset, "TeichAI/DeepSeek-v4-Pro-Agent");
    assert_eq!(draft.source_domain_tag, "agent-traces/deepseek-v4-pro");
    assert!(draft.trace_body.contains("part 1"));
    assert!(draft.trace_body.contains("part 2"));
}

#[test]
fn word_filter_screens_short_and_long_sessions() {
    let body_short = "one two three";
    let body_ok = (0..50)
        .map(|i| format!("word{i}"))
        .collect::<Vec<_>>()
        .join(" ");
    assert!(!passes_word_filter(body_short, 10, 100));
    assert!(passes_word_filter(&body_ok, 10, 100));
    assert!(!passes_word_filter(&body_ok, 10, 49));
}
