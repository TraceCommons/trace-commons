//! Public localized ABI regressions: English compatibility, facts and keys.

use std::ffi::{CStr, CString};

use serde_json::{Value, json};
use trace_commons_contributor::{localized_copy, private_inference_copy as copy};
use trace_commons_contributor_ffi::{tc_localized_presentation_json, tc_string_free};

fn presentation(operation: &str, input: Value) -> Value {
    let operation = CString::new(operation).unwrap();
    let input = CString::new(input.to_string()).unwrap();
    let pointer = unsafe { tc_localized_presentation_json(operation.as_ptr(), input.as_ptr()) };
    if pointer.is_null() {
        return Value::Null;
    }
    let encoded = unsafe { CStr::from_ptr(pointer) }
        .to_str()
        .unwrap()
        .to_owned();
    unsafe { tc_string_free(pointer) };
    let presentation: Value = serde_json::from_str(&encoded).expect("valid owned JSON");
    assert!(
        presentation.is_object(),
        "a non-null result must be a message tree"
    );
    presentation
}

fn english(operation: &str, input: Value) -> String {
    localized_copy::english(&presentation(operation, input))
}

#[test]
fn state_keys_follow_facts_not_shared_english_and_future_states_are_bounded() {
    for state in copy::STATE_LABELS.into_iter().chain(["", "future_state"]) {
        for (operation, expected) in [
            ("private_inference_state_line", copy::state_line(state)),
            ("private_inference_runtime_word", copy::runtime_word(state)),
            ("private_inference_menu_word", copy::menu_state(state).0),
        ] {
            let message = presentation(operation, json!({"state": state}));
            assert_eq!(localized_copy::english(&message), expected);
            assert_eq!(message["parameters"], json!({}));
        }
    }
    let first = presentation(
        "private_inference_menu_word",
        json!({"state": copy::LABEL_RUNNING}),
    );
    let second = presentation(
        "private_inference_menu_word",
        json!({"state": copy::LABEL_RUNNING_NO_BACKENDS}),
    );
    assert_eq!(first["english"], second["english"]);
    assert_ne!(first["key"], second["key"]);
    for operation in [
        "private_inference_state_line",
        "near_ai_credential_state_line",
        "near_ai_balance_state_line",
        "harness_state_line",
        "near_ai_enroll_line",
        "queue_outcome_line",
    ] {
        let a = presentation(operation, json!({"state": "future_a", "label": "future_a"}));
        let b = presentation(operation, json!({"state": "future_b", "label": "future_b"}));
        assert_eq!(
            a, b,
            "unknown labels must use a stable fallback: {operation}"
        );
        assert_eq!(a["key"], format!("{operation}.fallback"));
    }
}

#[test]
fn contribution_reasons_and_refusals_retain_privacy_and_send_semantics() {
    use trace_commons_contributor::daemon::{attestation_mark, contribution_eligibility};
    for state in contribution_eligibility::ALL_STATES {
        assert_eq!(
            english("contribution_eligibility_line", json!({"state": state})),
            copy::eligibility_state_line(state)
        );
    }
    for mark in attestation_mark::ALL_MARKS {
        assert_eq!(
            english("contribution_attestation_line", json!({"mark": mark})),
            copy::attestation_state_line(mark)
        );
    }
    for reason in attestation_mark::ALL_REASONS
        .into_iter()
        .chain(["", "future_reason"])
    {
        assert_eq!(
            english(
                "contribution_eligibility_reason_line",
                json!({"reason": reason})
            ),
            copy::eligibility_reason_line(reason)
        );
        assert_eq!(
            english(
                "contribution_attestation_reason_line",
                json!({"reason": reason})
            ),
            copy::attestation_reason_line(reason)
        );
    }
    for refusal in trace_commons_protocol::admission::AdmissionRefusal::ALL {
        let label = refusal.label();
        let expected = copy::outcome_refusal_line(label).unwrap();
        assert_eq!(
            english("outcome_refusal_line", json!({"label": label})),
            expected
        );
        assert_eq!(
            english("queue_outcome_line", json!({"label": label})),
            expected
        );
    }
    assert!(presentation("outcome_refusal_line", json!({"label": "future_reason"})).is_null());
    for label in [
        "dismissed-by-contributor",
        "queue-full",
        "witness-not-trusted",
        "",
        "future_reason",
    ] {
        assert_eq!(
            english("queue_outcome_line", json!({"label": label})),
            copy::queue_outcome_line(label)
        );
    }
    for count in [0, 1, 2, u64::MAX] {
        let message = presentation("contribution_withheld_line", json!({"withheld": count}));
        assert_eq!(
            localized_copy::english(&message),
            copy::group_withheld_line(count)
        );
        if count > 0 {
            assert_eq!(message["parameters"]["count"].as_u64(), Some(count));
            assert!(message["english"]["one"].is_string());
            assert!(message["english"]["other"].is_string());
        }
    }
    assert_eq!(
        english("contribution_withheld_line", json!({"withheld": -1})),
        ""
    );
}

#[test]
fn ages_use_numeric_native_plural_selectors_at_every_bucket_boundary() {
    for seconds in [
        None,
        Some(0),
        Some(59),
        Some(60),
        Some(119),
        Some(120),
        Some(3599),
        Some(3600),
        Some(7200),
        Some(86399),
        Some(86400),
        Some(172800),
        Some(i64::MAX as u64),
    ] {
        for (operation, expected) in [
            (
                "harness_last_call_line",
                copy::harness_last_call_line(seconds),
            ),
            (
                "near_ai_balance_observed_line",
                copy::balance_observed_line(seconds),
            ),
        ] {
            assert_eq!(
                english(operation, json!({"seconds_ago": seconds})),
                expected
            );
        }
    }
    let minute = presentation("harness_last_call_line", json!({"seconds_ago": 60}));
    assert_eq!(minute["parameters"]["count"], 1);
    assert_eq!(minute["key"], "harness_last_call_line.minute");
    assert_eq!(minute["english"]["other"], "Last call: {count} minutes ago");
    assert_eq!(
        english("harness_last_call_line", json!({"seconds_ago": -1})),
        ""
    );
    for port in [-1, 0, 1, 8463, 65535, 65536] {
        let expected = copy::serving_line(u16::try_from(port).ok());
        assert_eq!(
            english("private_inference_serving_line", json!({"port": port})),
            expected
        );
    }
}

#[test]
fn unavailable_is_never_zero_and_money_is_rounded_once_in_core() {
    for nanos in [
        None,
        Some(i64::MIN),
        Some(-9_996_000_000),
        Some(-1),
        Some(0),
        Some(1),
        Some(9_999_999),
        Some(18_185_000_000),
        Some(i64::MAX),
    ] {
        for scale in [0, 2, 6, 9, 12, 38, 39, 200] {
            let input = json!({"nanos": nanos, "scale": scale});
            assert_eq!(
                english("near_ai_balance_remaining_line", input.clone()),
                copy::balance_remaining_line(nanos, scale)
            );
            assert_eq!(
                english("near_ai_balance_limit_line", input.clone()),
                copy::balance_limit_line(nanos, scale)
            );
            assert_eq!(
                english("near_ai_balance_amount", input.clone()),
                nanos
                    .map(|n| copy::format_amount(n, scale))
                    .unwrap_or_default()
            );
            for remaining in [None, Some(111_815_000_000)] {
                let mut spent = input.clone();
                spent["remaining"] = json!(remaining);
                assert_eq!(
                    english("near_ai_balance_spent_line", spent),
                    copy::balance_spent_line(nanos, remaining, scale)
                );
            }
        }
    }
    assert_eq!(
        english(
            "near_ai_balance_remaining_line",
            json!({"nanos": null, "scale": 9})
        ),
        "No spending limit set."
    );
    assert_eq!(
        english(
            "near_ai_balance_remaining_line",
            json!({"nanos": 0, "scale": 9})
        ),
        "$0.00 left"
    );
    assert_eq!(
        english(
            "near_ai_balance_remaining_line",
            json!({"nanos": 111_815_000_000_i64, "scale": 9})
        ),
        "$111.81 left"
    );
    let spent = presentation(
        "near_ai_balance_spent_line",
        json!({"nanos": 18_185_000_000_i64, "remaining": 111_815_000_000_i64, "scale": 9}),
    );
    assert_eq!(localized_copy::english(&spent), "Spent: $18.19");
    assert_eq!(
        spent["parameters"]["amount"]["parameters"]["amount"],
        json!({"currency": "18.19", "code": "USD", "english": "$18.19"})
    );
    assert_eq!(
        english(
            "near_ai_balance_spent_line",
            json!({"nanos": 1, "remaining": 0, "scale": 9})
        ),
        "Spent: less than $0.01"
    );
    assert_eq!(
        english(
            "near_ai_balance_remaining_line",
            json!({"present": 0, "nanos": 0, "scale": 9})
        ),
        "No spending limit set."
    );
    assert_eq!(
        english(
            "near_ai_balance_spent_line",
            json!({"present": 1, "nanos": 18_185_000_000_i64, "remaining_present": 1, "scale": 9})
        ),
        "Spent: $18.19"
    );
    for micros in [
        None,
        Some(0),
        Some(1),
        Some(4999),
        Some(5000),
        Some(1_994_999),
        Some(1_995_000),
        Some(1_999_999),
        Some(i64::MAX as u64),
    ] {
        assert_eq!(
            english("harness_spend_line", json!({"micros": micros})),
            copy::harness_spend_line(micros)
        );
    }
    assert_eq!(
        english("harness_spend_line", json!({"micros": 1_999_999})),
        "Today on this computer: $2.00"
    );
}

#[test]
fn credential_gates_empty_copy_and_certificate_audiences_are_preserved() {
    for (input, value) in [
        (json!(null), None),
        (json!(-1), None),
        (json!(0), Some(false)),
        (json!(false), Some(false)),
        (json!(1), Some(true)),
        (json!(true), Some(true)),
    ] {
        assert_eq!(
            english("harness_credential_notice", json!({"credentialed": input})),
            copy::harness_credential_notice(value)
        );
    }
    for state in [
        "not_connected",
        "connected_no_calls",
        "answering",
        "activity_shared",
        "unknown",
        "",
        "future_state",
    ] {
        assert_eq!(
            english("harness_state_line", json!({"state": state})),
            copy::harness_state_line(state)
        );
    }
    for outcome in [
        "changes",
        "noop",
        "unparseable",
        "not_installed",
        "entry_unusable",
        "no_config_path",
        "unsupported_version",
        "models_unavailable",
        "",
        "future_outcome",
    ] {
        assert_eq!(
            english("harness_outcome_line", json!({"outcome": outcome})),
            copy::harness_outcome_line(outcome)
        );
    }
    for state in [
        copy::LABEL_CREDENTIAL_ABSENT,
        copy::LABEL_CREDENTIAL_OBTAINING,
        copy::LABEL_CREDENTIAL_PRESENT,
        copy::LABEL_CREDENTIAL_FAILED,
        copy::LABEL_CREDENTIAL_CANCELLED,
        copy::LABEL_CREDENTIAL_CLEANUP_REQUIRED,
        copy::LABEL_CREDENTIAL_MIGRATION_AVAILABLE,
        copy::LABEL_CREDENTIAL_STORAGE_UNAVAILABLE,
        copy::LABEL_CREDENTIAL_STORAGE_UNENTITLED,
        "",
        "future_state",
    ] {
        assert_eq!(
            english("near_ai_credential_state_line", json!({"state": state})),
            copy::credential_state_line(state)
        );
    }
    for state in [
        copy::LABEL_BALANCE_KNOWN,
        copy::LABEL_BALANCE_NO_SESSION,
        copy::LABEL_BALANCE_SESSION_EXPIRED,
        copy::LABEL_BALANCE_NO_ORGANIZATION,
        copy::LABEL_BALANCE_UNAVAILABLE,
        "",
        "future_state",
    ] {
        assert_eq!(
            english("near_ai_balance_state_line", json!({"state": state})),
            copy::balance_state_line(state)
        );
    }
    for label in [
        copy::NEAR_AI_ENROLL_ALREADY_ENROLLED,
        copy::NEAR_AI_ENROLL_NO_SESSION,
        copy::NEAR_AI_ENROLL_ENDPOINT_REFUSED,
        copy::NEAR_AI_ENROLL_TOKEN_UNAVAILABLE,
        copy::NEAR_AI_ENROLL_START_FAILED,
        copy::NEAR_AI_ENROLL_COMMONS_UNREACHABLE,
        copy::NEAR_AI_ENROLL_COMMONS_UNSUPPORTED,
        copy::NEAR_AI_ENROLL_INVALID,
        copy::NEAR_AI_ENROLL_VERIFICATION_FAILED,
        copy::NEAR_AI_ENROLL_UNAVAILABLE,
        "future_label",
    ] {
        assert_eq!(
            english("near_ai_enroll_line", json!({"label": label})),
            copy::near_ai_enroll_line(label)
        );
    }
    for evidence in [true, false] {
        assert_eq!(
            english(
                "certificate_row_line",
                json!({"evidence_admitted": evidence})
            ),
            copy::certificate_row_line(evidence)
        );
        assert_eq!(
            english(
                "certificate_list_title",
                json!({"evidence_admitted": evidence})
            ),
            copy::certificate_list_title(evidence)
        );
    }
    assert_eq!(
        english("certificate_list_empty", json!({})),
        copy::certificate_list_empty()
    );
}

#[test]
fn account_json_prose_preserves_raw_names_and_signed_change_rules() {
    let name = "A {count} {organization} %@ <team>";
    let view = presentation("spend_org_view", json!({"current_name": name}));
    let legacy = copy::spend_org_view(Some(name));
    for (field, message) in view.as_object().unwrap() {
        assert_eq!(
            localized_copy::english(message),
            legacy[field].as_str().unwrap()
        );
    }
    assert_eq!(view["prompt_line"]["parameters"]["organization"], name);
    assert_eq!(
        english(
            "funding_message",
            json!({"state": "ready", "organization_name": name})
        ),
        format!("Organization: {name}")
    );
    for nanos in [i64::MIN, -3_209_000_000, -1, 0, 1, 3_209_000_000, i64::MAX] {
        for scale in [0, 9, 38, 200] {
            assert_eq!(
                english(
                    "near_ai_balance_change_line",
                    json!({"nanos": nanos, "scale": scale})
                ),
                copy::balance_change_line(nanos, scale)
            );
            assert_eq!(
                english(
                    "near_ai_credit_received_24h_line",
                    json!({"nanos": nanos, "scale": scale})
                ),
                copy::credit_received_24h_line(nanos, scale)
            );
        }
    }
    assert_eq!(
        english(
            "near_ai_balance_change_line",
            json!({"nanos": -1, "scale": 9})
        ),
        "$0.00 in the last 24 hours"
    );
    assert_eq!(
        english(
            "near_ai_balance_change_line",
            json!({"nanos": -3_209_000_000_i64, "scale": 9})
        ),
        "-$3.20 in the last 24 hours"
    );
    assert_eq!(
        english(
            "near_ai_balance_change_line",
            json!({"nanos": null, "scale": 9})
        ),
        copy::BALANCE_CHANGE_24H_UNAVAILABLE
    );
    assert_eq!(
        english(
            "near_ai_credit_received_24h_line",
            json!({"nanos": null, "scale": 9})
        ),
        copy::CREDIT_RECEIVED_24H_UNAVAILABLE
    );
    assert!(
        presentation(
            "near_ai_balance_change_line",
            json!({"nanos": "not a numeric fact", "scale": 9})
        )
        .is_null()
    );
}

#[test]
fn exported_templates_cover_nested_messages_without_key_collisions() {
    let templates = localized_copy::templates();
    fn collect(value: &Value, catalog: &mut std::collections::BTreeMap<String, Value>) {
        if let Some(key) = value.get("key") {
            let key = key.as_str().expect("message key must be a string");
            assert!(value["parameters"].is_object(), "parameters for {key}");
            let english = &value["english"];
            assert!(
                english.is_string() || (english["one"].is_string() && english["other"].is_string()),
                "English template for {key}"
            );
            if let Some(previous) = catalog.insert(key.to_owned(), english.clone()) {
                assert_eq!(previous, *english, "conflicting template for {key}");
            }
        }
        match value {
            Value::Object(object) => {
                for child in object.values() {
                    collect(child, catalog);
                }
            }
            Value::Array(array) => {
                for child in array {
                    collect(child, catalog);
                }
            }
            _ => {}
        }
    }
    let mut catalog = std::collections::BTreeMap::new();
    // Exercise both object and array traversal even if today's roots are maps.
    let structured_templates = json!({"structured": [templates]});
    collect(&structured_templates, &mut catalog);
    assert!(
        !catalog.is_empty(),
        "the inventory must contain message leaves"
    );
    fn check(value: &Value, catalog: &std::collections::BTreeMap<String, Value>) {
        if let Some(key) = value.get("key") {
            let key = key.as_str().expect("message key must be a string");
            assert!(value["parameters"].is_object(), "parameters for {key}");
            assert_eq!(
                catalog.get(key),
                Some(&value["english"]),
                "uncatalogued {key}"
            );
        }
        match value {
            Value::Object(object) => {
                for child in object.values() {
                    check(child, catalog);
                }
            }
            Value::Array(array) => {
                for child in array {
                    check(child, catalog);
                }
            }
            _ => {}
        }
    }
    check(&structured_templates, &catalog);
    for (operation, input) in [
        (
            "near_ai_balance_spent_line",
            json!({"nanos": 1, "remaining": 0, "scale": 9}),
        ),
        (
            "near_ai_balance_remaining_line",
            json!({"nanos": -1, "scale": 9}),
        ),
        (
            "near_ai_balance_remaining_line",
            json!({"nanos": 1, "scale": 200}),
        ),
        ("harness_spend_line", json!({"micros": 1})),
        ("contribution_withheld_line", json!({"withheld": 1})),
        ("harness_last_call_line", json!({"seconds_ago": 86400})),
        (
            "near_ai_balance_change_line",
            json!({"nanos": null, "scale": 9}),
        ),
        (
            "near_ai_credit_received_24h_line",
            json!({"nanos": null, "scale": 9}),
        ),
        ("spend_org_view", json!({"current_name": "{count}"})),
        ("spend_org_view", json!({"current_name": null})),
        (
            "funding_message",
            json!({"state": "ready", "organization_name": "{count}"}),
        ),
    ] {
        let message = presentation(operation, input);
        assert!(message.is_object(), "supported operation {operation}");
        check(&message, &catalog);
    }
    assert!(presentation("not_a_presentation_operation", json!({})).is_null());
}
