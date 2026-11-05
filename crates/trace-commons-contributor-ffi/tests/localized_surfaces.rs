//! Public presentation contracts for routing, witness results and nudge tags.

use std::ffi::{CStr, CString};

use serde_json::{Value, json};
use trace_commons_contributor::{localized_copy, preview_copy, routing_copy, witness_copy};
use trace_commons_contributor_ffi::{tc_localized_presentation_json, tc_string_free};

fn presentation(operation: &str, input: &str) -> Value {
    let operation = CString::new(operation).unwrap();
    let input = CString::new(input).unwrap();
    let pointer = unsafe { tc_localized_presentation_json(operation.as_ptr(), input.as_ptr()) };
    assert!(!pointer.is_null(), "supported presentation");
    let encoded = unsafe { CStr::from_ptr(pointer) }
        .to_str()
        .unwrap()
        .to_owned();
    unsafe { tc_string_free(pointer) };
    serde_json::from_str(&encoded).unwrap()
}

#[test]
fn routing_preserves_legacy_wiring_and_literal_paths() {
    for (code, wiring) in [
        (0, routing_copy::ToolWiring::Wired),
        (1, routing_copy::ToolWiring::NotWired),
        (2, routing_copy::ToolWiring::Unknown),
        (-1, routing_copy::ToolWiring::Unknown),
    ] {
        for mode in ["off", "watch", "future"] {
            let message = presentation(
                "routing_tool_word",
                &json!({"source_mode":mode,"wiring":code}).to_string(),
            );
            assert_eq!(
                localized_copy::english(&message),
                routing_copy::tool_word(mode, wiring)
            );
        }
    }
    let path = "/tmp/{count}/日本語/%lld";
    let message = presentation("routing_token_line", &json!({"path":path}).to_string());
    assert_eq!(message["parameters"]["path"], path);
    assert_eq!(
        localized_copy::english(&message),
        routing_copy::ironwire_token_line(Some(path))
    );
    let folder = presentation("routing_folder_note", "{}");
    assert_eq!(
        localized_copy::english(&folder),
        routing_copy::ironwire_folder_note_here()
    );
    for port in [0, 8463, 65535] {
        let message = presentation("routing_discovery_line", &json!({"port":port}).to_string());
        assert_eq!(
            localized_copy::english(&message),
            routing_copy::ironwire_discovery_line((port > 0).then_some(port as u16))
        );
    }
}

#[test]
fn witness_and_decision_counts_keep_independent_numeric_facts() {
    for count in [0, 1, 2, 1001] {
        let message = presentation(
            "witness_pinned_count_line",
            &json!({"count":count}).to_string(),
        );
        assert_eq!(
            localized_copy::english(&message),
            witness_copy::witness_pinned_count_line(count)
        );
        let owed = presentation("decisions_owed_text", &json!({"count":count}).to_string());
        assert_eq!(
            localized_copy::english(&owed),
            preview_copy::decisions_owed_text(Some(count as u64))
        );
    }
    for (n, m) in [(1, 7), (3, 1), (0, 0)] {
        let message = presentation(
            "witness_last_result_line",
            &json!({"outcome":"certified","n_of_m":{"n":n,"m":m}}).to_string(),
        );
        let expected = witness_copy::witness_last_result_line(
            &trace_commons_contributor::witness::status::WitnessLastResult::Certified {
                n_of_m: Some(
                    trace_commons_contributor::witness::status::InferenceReceiptCount { n, m },
                ),
            },
        );
        assert_eq!(localized_copy::english(&message), expected);
        assert_eq!(message["parameters"]["receipts"], n);
        assert_eq!(message["parameters"]["count"], m);
    }
    let busy = presentation(
        "witness_review_failure_json",
        r#"{"label":"witness_saturated","view":{"state":"Busy","retry_at":"2026-11-01T12:00:00Z","raw":"{count}"}}"#,
    );
    assert_eq!(busy["state"], "Busy");
    assert_eq!(busy["retry_at"], "2026-11-01T12:00:00Z");
    assert_eq!(busy["raw"], "{count}");
    assert_eq!(
        localized_copy::english(&busy["message"]),
        witness_copy::witness_copy().review.failed_busy
    );
}

#[test]
fn nudge_tags_preserve_suppression_rounding_and_malformed_input_behavior() {
    for operation in ["nudge_entry_tags_json", "nudge_digest_help_json"] {
        for input in ["{}", "broken", "null", "[]"] {
            assert_eq!(presentation(operation, input), json!({}));
        }
    }
    let input = json!({"mission_fit":2,"settlement_estimate":{"low_usd_micros":1_239_999,"high_usd_micros":2_000_001,"tier":"higher","drawn":true}});
    let tags = presentation("nudge_entry_tags_json", &input.to_string());
    let expected = trace_commons_contributor::nudge_render::entry_tags(
        Some(2),
        Some(&trace_commons_contributor::nudge_render::EntryUsdEstimate {
            low_usd_micros: 1_239_999,
            high_usd_micros: 2_000_001,
            tier: Some("higher".into()),
            drawn: true,
        }),
    );
    assert_eq!(
        localized_copy::english(&tags["estimate_band"]),
        expected.estimate_band.unwrap()
    );
    assert_eq!(
        tags["estimate_band"]["parameters"]["low"]["currency"],
        "1.23"
    );
    assert_eq!(tags["estimate_band"]["parameters"]["high"]["currency"], "2");
    let hidden = presentation(
        "nudge_entry_tags_json",
        r#"{"settlement_estimate":{"low_usd_micros":200,"high_usd_micros":100,"drawn":true}}"#,
    );
    assert_eq!(hidden, json!({}));
}
