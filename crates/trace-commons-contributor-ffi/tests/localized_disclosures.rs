//! The localized ABI renders exactly the legacy disclosure tree in English,
//! while keeping facts raw and counts numeric. No service or environment writes.

use std::ffi::{CStr, CString, c_char};

use serde_json::{Value, json};
use trace_commons_contributor::localized_copy;
use trace_commons_contributor_ffi::*;

fn owned(pointer: *mut c_char) -> Option<String> {
    if pointer.is_null() {
        return None;
    }
    let text = unsafe { CStr::from_ptr(pointer) }
        .to_str()
        .expect("UTF-8")
        .to_owned();
    unsafe { tc_string_free(pointer) };
    Some(text)
}

fn localized(operation: &str, input: &Value) -> Option<Value> {
    let operation = CString::new(operation).unwrap();
    let input = CString::new(input.to_string()).unwrap();
    owned(unsafe { tc_localized_presentation_json(operation.as_ptr(), input.as_ptr()) })
        .map(|text| serde_json::from_str(&text).expect("owned presentation JSON"))
}

fn render(tree: &Value) -> Value {
    if tree.get("key").is_some() {
        return json!(localized_copy::english(tree));
    }
    match tree {
        Value::Object(object) => {
            Value::Object(object.iter().map(|(k, v)| (k.clone(), render(v))).collect())
        }
        Value::Array(array) => Value::Array(array.iter().map(render).collect()),
        _ => tree.clone(),
    }
}

fn legacy_json(pointer: *mut c_char) -> Option<Value> {
    owned(pointer).map(|text| serde_json::from_str(&text).expect("legacy JSON"))
}

fn assert_parity(op: &str, input: &Value, expected: Option<Value>) -> Option<Value> {
    let localized = localized(op, input);
    assert_eq!(localized.as_ref().map(render), expected, "{op}: {input}");
    localized
}

#[test]
fn project_counts_and_literal_braces_match_the_legacy_abi() {
    for label in ["Synthetic folder", "{count} / {project} / 日本語", ""] {
        let c_label = CString::new(label).unwrap();
        for n in [-1, 0, 1, 2, 1000] {
            let copy = assert_parity(
                "project_ignore_copy_json",
                &json!({"project": label, "pending": n}),
                legacy_json(unsafe { tc_project_ignore_copy_json(c_label.as_ptr(), n) }),
            )
            .unwrap();
            assert_eq!(copy["title"]["parameters"]["project"], label);
            if n > 0 {
                assert_eq!(copy["body"]["parameters"]["count"], n);
            }
            let expected =
                owned(unsafe { tc_project_ignore_reconciled_text(c_label.as_ptr(), 3, n) })
                    .map(Value::String);
            assert_parity(
                "project_ignore_reconciled_text",
                &json!({"project": label, "promised": 3, "purged": n}),
                expected,
            );
        }
        assert_parity(
            "project_ignore_reconciled_text",
            &json!({"project": label, "promised": 2, "purged": 2}),
            Some(json!("")),
        );
        for n in [0, 1, 2, u32::MAX] {
            let copy = assert_parity(
                "arming_offer_copy_json",
                &json!({"project": label, "count": n}),
                legacy_json(unsafe { tc_arming_offer_copy_json(c_label.as_ptr(), n) }),
            )
            .unwrap();
            assert_eq!(copy["evidence"]["parameters"]["count"], n);
            assert_eq!(
                copy["evidence"]["english"]["one"],
                "You've contributed from {project} once."
            );
        }
    }
    assert!(localized("project_ignore_copy_json", &json!({"pending": 1})).is_none());
}

#[test]
fn configuration_reads_are_fail_closed_and_only_automatic_reads_config() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().to_str().unwrap();
    let c_dir = CString::new(dir).unwrap();
    assert_parity(
        "automatic_contribution_copy_json",
        &json!({"config_dir": dir}),
        legacy_json(unsafe { tc_automatic_contribution_copy_json(c_dir.as_ptr()) }),
    );
    for mode in ["notify_only", "auto_upload", "ignore", "clear", "future"] {
        let c_mode = CString::new(mode).unwrap();
        let copy = assert_parity(
            "contribution_override_confirm_json",
            &json!({"mode": mode, "config_dir": dir}),
            legacy_json(unsafe {
                tc_contribution_override_confirm_json(c_mode.as_ptr(), c_dir.as_ptr())
            }),
        );
        if let Some(copy) = copy {
            assert_eq!(copy["mode"], mode);
        }
    }
    // Corrupt persisted settings: neither entry point may guess a disclosure.
    std::fs::write(temp.path().join("contributor.json"), b"not JSON").unwrap();
    assert_parity(
        "automatic_contribution_copy_json",
        &json!({"config_dir": dir}),
        legacy_json(unsafe { tc_automatic_contribution_copy_json(c_dir.as_ptr()) }),
    );
    let mode = CString::new("auto_upload").unwrap();
    assert_parity(
        "contribution_override_confirm_json",
        &json!({"mode": "auto_upload", "config_dir": dir}),
        legacy_json(unsafe {
            tc_contribution_override_confirm_json(mode.as_ptr(), c_dir.as_ptr())
        }),
    );
    assert!(localized("automatic_contribution_copy_json", &json!({})).is_none());
    assert!(
        localized(
            "contribution_override_confirm_json",
            &json!({"mode": "auto_upload"})
        )
        .is_none()
    );
    for mode in ["notify_only", "ignore", "clear"] {
        let mode_c = CString::new(mode).unwrap();
        assert_parity(
            "contribution_override_confirm_json",
            &json!({"mode": mode}),
            legacy_json(unsafe {
                tc_contribution_override_confirm_json(mode_c.as_ptr(), std::ptr::null())
            }),
        );
    }
}

#[test]
fn fixed_grants_help_and_refusals_keep_the_core_choices() {
    for disclosure in ["patterns_only", "model_scrubbed", "future"] {
        let c = CString::new(disclosure).unwrap();
        if let Some(copy) = assert_parity(
            "automatic_grant_copy_json",
            &json!({"disclosure": disclosure}),
            legacy_json(unsafe { tc_automatic_grant_copy_json(c.as_ptr()) }),
        ) {
            assert_eq!(copy["disclosure"], disclosure);
        }
    }
    for pinned in [false, true] {
        assert_parity(
            "consent_gate_help",
            &json!({"pinned": pinned}),
            owned(tc_consent_gate_help(i32::from(pinned))).map(Value::String),
        );
    }
    assert_parity(
        "withdrawal_confirmation_prompt_text",
        &json!({}),
        owned(tc_withdrawal_confirmation_prompt_text()).map(Value::String),
    );
    for label in ["arming-terms-unavailable", "future", ""] {
        let c = CString::new(label).unwrap();
        assert_parity(
            "contribution_override_refusal_text",
            &json!({"label": label}),
            owned(unsafe { tc_contribution_override_refusal_text(c.as_ptr()) }).map(Value::String),
        );
    }
    for label in [
        "legacy_migration_tenant_pooled",
        "legacy_migration_invite_not_linked",
        "legacy_migration_already_migrated",
        "future",
        "",
    ] {
        let c = CString::new(label).unwrap();
        assert_parity(
            "legacy_migration_refusal_text",
            &json!({"label": label}),
            owned(unsafe { tc_legacy_migration_refusal_text(c.as_ptr()) }).map(Value::String),
        );
    }
}

#[test]
fn health_suppression_fallback_and_grouped_counts_are_unchanged() {
    for reachable in [false, true] {
        for label in trace_commons_contributor::daemon::health::ALL_LABELS
            .iter()
            .copied()
            .chain(["future", ""])
        {
            let c = CString::new(label).unwrap();
            for max in [-1, 0, 1, 2, 1000, i64::MAX] {
                let copy = assert_parity(
                    "health_copy_json",
                    &json!({"reachable": reachable, "label": label, "max_queue_entries": max}),
                    legacy_json(unsafe {
                        tc_health_copy_json(i32::from(reachable), c.as_ptr(), max)
                    }),
                );
                if let Some(copy) = copy {
                    assert!(copy["severity"].is_string());
                    assert!(copy["action_kind"].is_null() || copy["action_kind"].is_string());
                }
            }
        }
    }
    assert!(localized("health_copy_json", &json!({"reachable": true})).is_none());
}

type WireCopy = unsafe extern "C" fn(*const c_char) -> *mut c_char;

fn wire(op: &str, input: &Value, legacy: WireCopy) -> Option<Value> {
    let c = CString::new(input.to_string()).unwrap();
    assert_parity(op, input, legacy_json(unsafe { legacy(c.as_ptr()) }))
}

#[test]
fn wire_notices_preserve_suppression_unknowns_and_action_identifiers() {
    for input in [Value::Null, json!([]), json!(1)] {
        for (op, legacy) in [
            ("grant_void_notice", tc_grant_void_notice as WireCopy),
            (
                "arming_reworded_notice",
                tc_arming_reworded_notice as WireCopy,
            ),
            ("gate_held_notice", tc_gate_held_notice as WireCopy),
            (
                "legacy_migration_notice",
                tc_legacy_migration_notice as WireCopy,
            ),
            (
                "account_attention_copy_json",
                tc_account_attention_copy_json as WireCopy,
            ),
            (
                "witness_capacity_notice",
                tc_witness_capacity_notice as WireCopy,
            ),
        ] {
            wire(op, &input, legacy);
        }
    }
    for kind in [
        "project",
        "automatic_grant",
        "contribution_override",
        "future",
    ] {
        for label in ["", "{project} and {count}"] {
            for id in ["", "technical-id"] {
                let input = json!({"kind": kind, "project_label": label, "project_id": id,
                    "reasons": ["witness-changed", "future", "future-2", "witness-changed", 1]});
                let copy = wire("grant_void_notice", &input, tc_grant_void_notice).unwrap();
                assert_eq!(copy["reasons"].as_array().unwrap().len(), 2);
            }
        }
    }
    for label in ["", "{project} and {count}"] {
        for scrub in [false, true] {
            for id in ["", "technical-id"] {
                wire(
                    "arming_reworded_notice",
                    &json!({"project_label": label, "project_id": id, "scrub_check_defaulted": scrub}),
                    tc_arming_reworded_notice,
                );
            }
        }
    }
    for held in [0, 1, 2, u64::MAX] {
        let copy = wire(
            "gate_held_notice",
            &json!({"held_sessions": held,
            "reasons": ["no-data-use-scope-chosen", "future", "future-2"],
            "projects": [{"project_id": "raw-id", "project_label": "{count}", "held_sessions": held},
                {"project_label": "no-id"}, {"project_label": ""}, null]}),
            tc_gate_held_notice,
        );
        if let Some(copy) = copy {
            assert_eq!(copy["projects"][0]["project_id"], "raw-id");
            assert_eq!(copy["body"]["parameters"]["count"], held);
            assert_eq!(copy["projects"][1]["ask_first_action"], Value::Null);
        }
        wire(
            "witness_capacity_notice",
            &json!({"waiting_sessions": held}),
            tc_witness_capacity_notice,
        );
    }
    for folders in [0, 1, 5] {
        for grant in [false, true] {
            wire(
                "legacy_migration_notice",
                &json!({"folders_kept": folders, "automatic_grant_kept": grant}),
                tc_legacy_migration_notice,
            );
        }
    }
    for state in ["ok", "needs_attention", "future"] {
        for reason in [
            "account_record_unreadable",
            "account_address_refused",
            "future",
        ] {
            for login in [false, true] {
                wire(
                    "account_attention_copy_json",
                    &json!({"account_state": state, "account_state_reason": reason, "near_ai_login_unconfirmed": login}),
                    tc_account_attention_copy_json,
                );
            }
        }
    }
}

#[test]
fn route_facts_remain_raw_and_all_privacy_variants_match() {
    for route in [
        "witness",
        "witness_refusing",
        "local",
        "not_enrolled",
        "settings_unreadable",
        "future",
    ] {
        for origin in [
            "published_at_join",
            "connected_inference",
            "settings",
            "environment",
            "not_recorded",
            "future",
        ] {
            for filter in ["none", "near_ai", "self_hosted", "sidecar", "invalid"] {
                for (endpoint, checked) in [(false, false), (true, false), (true, true)] {
                    for bodies in [false, true] {
                        let input = json!({"route": route, "local_filter": filter,
                            "receipts": {"endpoint_configured": endpoint, "check_attestation": checked}, "attested_bodies": bodies,
                            "witness": {"state": "pinned", "url": "https://synthetic.invalid/{count}", "signing_address": "raw-address",
                                "pinned_measurements": ["raw-measurement"], "origin": origin}});
                        if let Some(copy) =
                            wire("route_disclosure_copy", &input, tc_route_disclosure_copy)
                        {
                            assert_eq!(copy["facts"], input);
                            assert!(copy["copy"]["route"]["key"].is_string());
                        }
                    }
                }
            }
        }
    }
    wire(
        "route_disclosure_copy",
        &json!({}),
        tc_route_disclosure_copy,
    );
}

#[test]
fn template_inventory_covers_live_keys_with_the_same_english_and_numeric_plurals() {
    fn collect(tree: &Value, inventory: &mut std::collections::BTreeMap<String, Value>) {
        if let Some(key) = tree.get("key").and_then(Value::as_str) {
            if let Some(previous) = inventory.insert(key.to_owned(), tree.clone()) {
                assert_eq!(previous["english"], tree["english"], "conflicting {key}");
            }
        }
        match tree {
            Value::Object(fields) => {
                for child in fields.values() {
                    collect(child, inventory);
                }
            }
            Value::Array(values) => {
                for child in values {
                    collect(child, inventory);
                }
            }
            _ => {}
        }
    }
    let mut inventory = std::collections::BTreeMap::new();
    for template in localized_copy::templates() {
        collect(&template, &mut inventory);
    }
    fn check(tree: &Value, inventory: &std::collections::BTreeMap<String, Value>) {
        if let Some(key) = tree.get("key").and_then(Value::as_str) {
            assert_eq!(
                inventory
                    .get(key)
                    .unwrap_or_else(|| panic!("missing {key}"))["english"],
                tree["english"]
            );
            if tree["english"].is_object() {
                assert!(tree["parameters"]["count"].is_u64(), "{key}");
            }
        } else {
            match tree {
                Value::Object(object) => {
                    for child in object.values() {
                        check(child, inventory);
                    }
                }
                Value::Array(array) => {
                    for child in array {
                        check(child, inventory);
                    }
                }
                _ => {}
            }
        }
    }
    for (op, input) in [
        (
            "project_ignore_copy_json",
            json!({"project": "literal {count}", "pending": 2}),
        ),
        (
            "arming_offer_copy_json",
            json!({"project": "Project", "count": 1}),
        ),
        (
            "grant_void_notice",
            json!({"kind": "project", "project_label": "Project", "project_id": "id", "reasons": ["witness-changed"]}),
        ),
        (
            "arming_reworded_notice",
            json!({"project_label": "Project", "project_id": "id", "scrub_check_defaulted": true}),
        ),
        (
            "gate_held_notice",
            json!({"held_sessions": 3, "projects": [{"project_id": "id", "project_label": "Project", "held_sessions": 2}]}),
        ),
        (
            "automatic_grant_copy_json",
            json!({"disclosure": "model_scrubbed"}),
        ),
        (
            "health_copy_json",
            json!({"reachable": true, "label": "queue-full", "max_queue_entries": 1000}),
        ),
    ] {
        check(&localized(op, &input).unwrap(), &inventory);
    }
    // This internal adapter exists for the handle resolver, not for clients
    // to assert that they host a watcher when they do not.
    assert!(localized("quit_prompt", &json!({"role": "hosting"})).is_none());
    use trace_commons_contributor::quit_copy::{QuitRole, quit_prompt};
    for (role, resolved) in [
        ("hosting", QuitRole::Hosting),
        ("attached", QuitRole::Attached),
        ("unavailable", QuitRole::Unavailable),
    ] {
        let tree = localized_copy::message("quit_prompt", &json!({"role": role})).unwrap();
        check(&tree, &inventory);
        assert_eq!(
            render(&tree),
            serde_json::to_value(quit_prompt(resolved)).unwrap()
        );
    }
}
