//! Additive notification presentations: selected keys, opaque subjects and
//! backward-compatible English across the public localized ABI.

use std::collections::BTreeMap;
use std::ffi::{CStr, CString};

use serde_json::{Value, json};
use trace_commons_contributor::{localized_copy, nudge_copy as copy, nudge_render};
use trace_commons_contributor_ffi::{tc_localized_presentation_json, tc_string_free};

fn presentation(operation: &str, input: Value) -> Value {
    let operation = CString::new(operation).unwrap();
    let input = CString::new(input.to_string()).unwrap();
    let pointer = unsafe { tc_localized_presentation_json(operation.as_ptr(), input.as_ptr()) };
    assert!(!pointer.is_null(), "known notification operation");
    let encoded = unsafe { CStr::from_ptr(pointer) }
        .to_str()
        .unwrap()
        .to_owned();
    unsafe { tc_string_free(pointer) };
    serde_json::from_str(&encoded).unwrap()
}

fn render(tree: &Value) -> Value {
    if tree.get("key").is_some() {
        return json!(localized_copy::english(tree));
    }
    match tree {
        Value::Object(fields) => {
            Value::Object(fields.iter().map(|(k, v)| (k.clone(), render(v))).collect())
        }
        Value::Array(values) => Value::Array(values.iter().map(render).collect()),
        _ => tree.clone(),
    }
}

fn legacy(mut value: Value) -> Value {
    value.as_object_mut().unwrap().remove("presentation");
    value
}

#[test]
fn idle_and_backlog_messages_carry_typed_counts_and_opaque_tool_names() {
    for count in [0, 1, 2, 20] {
        for days in [0, 1, 3] {
            for names in [
                vec![],
                vec!["{days} %@"],
                vec!["{b}", "{count}"],
                vec!["A", "B", "C"],
            ] {
                let tools = names.into_iter().map(str::to_owned).collect::<Vec<_>>();
                let input = json!({"count": count, "idle_days": days, "tools": tools});
                let batch = nudge_render::Batch {
                    count,
                    tools,
                    idle_days: days,
                    mission_fit: None,
                    estimate: None,
                };
                for (operation, expected) in [
                    (
                        "notification_idle",
                        serde_json::to_value(nudge_render::idle_notification(&batch)).unwrap(),
                    ),
                    (
                        "nudge_idle",
                        serde_json::to_value(nudge_render::idle_card(&batch)).unwrap(),
                    ),
                    (
                        "nudge_backlog",
                        serde_json::to_value(nudge_render::backlog_card(&batch)).unwrap(),
                    ),
                    (
                        "nudge_mark_ready",
                        serde_json::to_value(nudge_render::mark_ready_text(&batch)).unwrap(),
                    ),
                ] {
                    assert_eq!(
                        render(&presentation(operation, input.clone())),
                        legacy(expected)
                    );
                }
                assert_eq!(
                    localized_copy::english(&presentation("digest_idle", input)),
                    nudge_render::digest_idle_sentence(&batch)
                );
            }
        }
    }
    let message = presentation(
        "notification_idle",
        json!({"count": 2, "idle_days": 3, "tools": ["{days}"]}),
    );
    assert_eq!(message["body"]["parameters"]["count"], 2);
    assert_eq!(message["body"]["parameters"]["tool"], "{days}");
    assert!(localized_copy::english(&message["body"]).contains("{days}"));
    assert_eq!(message["actions"][0]["id"], "review");
    assert_eq!(message["actions"][1]["id"], "not_now");
}

#[test]
fn verdict_selection_drops_zero_clauses_and_carries_calendar_dates_and_credit() {
    for accepted in [0, 1, 2] {
        for held in [0, 1, 2] {
            for credit in [None, Some(15)] {
                let input = json!({"accepted": accepted, "held": held, "credit_final_tenths": credit, "since": "2026-10-08"});
                let date = "2026-10-08".parse().unwrap();
                let verdicts = nudge_render::Verdicts {
                    accepted,
                    held,
                    credit_final_tenths: credit,
                    since: date,
                };
                for (operation, expected) in [
                    (
                        "notification_verdicts",
                        serde_json::to_value(nudge_render::verdicts_notification(&verdicts))
                            .unwrap(),
                    ),
                    (
                        "nudge_verdicts",
                        serde_json::to_value(nudge_render::verdicts_card(&verdicts)).unwrap(),
                    ),
                    (
                        "nudge_mark_news",
                        serde_json::to_value(nudge_render::mark_news_text(&verdicts)).unwrap(),
                    ),
                ] {
                    assert_eq!(
                        render(&presentation(operation, input.clone())),
                        legacy(expected)
                    );
                }
                assert_eq!(
                    localized_copy::english(&presentation("digest_verdicts", input)),
                    nudge_render::digest_verdict_sentence(&verdicts)
                );
            }
        }
    }
    let card = presentation(
        "nudge_verdicts",
        json!({"accepted": 1, "held": 0, "since": "2026-10-08", "credit_final_tenths": 15}),
    );
    assert_eq!(
        localized_copy::english(&card["title"]),
        "Since 8 October: 1 accepted"
    );
    assert_eq!(card["title"]["parameters"]["date"]["date"], "2026-10-08");
    assert_eq!(card["title"]["parameters"]["date"]["english"], "8 October");
    assert_eq!(card["body"]["parameters"]["x"]["decimal"], "1.5");
    let final_only = presentation(
        "notification_verdicts",
        json!({"accepted": 0, "held": 0, "since": "2026-10-08", "credit_final_tenths": 15}),
    );
    assert_eq!(
        localized_copy::english(&final_only["body"]),
        "1.5 credit is now final."
    );
}

#[test]
fn settlement_and_received_credit_preserve_canonical_rounding_and_currency() {
    for (low, high) in [(0, 500_000), (3_500_000, 9_000_000), (u64::MAX, u64::MAX)] {
        for pricing in [false, true] {
            let tree = presentation(
                "notification_settlement",
                json!({"low_usd_micros": low, "high_usd_micros": high, "pricing_changed": pricing}),
            );
            assert_eq!(tree["body"]["parameters"]["high"]["code"], "USD");
            let total = trace_commons_protocol::settlement_estimate::SettlementTotal {
                low_usd_micros: low,
                high_usd_micros: high,
                known: 1,
                unknown: 0,
            };
            assert_eq!(
                render(&tree),
                legacy(
                    serde_json::to_value(nudge_render::settlement_notification(&total, pricing))
                        .unwrap()
                )
            );
        }
    }
    let tree = presentation(
        "notification_settlement",
        json!({"low_usd_micros": 3_500_000, "high_usd_micros": 9_000_000}),
    );
    assert_eq!(tree["body"]["parameters"]["low"]["currency"], "3.50");
    assert_eq!(tree["body"]["parameters"]["high"]["currency"], "9");
    assert_eq!(
        localized_copy::english(&tree["body"]),
        "About $3.50 to $9 of inference, estimated on this Mac. Nothing was sent."
    );
    for nanos in [1, 10_000_000, 1_234_567_890] {
        let tree = presentation(
            "notification_near_ai_credit",
            json!({"received_nanos": nanos}),
        );
        assert_eq!(
            render(&tree),
            legacy(serde_json::to_value(nudge_render::near_ai_credit_notification(nanos)).unwrap())
        );
        assert_eq!(tree["title"]["english"], copy::NOTIFY_NEAR_AI_CREDIT_TITLE);
    }
}

#[test]
fn digest_sentences_keep_original_project_policy_credit_states_and_fold_order() {
    let pending = presentation(
        "digest_pending",
        json!({"count": 5, "projects": ["d", "c", "b", "a", ""]}),
    );
    assert_eq!(
        localized_copy::english(&pending),
        "5 traces ready to contribute from a, b, c and 1 more"
    );
    let contributed = presentation(
        "digest_contribution",
        json!({"count": 1, "projects": ["{count} {x}", " "], "credit_final": 0.35, "credit_pending": 6.0}),
    );
    assert_eq!(
        localized_copy::english(&contributed),
        "1 trace contributed from {count} {x}. 0.4 credit earned. 6.0 credit pending."
    );
    let multiple = presentation(
        "digest_contribution",
        json!({"count": 2, "projects": ["a", "b"], "credit_final": 0.04, "credit_pending": 0.0}),
    );
    assert_eq!(localized_copy::english(&multiple), "2 traces contributed.");
    let fold = presentation(
        "digest_verdicts",
        json!({"accepted": 1, "held": 0, "credit_final_tenths": 15, "since": "2026-10-08"}),
    );
    let complete = localized_copy::notifications::join(
        localized_copy::notifications::join(pending, multiple, true),
        fold,
        true,
    );
    assert_eq!(
        localized_copy::english(&complete),
        "5 traces ready to contribute from a, b, c and 1 more\n2 traces contributed.\n1 trace accepted. 1.5 credit is now final."
    );
}

#[test]
fn all_explicit_notification_variants_are_exported_for_catalog_generation() {
    let templates: BTreeMap<_, _> = localized_copy::templates()
        .into_iter()
        .filter_map(|message| {
            message["key"]
                .as_str()
                .map(str::to_owned)
                .map(|key| (key, message))
        })
        .collect();
    fn check(tree: &Value, templates: &BTreeMap<String, Value>) {
        if let Some(key) = tree["key"].as_str() {
            assert_eq!(
                templates.get(key).map(|m| &m["english"]),
                Some(&tree["english"]),
                "{key}"
            );
        }
        match tree {
            Value::Object(fields) => {
                for child in fields.values() {
                    check(child, templates);
                }
            }
            Value::Array(values) => {
                for child in values {
                    check(child, templates);
                }
            }
            _ => {}
        }
    }
    for tree in localized_copy::notifications::templates() {
        check(&tree, &templates);
    }
    for hours in [1, 4] {
        let tree = presentation("digest_help", json!({"interval_secs": hours * 3600}));
        check(&tree, &templates);
        assert_eq!(tree["parameters"]["count"], hours);
    }
}
