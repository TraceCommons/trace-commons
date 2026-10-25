//! Discoverable configuration for agents, using the same parser as the app.
//! A preview is a cloned candidate, never a reservation or a settings write.

use serde_json::{Map, Value, json};

use super::settings::{self, DaemonSettings};

pub(crate) fn schema() -> Value {
    let mut properties = Map::new();
    let ranges =
        serde_json::to_value(settings::settings_ranges()).expect("settings ranges serialize");
    for (key, bounds) in ranges.as_object().expect("settings ranges object") {
        properties.insert(key.clone(), json!({
            "type":"integer", "minimum":bounds["min"], "maximum":bounds["max"],
            "description": match key.as_str() {
                "quiescence_secs" => "Seconds without writes before a trace is considered finished.",
                "approval_hold_secs" => "Seconds before an approved upload can start; zero removes the undo hold.",
                "digest_interval_secs" => "Seconds between interval notifications.",
                "max_uploads_per_day" => "Local daily upload count limit; does not grant upload permission.",
                _ => "Local daily upload byte limit; does not grant upload permission.",
            },
            "examples":[bounds["min"]]
        }));
    }
    for (key, description) in [
        (
            "local_notifications",
            "Enable daemon-generated local notifications.",
        ),
        (
            "token_distributions_contribution",
            "Allow contribution of token distributions under existing consent and admission rules.",
        ),
        (
            "ironwire_attested_bodies",
            "Allow inference content to be sent for attestation under existing consent rules; separate from routing metadata.",
        ),
        (
            "token_capture_enabled",
            "Enable optional inference token capture; does not repoint coding tools.",
        ),
        (
            "private_inference",
            "Start or stop this daemon's private inference proxy. Apply reports runtime state separately from the saved preference.",
        ),
        (
            "private_inference_offer_seen",
            "Record that the private inference offer was shown; does not start inference.",
        ),
        ("suggestions_enabled", "Enable in-app suggestions."),
        (
            "settlement_estimate_enabled",
            "Enable local trace credit estimates; disabling removes the estimate cache. Estimates are not guaranteed payouts.",
        ),
        (
            "missions_enabled",
            "Enable mission matches in trace and suggestion surfaces.",
        ),
        ("notifications_enabled", "Master switch for notifications."),
        ("menu_bar_mark_enabled", "Show the menu-bar news mark."),
        (
            "verdicts_offer_pending",
            "Whether the one-time verdict notification offer remains pending.",
        ),
        (
            "idle_offer_pending",
            "Whether the one-time idle-session notification offer remains pending.",
        ),
        ("insights_ledger_feed", "Enable the Insights ledger feed."),
        (
            "insights_counter_pass",
            "Enable the optional Insights counter pass.",
        ),
        (
            "insights_recap_card_enabled",
            "Enable the weekly Insights recap card; it also needs the counter feed.",
        ),
    ] {
        properties.insert(
            key.into(),
            json!({"type":"boolean","description":description,"examples":[false]}),
        );
    }
    for key in ["claude_root", "codex_root"] {
        properties.insert(key.into(), json!({
            "type":["string","null"], "examples":[null],
            "description":"Legacy alias for the corresponding source declaration: a path watches that folder; null clears the declaration. Paths are redacted in inspection and preview.",
            "x-canonical-setting":key.replace("_root", "_source")
        }));
    }
    let source = json!({"anyOf":[
        {"type":"null"},
        {"type":"object","required":["mode"],"properties":{"mode":{"const":"off"}}},
        {"type":"object","required":["mode","path"],"properties":{"mode":{"const":"watch"},"path":{"type":"string"}}}
    ],"examples":[null],"description":"Source declaration: null means not yet declared, off disables discovery, watch names a local folder. Replaces the whole declaration. Paths are redacted in inspection and preview."});
    for key in [
        "claude_source",
        "codex_source",
        "gemini_source",
        "cline_source",
        "opencode_source",
        "trajectory_source",
    ] {
        let mut entry = source.clone();
        if key == "trajectory_source" {
            entry["x-constraints"] = json!([
                "Watch path must be absolute and cannot be the filesystem root or the current user's home directory."
            ]);
        }
        properties.insert(key.into(), entry);
    }
    properties.insert("ironwire".into(), json!({
        "anyOf":[{"type":"null"},
            {"type":"object","required":["mode"],"properties":{"mode":{"const":"off"}}},
            {"type":"object","required":["mode","port"],"properties":{
                "mode":{"const":"watch"},"port":{"type":"integer","minimum":1,"maximum":65535},
                "token_dir":{"type":["string","null"],"description":"An absolute directory containing the proxy control token, never the token itself. Empty string clears the directory."}
            }}],
        "examples":[null],
        "description":"Read metadata from an existing local proxy; null or off stops watching it. Replaces the whole declaration. Does not authorize sending inference bodies or start a proxy."
    }));
    properties.insert("digest_schedule".into(), json!({
        "oneOf":[
            {"type":"object","required":["mode"],"properties":{"mode":{"const":"interval"}}},
            {"type":"object","required":["mode"],"properties":{"mode":{"const":"evening"},"hour":{"type":"integer","minimum":0,"maximum":23,"default":18}}}
        ],"examples":[{"mode":"interval"}],
        "description":"Replace the notification schedule. Evening hour is local time; omitted hour defaults to 18."
    }));
    let notify: Map<String, Value> = settings::notify_kind_labels()
        .into_iter()
        .map(|key| (key.into(), json!({"type":"boolean"})))
        .collect();
    properties.insert("notify".into(), json!({
        "type":"object","properties":notify,"additionalProperties":false,"examples":[{}],
        "description":"Partial notification-kind update: omitted kinds remain unchanged. Answering verdicts_landed or idle_sessions also clears its pending offer."
    }));
    let terminals: Vec<_> = crate::managed::terminal::TerminalApp::offered()
        .iter()
        .map(|app| app.id())
        .collect();
    let mut terminal = json!({
        "type":"string", "examples":terminals.first().into_iter().collect::<Vec<_>>(),
        "description":"Terminal application for managed sessions; only this platform's offered identifiers are accepted."
    });
    if terminals.is_empty() {
        // No accepted value on an unsupported platform, without an invalid
        // empty enum or assuming at least one offered terminal exists.
        terminal["not"] = json!({});
    } else {
        terminal["enum"] = json!(terminals);
    }
    properties.insert("managed_terminal".into(), terminal);
    properties.insert("scrub_check".into(), json!({
        "type":"string","enum":["automatic","manual"],"examples":["manual"],
        "description":"Privacy-review mode. Switching to manual returns unsent unattended approvals to waiting. Does not bypass upload consent."
    }));
    properties.insert(
        "insights_context_threshold".into(),
        json!({
            "type":["integer","null"],"minimum":settings::INSIGHTS_CONTEXT_THRESHOLD_MIN,
            "maximum":settings::INSIGHTS_CONTEXT_THRESHOLD_MAX,"examples":[null],
            "description":"Context tip threshold; null clears it rather than selecting a default."
        }),
    );
    json!({
        "schema_version":"trace_commons.config_schema.v1",
        "patch_schema":{
            "$schema":"https://json-schema.org/draft/2020-12/schema",
            "type":"object", "minProperties":1,"additionalProperties":false,"properties":properties
        },
        "semantics":{
            "patch":"Partial settings object; omitted top-level fields remain unchanged. Structured declarations replace their value except notify, which merges supplied kind switches.",
            "preview":"Validation and proposed canonical changes only; does not persist, start services, submit traces, or reserve a transaction.",
            "apply":"Revalidates against current settings. Runtime result fields are separate from the saved preference.",
            "contextual_validation":"Filesystem and platform constraints are checked by the daemon parser; JSON Schema alone is not sufficient validation.",
            "inspection":"The settings response includes read-only and redacted fields; do not copy it wholesale into a patch."
        }
    })
}

pub(crate) fn preview(settings: &DaemonSettings, patch: &Value) -> Result<Value, &'static str> {
    let mut candidate = settings.clone();
    if !settings::apply_settings_object(&mut candidate, patch)? {
        return Err("no-known-setting-supplied");
    }
    // Compare internally before redaction: two different source paths both
    // display as watch, but the requested change must still be visible.
    let mut before = serde_json::to_value(settings).map_err(|_| "settings-preview-unavailable")?;
    let mut after = serde_json::to_value(&candidate).map_err(|_| "settings-preview-unavailable")?;
    let mut shown_before = super::ipc::redacted_settings(settings);
    let mut shown_after = super::ipc::redacted_settings(&candidate);
    // These booleans are omitted from persisted JSON when false. A preview
    // describes their effective value, not the omission used to encode it.
    for (value, state) in [
        (&mut before, settings),
        (&mut shown_before, settings),
        (&mut after, &candidate),
        (&mut shown_after, &candidate),
    ] {
        value["verdicts_offer_pending"] = json!(state.verdicts_offer_pending);
        value["idle_offer_pending"] = json!(state.idle_offer_pending);
    }
    let keys: std::collections::BTreeSet<_> = before
        .as_object()
        .into_iter()
        .flat_map(|o| o.keys())
        .chain(after.as_object().into_iter().flat_map(|o| o.keys()))
        .collect();
    let mut changes = Vec::new();
    for key in keys {
        if before.get(key) == after.get(key) {
            continue;
        }
        let old = shown_before.get(key).cloned().unwrap_or(Value::Null);
        let new = shown_after.get(key).cloned().unwrap_or(Value::Null);
        let redacted = before.get(key).is_some_and(|raw| raw != &old)
            || after.get(key).is_some_and(|raw| raw != &new);
        changes.push(json!({"key":key,"before":old,"after":new,"redacted":redacted}));
    }
    Ok(
        json!({"schema_version":"trace_commons.settings_preview.v1","applied":false,"changes":changes}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_config_schema_covers_every_accepted_setting_and_examples_validate() {
        let schema = schema();
        let props = schema["patch_schema"]["properties"].as_object().unwrap();
        let source = include_str!("settings.rs");
        let arms = source
            .split("// SET-SETTINGS-KEYS-BEGIN")
            .nth(1)
            .unwrap()
            .split("// SET-SETTINGS-KEYS-END")
            .next()
            .unwrap();
        let keys: std::collections::BTreeSet<&str> = arms
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                line.starts_with('"')
                    .then(|| line.split('"').nth(1).unwrap())
            })
            .collect();
        assert_eq!(keys, props.keys().map(String::as_str).collect());
        for (key, property) in props {
            for example in property["examples"].as_array().unwrap() {
                let mut settings = DaemonSettings::default();
                assert!(
                    settings::apply_settings_object(&mut settings, &json!({key:example})).is_ok(),
                    "{key}"
                );
            }
        }
        for secret in [
            "near_ai",
            "near_ai_inference",
            "near_ai_session",
            "cloud_credentials",
        ] {
            assert!(!props.contains_key(secret));
        }
    }

    #[test]
    fn agent_config_preview_reports_alias_and_redacted_changes_without_mutation() {
        let mut settings = DaemonSettings::default();
        settings::apply_settings_object(
            &mut settings,
            &json!({"claude_root":"/private/old-sensitive-folder"}),
        )
        .unwrap();
        let original = serde_json::to_value(&settings).unwrap();
        let result = preview(
            &settings,
            &json!({"claude_root":"/private/new-sensitive-folder"}),
        )
        .unwrap();
        assert_eq!(
            result["changes"],
            json!([{"key":"claude_source","before":null,"after":null,"redacted":true}])
        );
        assert!(!result.to_string().contains("sensitive-folder"));
        assert_eq!(serde_json::to_value(&settings).unwrap(), original);
    }

    #[test]
    fn agent_config_preview_includes_nested_side_effects_and_preserves_omitted_fields() {
        let settings = DaemonSettings {
            verdicts_offer_pending: true,
            ..DaemonSettings::default()
        };
        let result = preview(
            &settings,
            &json!({"notify":{"verdicts_landed":false},"max_uploads_per_day":12}),
        )
        .unwrap();
        let changes = result["changes"].as_array().unwrap();
        assert!(
            changes
                .iter()
                .any(|c| c["key"] == "max_uploads_per_day" && c["after"] == 12)
        );
        assert!(changes.iter().all(|c| c["key"] != "max_bytes_per_day"));
        if settings.verdicts_offer_pending {
            assert!(
                changes
                    .iter()
                    .any(|c| c["key"] == "verdicts_offer_pending" && c["after"] == false)
            );
        }
        assert_eq!(
            preview(
                &settings,
                &json!({"max_uploads_per_day":settings.max_uploads_per_day})
            )
            .unwrap()["changes"],
            json!([])
        );
    }

    #[test]
    fn agent_config_preview_never_exposes_or_changes_saved_credentials() {
        let settings = DaemonSettings {
            near_ai_inference: Some(settings::NearAiInferenceCredential {
                key: "sk-sensitive-sentinel".into(),
                key_id: "key-id".into(),
                key_prefix: "sk-sensitive".into(),
                organization_id: "org".into(),
                workspace_id: "workspace".into(),
                minted_at: chrono::Utc::now(),
            }),
            ..DaemonSettings::default()
        };
        let result = preview(&settings, &json!({"max_uploads_per_day":12})).unwrap();
        assert!(!result.to_string().contains("sensitive"));
        assert_eq!(result["changes"].as_array().unwrap().len(), 1);
        assert_eq!(
            settings.near_ai_inference.as_ref().unwrap().key,
            "sk-sensitive-sentinel"
        );
        assert!(preview(&settings, &json!({"near_ai_inference":null})).is_err());
    }

    #[test]
    fn agent_config_preview_refuses_invalid_patches_without_echoing_values() {
        for patch in [
            json!({}),
            json!({"unknown-secret-value":"secret"}),
            json!({"max_uploads_per_day":0}),
            json!({"digest_schedule":{"mode":"evening","hour":24}}),
            json!({"notify":{"unknown":true}}),
            json!({"managed_terminal":"/secret/command"}),
            json!({"private_inference":true,"quiescence_secs":false}),
        ] {
            let settings = DaemonSettings::default();
            let before = serde_json::to_value(&settings).unwrap();
            let error = preview(&settings, &patch).unwrap_err();
            assert!(!error.contains("secret"));
            assert_eq!(serde_json::to_value(&settings).unwrap(), before);
        }
    }
}
