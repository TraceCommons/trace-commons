//! Presentation adapters for the core's disclosure decisions. Only named
//! prose fields are wrapped; wire facts, identifiers and action kinds stay raw.

use serde_json::{Value, json};

use super::text;
use crate::{consent_copy as consent, health_copy as health, project_copy as project};

const VOID_REASONS: &[&str] = &[
    "destination-changed",
    "identity-changed",
    "scopes-widened",
    "privacy-filter-changed",
    "receipt-endpoint-changed",
    "witness-changed",
    "witness-measurement-admitted",
    "attested-bodies-on",
    "terms-unrecorded",
];
const HELD_REASONS: &[&str] = &[
    "admission-evidence-is-per-session",
    "account-allowance-spent",
    "no-data-use-scope-chosen",
];
const MIGRATION_REFUSALS: &[&str] = &[
    "legacy_migration_tenant_pooled",
    "legacy_migration_admission_not_ready",
    "legacy_migration_no_near_ai_session",
    "legacy_migration_invite_needed",
    "legacy_migration_invite_other_commons",
    "legacy_migration_invite_invalid",
    "legacy_migration_tenant_claimed",
    "legacy_migration_invite_not_linked",
    "legacy_migration_invite_revoked",
    "legacy_migration_device_not_eligible",
    "legacy_migration_link_not_enabled",
    "legacy_migration_verification_failed",
    "legacy_migration_identity_changed",
    "legacy_migration_commons_changed",
    "legacy_migration_already_migrated",
];

fn value(copy: impl serde::Serialize) -> Value {
    serde_json::to_value(copy).expect("copy structs serialize")
}

fn line(op: &str, variant: &str, field: &str, english: &str, parameters: Value) -> Value {
    text(
        &format!("message.{op}.{variant}.{field}"),
        english,
        parameters,
    )
}

fn fields(copy: &mut Value, op: &str, variant: &str, names: &[&str], parameters: &Value) {
    for field in names {
        if let Some(english) = copy[*field].as_str() {
            copy[*field] = line(op, variant, field, english, parameters.clone());
        }
    }
}

fn known<'a>(label: &'a str, labels: &[&str]) -> &'a str {
    if labels.contains(&label) {
        label
    } else {
        "unknown"
    }
}

fn count(input: &Value, name: &str) -> u64 {
    input[name].as_u64().unwrap_or(0)
}

fn project_label(input: &Value) -> Option<&str> {
    input["project_label"]
        .as_str()
        .filter(|label| !label.is_empty())
}

/// Replace only the known label argument, *before* asking the core to render.
/// No completed English or user-supplied value is searched or rewritten.
fn labeled_wire(input: &Value) -> Value {
    let mut wire = input.clone();
    if project_label(input).is_some() {
        wire["project_label"] = json!("{project}");
    }
    wire
}

fn reasons(input: &Value, op: &str, held: bool) -> Value {
    let mut labels = Vec::new();
    for label in input["reasons"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        let label = known(label, if held { HELD_REASONS } else { VOID_REASONS });
        if !labels.contains(&label) {
            labels.push(label);
        }
    }
    if labels.is_empty() {
        labels.push("unknown");
    }
    Value::Array(
        labels
            .into_iter()
            .map(|label| {
                line(
                    op,
                    label,
                    "reason",
                    if held {
                        consent::gate_held_reason_line(label)
                    } else {
                        consent::void_reason_line(label)
                    },
                    json!({}),
                )
            })
            .collect(),
    )
}

fn automatic(mut copy: Value, op: &str) -> Value {
    let variant = copy["disclosure"].as_str().unwrap_or("unknown").to_owned();
    fields(
        &mut copy,
        op,
        &variant,
        &[
            "summary",
            "no_review",
            "scope_required",
            "path_automatic",
            "path_ask_first",
            "path_automatic_title",
            "path_automatic_detail",
            "path_ask_first_title",
            "path_ask_first_detail",
            "raw_send",
        ],
        &json!({}),
    );
    for name in ["patterns_only", "model_scrubbed"] {
        fields(&mut copy[name], op, name, &["scope", "limit"], &json!({}));
    }
    copy
}

fn override_copy(mode: &str, config: Option<&crate::config::ContributorConfig>) -> Option<Value> {
    use crate::daemon::policy::ProjectMode;
    let copy = if mode == project::CONTRIBUTION_OVERRIDE_CLEAR_MODE {
        project::contribution_override_clear_confirm_copy()
    } else {
        let mode: ProjectMode = serde_json::from_value(json!(mode)).ok()?;
        project::contribution_override_confirm_copy(mode, config)
    };
    let mut copy = value(copy);
    fields(
        &mut copy,
        "contribution_override_confirm_json",
        mode,
        &["title", "body", "confirm", "cancel"],
        &json!({}),
    );
    if !copy["arming"].is_null() {
        copy["arming"] = automatic(copy["arming"].take(), "contribution_override_confirm_json");
    }
    Some(copy)
}

fn read_config(input: &Value) -> Option<Option<crate::config::ContributorConfig>> {
    let dir = input["config_dir"].as_str()?;
    let store = crate::config::ConfigStore::open(std::path::PathBuf::from(dir)).ok()?;
    store.load_config().ok()
}

/// `quit_prompt` is an internal operation: its role must come from the FFI's
/// resolved handle, never from an untrusted presentation request.
pub fn message(operation: &str, input: &Value) -> Option<Value> {
    let empty = json!({});
    match operation {
        "project_ignore_copy_json" => {
            let label = input["project"].as_str()?;
            let pending = usize::try_from(count(input, "pending")).unwrap_or(0);
            let mut copy = value(project::ignore_project_copy(label, pending));
            fields(
                &mut copy,
                operation,
                "default",
                &["button", "tooltip", "keep", "ignore_project_tooltip"],
                &empty,
            );
            copy["title"] = project::ignore_project_title_message(label);
            copy["body"] = project::ignore_project_body_message(pending);
            Some(copy)
        }
        "project_ignore_reconciled_text" => {
            let label = input["project"].as_str()?;
            let promised = usize::try_from(count(input, "promised")).unwrap_or(0);
            // Legacy ABI returns an empty string rather than nil when counts agree.
            Some(
                project::ignore_project_reconciled_message(label, promised, count(input, "purged"))
                    .unwrap_or_else(|| json!("")),
            )
        }
        "arming_offer_copy_json" => {
            let label = input["project"].as_str()?;
            let count = u32::try_from(count(input, "count")).ok()?;
            let mut copy = value(project::arming_offer_copy("{project}", count));
            fields(
                &mut copy,
                operation,
                "default",
                &[
                    "question",
                    "confirm",
                    "decline",
                    "body",
                    "body_with_backlog",
                    "settings_question",
                    "settings_description",
                    "settings_decline",
                    "settings_confirm",
                ],
                &json!({"project": label}),
            );
            fields(
                &mut copy["customize"],
                operation,
                "customize",
                &[
                    "share_automatically_rule",
                    "backlog_rule",
                    "picker_heading",
                    "picker_explainer",
                    "picker_include",
                    "keep",
                    "kept_confirmation",
                    "kept_heading",
                    "undo_keep",
                    "undo_keep_queue_full",
                    "undo_keep_folder_never",
                ],
                &empty,
            );
            copy["evidence"] = project::arming_offer_evidence_message(label, count);
            Some(copy)
        }
        "contribution_override_confirm_json" => {
            let mode = input["mode"].as_str()?;
            let config = if mode == "auto_upload" {
                read_config(input)?
            } else {
                None
            };
            override_copy(mode, config.as_ref())
        }
        "contribution_override_refusal_text" => {
            let label = input["label"].as_str().unwrap_or("");
            Some(line(
                operation,
                known(label, &["arming-terms-unavailable"]),
                "line",
                project::contribution_override_refusal_line(label),
                empty,
            ))
        }
        "legacy_migration_refusal_text" => {
            let label = input["label"].as_str().unwrap_or("");
            Some(line(
                operation,
                known(label, MIGRATION_REFUSALS),
                "line",
                consent::legacy_migration_refusal_line(label),
                empty,
            ))
        }
        "automatic_contribution_copy_json" => {
            let config = read_config(input)?;
            Some(automatic(
                value(consent::automatic_contribution_copy(config.as_ref())),
                operation,
            ))
        }
        "automatic_grant_copy_json" => Some(automatic(
            value(consent::automatic_grant_copy_named(
                input["disclosure"].as_str()?,
            )?),
            operation,
        )),
        "withdrawal_confirmation_prompt_text" => Some(line(
            operation,
            "unknown_reach",
            "line",
            crate::withdraw::confirmation_prompt_unknown(),
            empty,
        )),
        "consent_gate_help" => {
            let pinned = input["pinned"]
                .as_bool()
                .unwrap_or_else(|| input["pinned"].as_i64().unwrap_or(0) != 0);
            Some(line(
                operation,
                if pinned { "pinned" } else { "unpinned" },
                "line",
                consent::gate_help(pinned),
                empty,
            ))
        }
        "health_copy_json" => health_message(input),
        "grant_void_notice" => {
            let mut copy = value(consent::void_notice_for_wire(&labeled_wire(input))?);
            let kind = input["kind"].as_str().unwrap_or("");
            let variant = match kind {
                "automatic_grant" | "contribution_override" => kind,
                "project" if project_label(input).is_some() => "project",
                _ => "unplaced",
            };
            fields(
                &mut copy,
                operation,
                variant,
                &[
                    "title",
                    "body",
                    "reasons_heading",
                    "rearm",
                    "acknowledge",
                    "rearm_action",
                    "rearm_failed",
                ],
                &json!({"project": project_label(input)}),
            );
            copy["reasons"] = reasons(input, operation, false);
            Some(copy)
        }
        "arming_reworded_notice" => {
            let mut copy = value(consent::arming_reworded_notice_for_wire(&labeled_wire(
                input,
            ))?);
            let variant = match (
                input["scrub_check_defaulted"].as_bool() == Some(true),
                project_label(input).is_some(),
            ) {
                (true, true) => "scrub_project",
                (true, false) => "scrub_unplaced",
                (false, true) => "project",
                (false, false) => "unplaced",
            };
            fields(
                &mut copy,
                operation,
                variant,
                &[
                    "title",
                    "body",
                    "now_heading",
                    "scope",
                    "limit",
                    "no_review",
                    "acknowledge",
                    "ask_first_action",
                    "ask_first_failed",
                ],
                &json!({"project": project_label(input)}),
            );
            Some(copy)
        }
        "gate_held_notice" => {
            let mut copy = value(consent::gate_held_notice_for_wire(input)?);
            fields(
                &mut copy,
                operation,
                "held",
                &["title", "release", "ask_first"],
                &empty,
            );
            copy["body"] = consent::gate_held_count_message(count(input, "held_sessions"));
            copy["reasons"] = reasons(input, operation, true);
            let projects = input["projects"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|p| project_label(p).is_some());
            for (out, source) in copy["projects"].as_array_mut()?.iter_mut().zip(projects) {
                out["line"] = consent::gate_held_project_message(
                    project_label(source)?,
                    count(source, "held_sessions"),
                );
                fields(
                    out,
                    operation,
                    "held",
                    &["ask_first_action", "ask_first_failed"],
                    &empty,
                );
            }
            Some(copy)
        }
        "legacy_migration_notice" => {
            let mut copy = value(consent::legacy_migration_notice_for_wire(input)?);
            let variant =
                if count(input, "folders_kept") > 0 || input["automatic_grant_kept"] == true {
                    "armed_kept"
                } else {
                    "nothing_armed"
                };
            fields(
                &mut copy,
                operation,
                variant,
                &["title", "body", "folders", "acknowledge"],
                &empty,
            );
            Some(copy)
        }
        "account_attention_copy_json" => {
            let mut copy = value(consent::account_attention_for_wire(input)?);
            fields(
                &mut copy,
                operation,
                "attention",
                &[
                    "title",
                    "private_ai",
                    "action",
                    "confirm_title",
                    "confirm_body",
                    "confirm_action",
                    "confirm_cancel",
                    "failed",
                ],
                &empty,
            );
            let reason = input["account_state_reason"].as_str().unwrap_or("");
            fields(
                &mut copy,
                operation,
                known(
                    reason,
                    &[
                        consent::ACCOUNT_ATTENTION_RECORD_UNREADABLE,
                        consent::ACCOUNT_ATTENTION_ADDRESS_REFUSED,
                    ],
                ),
                &["body"],
                &empty,
            );
            Some(copy)
        }
        "witness_capacity_notice" => {
            let mut copy = value(consent::witness_capacity_notice_for_wire(input)?);
            fields(
                &mut copy,
                operation,
                "waiting",
                &["title", "next_check"],
                &empty,
            );
            copy["body"] = consent::witness_capacity_body_message(count(input, "waiting_sessions"));
            Some(copy)
        }
        "route_disclosure_copy" => route_message(input),
        "quit_prompt" => {
            use crate::quit_copy::{QuitRole, quit_prompt};
            let role = input["role"].as_str()?;
            let mut copy = value(quit_prompt(match role {
                "hosting" => QuitRole::Hosting,
                "attached" => QuitRole::Attached,
                "unavailable" => QuitRole::Unavailable,
                _ => return None,
            }));
            fields(
                &mut copy,
                operation,
                role,
                &["title", "body", "confirm", "cancel"],
                &empty,
            );
            Some(copy)
        }
        _ => None,
    }
}

fn health_message(input: &Value) -> Option<Value> {
    use crate::daemon::health as labels;
    let reachable = input["reachable"]
        .as_bool()
        .unwrap_or_else(|| input["reachable"].as_i64().unwrap_or(0) != 0);
    let label = if reachable {
        input["label"].as_str().filter(|s| !s.is_empty())?
    } else {
        "core_down"
    };
    let max = input["max_queue_entries"].as_u64().filter(|n| *n > 0);
    let mut copy = value(if reachable {
        health::health_copy_for_label(label, max)
    } else {
        health::core_down_copy()
    });
    let variant = if !reachable {
        "core_down"
    } else {
        known(
            label,
            &[
                labels::LABEL_NOT_LOGGED_IN,
                labels::LABEL_NEAR_AI_NOTICE_PENDING,
                labels::LABEL_CANARY_FAILED,
                labels::LABEL_PII_FILTER_UNAVAILABLE,
                labels::LABEL_CLAIM_MINT_FAILED,
                labels::LABEL_INGEST_UNREACHABLE,
                labels::LABEL_OPENCODE_EXPORT_VERSION_UNSUPPORTED,
                labels::LABEL_QUEUE_FULL,
            ],
        )
    };
    fields(
        &mut copy,
        "health_copy_json",
        variant,
        &["title", "action"],
        &json!({}),
    );
    if reachable && label == labels::LABEL_QUEUE_FULL {
        copy["detail"] = health::queue_full_detail_message(max);
    } else {
        fields(
            &mut copy,
            "health_copy_json",
            variant,
            &["detail"],
            &json!({}),
        );
    }
    Some(copy)
}

fn route_message(input: &Value) -> Option<Value> {
    let mut result = consent::route_disclosure_for_wire(input)?;
    let facts = result["facts"].clone();
    let route = facts["route"].as_str()?;
    let copy = &mut result["copy"];
    fields(
        copy,
        "route_disclosure_copy",
        route,
        &[
            "title",
            "route",
            "disclosure_witness_route_tooltip",
            "attested_bodies",
        ],
        &json!({}),
    );
    fields(
        &mut copy["session"],
        "route_disclosure_copy",
        &format!("session_{route}"),
        &[
            "heading",
            "before_label",
            "before_line",
            "after_label",
            "after_line",
        ],
        &json!({}),
    );
    let filter = facts["local_filter"].as_str().unwrap_or("absent");
    fields(
        copy,
        "route_disclosure_copy",
        filter,
        &["local_filter"],
        &json!({}),
    );
    let receipts = if facts["receipts"]["endpoint_configured"] != true {
        "off"
    } else if facts["receipts"]["check_attestation"] == true {
        "checked"
    } else {
        "unchecked"
    };
    fields(
        copy,
        "route_disclosure_copy",
        receipts,
        &[
            "receipts",
            "disclosure_receipts_checked_tooltip",
            "disclosure_receipts_unchecked_tooltip",
        ],
        &json!({}),
    );
    fields(
        &mut copy["witness"],
        "route_disclosure_copy",
        "witness",
        &[
            "heading",
            "address_label",
            "signing_label",
            "measurements_label",
            "check",
            "classifier",
        ],
        &json!({}),
    );
    let origin = facts["witness"]["origin"]
        .as_str()
        .unwrap_or("not_recorded");
    fields(
        &mut copy["witness"],
        "route_disclosure_copy",
        origin,
        &["origin"],
        &json!({}),
    );
    Some(result)
}

/// Every conceptual variant, including forms whose live-config entry point
/// is fail-closed. No config store is opened to build this inventory.
pub fn templates() -> Vec<Value> {
    let mut trees = Vec::new();
    {
        let mut add = |op: &str, input: Value| {
            if let Some(copy) = message(op, &input) {
                trees.push(copy);
            }
        };
        for pending in [0, 1, 2] {
            add(
                "project_ignore_copy_json",
                json!({"project": "", "pending": pending}),
            );
            add(
                "project_ignore_reconciled_text",
                json!({"project": "", "promised": 3, "purged": pending}),
            );
            add(
                "arming_offer_copy_json",
                json!({"project": "", "count": pending}),
            );
        }
        for pinned in [false, true] {
            add("consent_gate_help", json!({"pinned": pinned}));
        }
        add("withdrawal_confirmation_prompt_text", json!({}));
        for label in MIGRATION_REFUSALS.iter().copied().chain(["unknown"]) {
            add("legacy_migration_refusal_text", json!({"label": label}));
        }
        for label in ["arming-terms-unavailable", "unknown"] {
            add(
                "contribution_override_refusal_text",
                json!({"label": label}),
            );
        }
        for role in ["hosting", "attached", "unavailable"] {
            add("quit_prompt", json!({"role": role}));
        }
        add("health_copy_json", json!({"reachable": false}));
        for label in crate::daemon::health::ALL_LABELS
            .iter()
            .copied()
            .chain(["unknown"])
        {
            for max in [0, 1, 1000] {
                add(
                    "health_copy_json",
                    json!({"reachable": true, "label": label, "max_queue_entries": max}),
                );
            }
        }
        for kind in [
            "automatic_grant",
            "contribution_override",
            "project",
            "unknown",
        ] {
            for label in ["", "Project"] {
                for id in ["", "id"] {
                    add(
                        "grant_void_notice",
                        json!({"kind": kind, "project_label": label, "project_id": id,
                    "reasons": VOID_REASONS.iter().copied().chain(["unknown"]).collect::<Vec<_>>()}),
                    );
                }
            }
        }
        for label in ["", "Project"] {
            for scrub in [false, true] {
                add(
                    "arming_reworded_notice",
                    json!({"project_label": label, "project_id": "id", "scrub_check_defaulted": scrub}),
                );
            }
        }
        for n in [1, 2] {
            add("witness_capacity_notice", json!({"waiting_sessions": n}));
            add(
                "gate_held_notice",
                json!({"held_sessions": n, "reasons": HELD_REASONS.iter().copied().chain(["unknown"]).collect::<Vec<_>>(),
            "projects": [{"project_label": "Project", "project_id": "id", "held_sessions": n}]}),
            );
        }
        for kept in [false, true] {
            add(
                "legacy_migration_notice",
                json!({"automatic_grant_kept": kept}),
            );
        }
        for reason in [
            consent::ACCOUNT_ATTENTION_RECORD_UNREADABLE,
            consent::ACCOUNT_ATTENTION_ADDRESS_REFUSED,
            "unknown",
        ] {
            add(
                "account_attention_copy_json",
                json!({"account_state": "needs_attention", "account_state_reason": reason, "near_ai_login_unconfirmed": true}),
            );
        }
        for route in [
            "witness",
            "witness_refusing",
            "local",
            "not_enrolled",
            "settings_unreadable",
        ] {
            for filter in ["none", "near_ai", "self_hosted", "sidecar", "invalid"] {
                for origin in [
                    "published_at_join",
                    "connected_inference",
                    "settings",
                    "environment",
                    "not_recorded",
                ] {
                    for (endpoint, checked) in [(false, false), (true, false), (true, true)] {
                        add(
                            "route_disclosure_copy",
                            json!({"route": route, "local_filter": filter,
                        "receipts": {"endpoint_configured": endpoint, "check_attestation": checked}, "attested_bodies": true,
                        "witness": {"state": "pinned", "url": "", "signing_address": "", "pinned_measurements": [], "origin": origin}}),
                        );
                    }
                }
            }
        }
    }
    for mode in ["notify_only", "auto_upload", "ignore", "clear"] {
        trees.push(override_copy(mode, None).expect("known mode"));
    }
    for disclosure in ["patterns_only", "model_scrubbed"] {
        trees.push(automatic(
            value(consent::automatic_grant_copy_named(disclosure).expect("known disclosure")),
            "automatic_grant_copy_json",
        ));
    }
    trees.push(automatic(
        value(consent::automatic_contribution_copy(None)),
        "automatic_contribution_copy_json",
    ));
    trees.push(consent::gate_held_count_message(0));
    trees.push(consent::witness_capacity_body_message(0));
    let mut messages = std::collections::BTreeMap::new();
    fn collect(tree: &Value, out: &mut std::collections::BTreeMap<String, Value>) {
        if let Some(key) = tree["key"].as_str() {
            if let Some(previous) = out.get(key) {
                assert_eq!(
                    previous["english"], tree["english"],
                    "conflicting disclosure key: {key}"
                );
            }
            out.entry(key.to_owned()).or_insert_with(|| tree.clone());
        } else {
            match tree {
                Value::Object(object) => {
                    for child in object.values() {
                        collect(child, out);
                    }
                }
                Value::Array(array) => {
                    for child in array {
                        collect(child, out);
                    }
                }
                _ => {}
            }
        }
    }
    for tree in trees {
        collect(&tree, &mut messages);
    }
    messages.into_values().collect()
}
