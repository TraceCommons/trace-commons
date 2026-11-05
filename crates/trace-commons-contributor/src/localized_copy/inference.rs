//! Private AI presentation. State keys follow facts, never rendered English.

use serde_json::{Value, json};

use super::text;
use crate::private_inference_copy as copy;

const HARNESS_STATES: &[&str] = &[
    "not_connected",
    "connected_no_calls",
    "answering",
    "activity_shared",
    "unknown",
];
const HARNESS_OUTCOMES: &[&str] = &[
    "changes",
    "noop",
    "unparseable",
    "not_installed",
    "entry_unusable",
    "no_config_path",
    "unsupported_version",
    "models_unavailable",
];
const CREDENTIAL_STATES: &[&str] = &[
    copy::LABEL_CREDENTIAL_ABSENT,
    copy::LABEL_CREDENTIAL_OBTAINING,
    copy::LABEL_CREDENTIAL_FAILED,
    copy::LABEL_CREDENTIAL_CANCELLED,
    copy::LABEL_CREDENTIAL_PRESENT,
    copy::LABEL_CREDENTIAL_CLEANUP_REQUIRED,
    copy::LABEL_CREDENTIAL_MIGRATION_AVAILABLE,
    copy::LABEL_CREDENTIAL_STORAGE_UNAVAILABLE,
    copy::LABEL_CREDENTIAL_STORAGE_UNENTITLED,
];
const BALANCE_STATES: &[&str] = &[
    copy::LABEL_BALANCE_KNOWN,
    copy::LABEL_BALANCE_NO_SESSION,
    copy::LABEL_BALANCE_SESSION_EXPIRED,
    copy::LABEL_BALANCE_NO_ORGANIZATION,
    copy::LABEL_BALANCE_UNAVAILABLE,
];
const ENROLL_LABELS: &[&str] = &[
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
];

fn queue_labels() -> Vec<&'static str> {
    use crate::daemon::{health, preview, queue, second_look};
    let mut labels = refusal_labels();
    labels.extend([
        second_look::REASON_SECOND_LOOK_REVIEW_REQUIRED,
        second_look::REASON_SCRUB_CHECK_MANUAL,
        queue::REASON_DISMISSED,
        queue::REASON_EXPIRED,
        queue::REASON_CHANGED,
        queue::REASON_SCOPES_CHANGED,
        preview::REASON_INPUTS_CHANGED,
        "envelope-changed-after-approval",
        health::LABEL_NOT_LOGGED_IN,
        health::LABEL_QUEUE_FULL,
        health::LABEL_INGEST_UNREACHABLE,
        health::LABEL_CLAIM_MINT_FAILED,
        health::LABEL_PII_FILTER_UNAVAILABLE,
        health::LABEL_CANARY_FAILED,
        health::LABEL_WITNESS_SATURATED,
        crate::submit::REASON_TRANSIENT_REDACTION,
        crate::submit::REASON_RATE_LIMITED,
        crate::submit::REASON_TRANSIENT_REDACTION_EXHAUSTED,
        crate::submit::REASON_WITNESS_NOT_TRUSTED,
        crate::submit::REASON_WITNESS_MEASUREMENT_NOT_ACCEPTED,
    ]);
    labels
}

fn refusal_labels() -> Vec<&'static str> {
    trace_commons_protocol::admission::AdmissionRefusal::ALL
        .iter()
        .map(|refusal| refusal.label())
        .collect()
}

fn menu_word(state: &str) -> &'static str {
    copy::menu_state(state).0
}

type StateLine = fn(&str) -> &'static str;

/// Explicit canonical whitelists. Unknown wire values are never interpolated
/// into keys, so future states cannot expand the catalog or borrow a claim.
fn state_operation(operation: &str) -> Option<(&'static str, Vec<&'static str>, StateLine)> {
    use crate::daemon::{attestation_mark, contribution_eligibility};
    Some(match operation {
        "private_inference_state_line" => ("state", copy::STATE_LABELS.to_vec(), copy::state_line),
        "private_inference_runtime_word" => {
            ("state", copy::STATE_LABELS.to_vec(), copy::runtime_word)
        }
        "private_inference_menu_word" => ("state", copy::STATE_LABELS.to_vec(), menu_word),
        "harness_state_line" => ("state", HARNESS_STATES.to_vec(), copy::harness_state_line),
        "harness_outcome_line" => (
            "outcome",
            HARNESS_OUTCOMES.to_vec(),
            copy::harness_outcome_line,
        ),
        "near_ai_credential_state_line" => (
            "state",
            CREDENTIAL_STATES.to_vec(),
            copy::credential_state_line,
        ),
        "contribution_eligibility_line" => (
            "state",
            contribution_eligibility::ALL_STATES.to_vec(),
            copy::eligibility_state_line,
        ),
        "contribution_eligibility_reason_line" => (
            "reason",
            attestation_mark::ALL_REASONS.to_vec(),
            copy::eligibility_reason_line,
        ),
        "contribution_attestation_line" => (
            "mark",
            attestation_mark::ALL_MARKS.to_vec(),
            copy::attestation_state_line,
        ),
        "contribution_attestation_reason_line" => (
            "reason",
            attestation_mark::ALL_REASONS.to_vec(),
            copy::attestation_reason_line,
        ),
        "near_ai_balance_state_line" => {
            ("state", BALANCE_STATES.to_vec(), copy::balance_state_line)
        }
        "near_ai_enroll_line" => ("label", ENROLL_LABELS.to_vec(), copy::near_ai_enroll_line),
        "queue_outcome_line" => ("label", queue_labels(), copy::queue_outcome_line),
        _ => return None,
    })
}

const STATE_OPERATIONS: &[&str] = &[
    "private_inference_state_line",
    "private_inference_runtime_word",
    "private_inference_menu_word",
    "harness_state_line",
    "harness_outcome_line",
    "near_ai_credential_state_line",
    "contribution_eligibility_line",
    "contribution_eligibility_reason_line",
    "contribution_attestation_line",
    "contribution_attestation_reason_line",
    "near_ai_balance_state_line",
    "near_ai_enroll_line",
    "queue_outcome_line",
];

fn boolean(value: &Value) -> Option<bool> {
    value.as_bool().or_else(|| value.as_i64().map(|n| n != 0))
}

fn optional_nanos(input: &Value) -> Option<i64> {
    if boolean(&input["present"]) == Some(false) {
        None
    } else {
        input["nanos"].as_i64()
    }
}

/// Operation names are the existing C symbols without `tc_`. Nullable facts
/// can be JSON null; legacy integer presence flags are also accepted.
pub fn message(operation: &str, input: &Value) -> Option<Value> {
    if let Some((field, labels, line)) = state_operation(operation) {
        // `label` also allows uniform state callers; legacy argument names
        // remain authoritative when present.
        let state = input
            .get(field)
            .or_else(|| input.get("label"))
            .or_else(|| input.get("state"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        let (variant, canonical) = if state.is_empty() {
            ("unreported", "")
        } else if let Some(&known) = labels.iter().find(|&&known| known == state) {
            (known, known)
        } else {
            ("fallback", "__unrecognized_state__")
        };
        return Some(text(
            &format!("{operation}.{variant}"),
            line(canonical),
            json!({}),
        ));
    }
    let scale = input["scale"]
        .as_u64()
        .and_then(|n| u8::try_from(n).ok())
        .unwrap_or(u8::MAX);
    Some(match operation {
        "private_inference_serving_line" => {
            copy::serving_message(input["port"].as_u64().and_then(|n| u16::try_from(n).ok()))
        }
        "harness_last_call_line" => copy::age_message(input["seconds_ago"].as_u64(), false),
        "near_ai_balance_observed_line" => copy::age_message(input["seconds_ago"].as_u64(), true),
        "harness_spend_line" => copy::harness_spend_message(
            input["micros"].as_i64().and_then(|n| u64::try_from(n).ok()),
        ),
        "contribution_withheld_line" => {
            copy::group_withheld_message(input["withheld"].as_u64().unwrap_or_default())
        }
        "harness_credential_notice" => {
            // C uses -1 for absent, 0 for false and 1 for true.
            let credentialed = match input["credentialed"].as_i64() {
                Some(n) if n < 0 => None,
                _ => boolean(&input["credentialed"]),
            };
            let variant = match credentialed {
                Some(false) => "absent",
                Some(true) => "present",
                None => "unreported",
            };
            text(
                &format!("{operation}.{variant}"),
                copy::harness_credential_notice(credentialed),
                json!({}),
            )
        }
        "near_ai_balance_amount" => match optional_nanos(input) {
            Some(nanos) => copy::amount_message(nanos, scale, false),
            None => text("near_ai_balance_amount.absent", "", json!({})),
        },
        "near_ai_balance_remaining_line"
        | "near_ai_balance_limit_line"
        | "near_ai_balance_spent_line" => {
            let remaining = match boolean(&input["remaining_present"]) {
                Some(true) => Some(0),
                Some(false) => None,
                None => input["remaining"].as_i64(),
            };
            copy::balance_figure_message(operation, optional_nanos(input), remaining, scale)
        }
        "near_ai_credit_received_24h_line" => match input["nanos"].as_i64() {
            Some(nanos) => copy::credit_received_message(nanos, scale),
            None if input["nanos"].is_null() => text(
                "near_ai_received_24h_line.unavailable",
                copy::CREDIT_RECEIVED_24H_UNAVAILABLE,
                json!({}),
            ),
            None => return None,
        },
        "near_ai_balance_change_line" => match input["nanos"].as_i64() {
            Some(nanos) => copy::balance_change_message(nanos, scale),
            None if input["nanos"].is_null() => text(
                "near_ai_balance_change_line.unavailable",
                copy::BALANCE_CHANGE_24H_UNAVAILABLE,
                json!({}),
            ),
            None => return None,
        },
        "certificate_row_line" | "certificate_list_title" => {
            let evidence = boolean(&input["evidence_admitted"]).unwrap_or(false);
            let variant = if evidence { "candidate" } else { "attested" };
            let english = if operation == "certificate_row_line" {
                copy::certificate_row_line(evidence)
            } else {
                copy::certificate_list_title(evidence)
            };
            text(&format!("{operation}.{variant}"), english, json!({}))
        }
        "certificate_list_empty" => text(
            "certificate_list_empty.value",
            copy::certificate_list_empty(),
            json!({}),
        ),
        "outcome_refusal_line" => {
            let label = input["label"].as_str().unwrap_or_default();
            let labels = refusal_labels();
            let &canonical = labels.iter().find(|&&known| known == label)?;
            text(
                &format!("{operation}.{canonical}"),
                copy::outcome_refusal_line(canonical)?,
                json!({}),
            )
        }
        // These prose APIs are carried in daemon JSON rather than standalone
        // C symbols, but are rendered by macOS too.
        "spend_org_view" => copy::spend_org_messages(input["current_name"].as_str()),
        "funding_message" => copy::funding_presentation(
            input["state"].as_str().unwrap_or_default(),
            input["organization_name"].as_str().unwrap_or_default(),
        ),
        _ => return None,
    })
}

/// Enumerate branches with facts, including intentional suppression. Nested
/// messages are included independently for native lookup and catalog export.
pub fn templates() -> Vec<Value> {
    let mut probes = Vec::new();
    for operation in STATE_OPERATIONS {
        let (field, mut labels, _) = state_operation(operation).expect("listed operation");
        labels.extend(["", "__unrecognized_state__"]);
        for state in labels {
            let mut input = json!({});
            input[field] = json!(state);
            probes.push(message(operation, &input).expect("state operation"));
        }
    }
    for label in refusal_labels() {
        probes.push(message("outcome_refusal_line", &json!({"label": label})).expect("refusal"));
    }
    for operation in ["harness_last_call_line", "near_ai_balance_observed_line"] {
        for seconds in [
            None,
            Some(0),
            Some(60),
            Some(120),
            Some(3600),
            Some(7200),
            Some(86400),
            Some(172800),
        ] {
            probes.push(message(operation, &json!({"seconds_ago": seconds})).expect("age"));
        }
    }
    for credentialed in [json!(null), json!(false), json!(true)] {
        probes.push(
            message(
                "harness_credential_notice",
                &json!({"credentialed": credentialed}),
            )
            .expect("credential notice"),
        );
    }
    for port in [None, Some(0), Some(8463)] {
        probes.push(copy::serving_message(port));
    }
    for micros in [None, Some(0), Some(1), Some(1_999_999)] {
        probes.push(copy::harness_spend_message(micros));
    }
    for withheld in [0, 1, 2] {
        probes.push(copy::group_withheld_message(withheld));
    }
    for operation in [
        "near_ai_balance_amount",
        "near_ai_balance_remaining_line",
        "near_ai_balance_limit_line",
        "near_ai_balance_spent_line",
    ] {
        for nanos in [None, Some(0), Some(1), Some(-1), Some(18_185_000_000_i64)] {
            for scale in [9, 200] {
                for remaining in [None, Some(0)] {
                    probes.push(
                        message(
                            operation,
                            &json!({"nanos": nanos, "scale": scale, "remaining": remaining}),
                        )
                        .expect("balance figure"),
                    );
                }
            }
        }
    }
    for nanos in [-1, 0, 1, 3_209_000_000] {
        for scale in [9, 200] {
            probes.push(copy::credit_received_message(nanos, scale));
            probes.push(copy::balance_change_message(nanos, scale));
        }
    }
    for operation in [
        "near_ai_credit_received_24h_line",
        "near_ai_balance_change_line",
    ] {
        probes.push(message(operation, &json!({"nanos": null})).expect("unavailable amount"));
    }
    for evidence in [false, true] {
        for operation in ["certificate_row_line", "certificate_list_title"] {
            probes.push(
                message(operation, &json!({"evidence_admitted": evidence})).expect("certificate"),
            );
        }
    }
    probes.push(message("certificate_list_empty", &json!({})).expect("empty certificate list"));
    for name in [None, Some("Example organization")] {
        probes.push(copy::spend_org_messages(name));
    }
    for state in [
        "ready",
        "no_session",
        "session_expired",
        "no_inference_key",
        "no_organization",
        "changed",
        "invalid_request",
        "unavailable",
        "__unrecognized_state__",
    ] {
        probes.push(copy::funding_presentation(state, "Example organization"));
    }
    fn collect(value: &Value, messages: &mut std::collections::BTreeMap<String, Value>) {
        if let Some(key) = value["key"].as_str() {
            messages.entry(key.into()).or_insert_with(|| value.clone());
        }
        if let Some(object) = value.as_object() {
            for child in object.values() {
                collect(child, messages);
            }
        }
    }
    let mut messages = std::collections::BTreeMap::new();
    for probe in probes {
        collect(&probe, &mut messages);
    }
    messages.into_values().collect()
}
