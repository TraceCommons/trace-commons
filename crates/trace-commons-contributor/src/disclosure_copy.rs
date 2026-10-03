//! The disclosure bundle: every table of shared copy the onboarding,
//! settings, history and Private AI screens read, assembled in one call.
//!
//! K5 (#1173) moved this out of the Tauri shell, where it was assembled
//! inside a Tauri command and no other shell could reach it. Tauri's
//! `contributor_disclosure_copy` command now returns this function's value
//! unchanged, and the C ABI exports it as
//! `tc_contributor_disclosure_copy_json`.
//!
//! # Words only
//!
//! Nothing here writes a sentence. Each field is a table or a sentence a
//! copy module already owns, carried whole or by name, and the one choice it
//! carries -- which runtime state may be painted as working -- is
//! [`private_inference_copy::state_copies`]'s, not a shell's.

use serde_json::{Value, json};

use crate::private_inference_copy;
use crate::source_copy;

/// The source tools whose watch, unset and off check lines the bundle
/// carries, by the key the source settings use.
const SOURCE_CHECK_TOOLS: [&str; 5] = ["claude", "codex", "gemini", "cline", "opencode"];

/// The disclosure bundle, as Tauri's `contributor_disclosure_copy` command
/// and `tc_contributor_disclosure_copy_json` return it.
///
/// `private_inference.states` is the core's sentence for each of the ten
/// runtime state labels a daemon reports (`private_inference_state.state`),
/// and whether an indicator may paint that state as working. A shell looks
/// the label up; a label it does not find is `state_unknown`, an absent one
/// `state_unreported`, and neither is working.
#[must_use]
pub fn contributor_disclosure_copy() -> Value {
    let witness = crate::witness_copy::witness_copy();
    let inference = private_inference_copy::private_inference_copy();
    // The contributor core names this hold once, in its shared status table;
    // History reads that label rather than keeping a second spelling here.
    let awaiting_pii_backstop = crate::public_run::public_run_copy()
        .contribution_status_choices
        .into_iter()
        .find(|choice| choice.value == "awaiting_pii_backstop")
        .map(|choice| choice.label);
    let source_checks = SOURCE_CHECK_TOOLS
        .into_iter()
        .filter_map(|key| {
            source_copy::SourceTool::from_key(key).map(|tool| {
                (
                    key.to_owned(),
                    json!({
                        "watch": source_copy::source_check_line(tool, "watch"),
                        "unset": source_copy::source_check_line(tool, "unset"),
                        "off": source_copy::source_check_line(tool, "off"),
                    }),
                )
            })
        })
        .collect::<serde_json::Map<String, Value>>();
    json!({
        "witness_review": witness.review,
        "wallet": witness.wallet,
        "admission": witness.admission,
        "onboarding": witness.onboarding,
        "onboarding_shell": crate::onboarding_copy::onboarding_copy(),
        "privacy_scan": crate::privacy_scan_copy::privacy_scan_copy(),
        "source_settings": source_copy::source_settings_copy(),
        "source_check_lines": source_checks,
        "insights_ui": crate::insights::service::ui_copy(),
        "mission_drafts_ui": crate::mission_draft_service::ui_copy(),
        "history_ui": {
            "held_row_body": crate::history_copy::HELD_ROW_BODY,
            "status_awaiting_pii_backstop": awaiting_pii_backstop,
        },
        "outcome": crate::outcome_copy::outcome_copy(),
        "private_inference": {
            "destination": inference.destination,
            "subtitle": inference.subtitle,
            "settings_title": inference.settings_title,
            "write_unconfirmed": inference.write_unconfirmed,
            "offer_title": inference.offer_title,
            "offer_what": inference.offer_what,
            "offer_exposure": inference.offer_exposure,
            "offer_no_repoint": inference.offer_no_repoint,
            "offer_accept": inference.offer_accept,
            "offer_decline": inference.offer_decline,
            "offer_asked_once": inference.offer_asked_once,
            "states": private_inference_copy::state_copies(),
            "state_unknown": inference.state_unknown,
            "state_unreported": inference.state_unreported,
        },
        "project_automatic_unavailable":
            crate::consent_copy::AUTO_PROJECT_DISCLOSURE_UNAVAILABLE,
        "credential_cost": private_inference_copy::CREDENTIAL_COST,
        "credential_wallet_notice": private_inference_copy::CREDENTIAL_WALLET_NOTICE,
        "near_ai_enroll_title": inference.near_ai_enroll_title,
        "near_ai_enroll_what": inference.near_ai_enroll_what,
        "near_ai_enroll_action": inference.near_ai_enroll_action,
        "near_ai_enroll_needs_login": inference.near_ai_enroll_needs_login,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::contributor_disclosure_copy;
    use crate::private_inference_copy::{
        DESTINATION, LABEL_OFF, LABEL_PORT_IN_USE, LABEL_RUNNING, LABEL_RUNNING_NO_BACKENDS,
        SETTINGS_TITLE, STATE_LABELS, STATE_UNKNOWN, STATE_UNREPORTED, SUBTITLE, WRITE_UNCONFIRMED,
        state_line, state_tone,
    };

    /// Every top-level table a shell reads is present, under the name the
    /// Tauri adapter and the macOS bridge read it by.
    #[test]
    fn the_bundle_carries_every_table_by_name() {
        let copy = contributor_disclosure_copy();
        let keys: Vec<&str> = copy
            .as_object()
            .expect("the bundle is an object")
            .keys()
            .map(String::as_str)
            .collect();
        let mut expected = [
            "witness_review",
            "wallet",
            "admission",
            "onboarding",
            "onboarding_shell",
            "privacy_scan",
            "source_settings",
            "source_check_lines",
            "insights_ui",
            "mission_drafts_ui",
            "history_ui",
            "outcome",
            "private_inference",
            "project_automatic_unavailable",
            "credential_cost",
            "credential_wallet_notice",
            "near_ai_enroll_title",
            "near_ai_enroll_what",
            "near_ai_enroll_action",
            "near_ai_enroll_needs_login",
        ];
        let mut keys = keys;
        keys.sort_unstable();
        expected.sort_unstable();
        assert_eq!(keys, expected);
        assert_eq!(
            copy["witness_review"],
            serde_json::to_value(crate::witness_copy::witness_copy().review).unwrap()
        );
        assert_eq!(
            copy["privacy_scan"],
            serde_json::to_value(crate::privacy_scan_copy::privacy_scan_copy()).unwrap()
        );
        assert_eq!(
            copy["onboarding_shell"],
            serde_json::to_value(crate::onboarding_copy::onboarding_copy()).unwrap()
        );
    }

    /// K6: an armed project's failure line reaches the shell from the core.
    #[test]
    fn the_bundle_carries_the_project_disclosure_failure_line() {
        assert_eq!(
            contributor_disclosure_copy()["project_automatic_unavailable"],
            crate::consent_copy::AUTO_PROJECT_DISCLOSURE_UNAVAILABLE
        );
    }

    #[test]
    fn private_ai_copy_carries_the_shared_destination_and_its_surrounding_lines() {
        let copy = contributor_disclosure_copy();
        for (key, expected) in [
            ("destination", DESTINATION),
            ("subtitle", SUBTITLE),
            ("settings_title", SETTINGS_TITLE),
            ("write_unconfirmed", WRITE_UNCONFIRMED),
        ] {
            assert_eq!(
                copy.pointer(&format!("/private_inference/{key}"))
                    .and_then(Value::as_str),
                Some(expected),
                "{key}"
            );
        }
    }

    /// #1146's state map: the runtime state a shell shows is the core's
    /// sentence for the daemon's label, and only the state the core paints
    /// as working may be drawn as on.
    #[test]
    fn private_ai_state_copy_is_the_cores_line_and_tone_for_each_label() {
        let copy = contributor_disclosure_copy();
        let states = copy
            .pointer("/private_inference/states")
            .and_then(Value::as_object)
            .expect("state copy is carried");
        assert_eq!(states.len(), 10);
        for label in STATE_LABELS {
            assert_eq!(states[label]["line"], state_line(label), "{label}");
            assert_eq!(
                states[label]["working"],
                state_tone(label).reads_as_working(),
                "{label}"
            );
            assert_eq!(states[label].as_object().map(serde_json::Map::len), Some(2));
        }
        for label in [
            LABEL_OFF,
            LABEL_RUNNING,
            LABEL_RUNNING_NO_BACKENDS,
            LABEL_PORT_IN_USE,
        ] {
            assert_eq!(states[label]["working"], label == LABEL_RUNNING, "{label}");
        }
        assert!(states.values().all(|state| state["line"] != STATE_UNKNOWN));
        assert_eq!(copy["private_inference"]["state_unknown"], STATE_UNKNOWN);
        assert_eq!(
            copy["private_inference"]["state_unreported"],
            STATE_UNREPORTED
        );
    }

    #[test]
    fn history_copy_names_the_privacy_backstop_hold_from_the_shared_status_table() {
        let copy = contributor_disclosure_copy();
        let shared = crate::public_run::public_run_copy()
            .contribution_status_choices
            .into_iter()
            .find(|choice| choice.value == "awaiting_pii_backstop")
            .expect("the shared status table names awaiting_pii_backstop")
            .label;
        assert_eq!(
            copy.pointer("/history_ui/status_awaiting_pii_backstop")
                .and_then(Value::as_str),
            Some(shared)
        );
        assert_eq!(
            copy["history_ui"]["held_row_body"],
            crate::history_copy::HELD_ROW_BODY
        );
    }

    /// Each source tool's three check lines are the source table's.
    #[test]
    fn the_source_check_lines_are_the_source_tables() {
        use crate::source_copy::{SourceTool, source_check_line};
        let copy = contributor_disclosure_copy();
        let lines = copy["source_check_lines"]
            .as_object()
            .expect("source check lines are carried");
        assert_eq!(lines.len(), super::SOURCE_CHECK_TOOLS.len());
        for key in super::SOURCE_CHECK_TOOLS {
            let tool = SourceTool::from_key(key).expect("a known source tool");
            for mode in ["watch", "unset", "off"] {
                assert_eq!(
                    lines[key][mode],
                    source_check_line(tool, mode),
                    "{key} {mode}"
                );
            }
        }
    }
}
