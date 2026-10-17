// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Integration tests for the per-dataset translators that the
//! `trace-commons-pilot-bootstrap` binary uses. Modules under `src/bin/` aren't
//! reachable through the library crate, so we pull them in here via
//! `#[path]` to keep the production binary the single source of truth.

#[path = "../src/bin/pilot_bootstrap/hf_dataset.rs"]
mod hf_dataset;

#[path = "../src/bin/pilot_bootstrap/translators.rs"]
mod translators;

use serde_json::{Value, json};
use translators::{DeepSeekAgentTranslator, PiMonoTranslator, Translator};

fn make_session(events: &[Value]) -> Vec<u8> {
    let mut out = Vec::new();
    for e in events {
        out.extend_from_slice(e.to_string().as_bytes());
        out.push(b'\n');
    }
    out
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
