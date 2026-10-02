//! The health banner's words: the core-down sentence for when the daemon
//! cannot be reached at all, and the per-label sentence for a condition a
//! reachable daemon reported through `status.health.last_error_label`.
//!
//! K3 (#1173) moved the per-label table here word for word from
//! `macos/Sources/TraceCommonsApp/HealthCopy.swift`, which has shipped it
//! since before this module existed; Windows
//! (`windows/src/TraceCommons.Interop/HealthCopy.cs`) independently wrote the
//! same sentences by hand. Neither the C ABI nor IPC exported them, so there
//! was no single spelling a third shell -- or a native macOS screen reading
//! across the C ABI instead of hand-typing Swift -- could read. This module
//! is that single spelling now; the Swift and C# tables are not touched by
//! this change and still carry their own copies until a shell PR switches
//! them over.
//!
//! Two rules bind every sentence below, both the shared design's:
//! - **Never name the mechanism.** "Privacy filter", "claim", "ingest",
//!   "canary" and "PII" are internal words. The label carries them; the
//!   sentence must not.
//! - **Always state the data consequence.** "Nothing has been lost", "your
//!   queue is safe", "rather than going out unscanned".
//!
//! `status.health.last_error_label` carries ONE label at a time, already
//! resolved by the daemon's precedence order (`daemon::health::precedence`).
//! [`health_copy_for_label`] does not reconstruct that order and must not: it
//! renders whichever label arrives, never ranks, merges or synthesises one.
//!
//! `opencode-export-version-unsupported` is deliberately absent from
//! [`health_copy_for_label`]'s table. Both existing shells special-case that
//! label before ever reaching their `forLabel` switch, reading the version
//! sentence from `source_copy::settings_copy` (`tc_source_settings_copy`)
//! instead; a caller here must keep doing the same rather than asking this
//! function for it.

use serde::Serialize;

/// The health banner's words for one condition: a title, the sentence that
/// states what is held and the data consequence, and the action button's
/// label where a condition has a real recovery step. `action` is `None` for
/// every condition that clears on its own -- a button beside a banner that
/// cannot change what it sits next to teaches a contributor the buttons in
/// this app do nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HealthLineCopy {
    pub title: String,
    pub detail: String,
    pub action: Option<String>,
}

impl HealthLineCopy {
    fn new(title: &'static str, detail: &'static str, action: Option<&'static str>) -> Self {
        Self {
            title: title.to_string(),
            detail: detail.to_string(),
            action: action.map(str::to_string),
        }
    }
}

/// The banner title for a spent daily budget. Shared with the real-numbers
/// banner a shell draws instead when it has `status.daily_budget`
/// (`daemon::uploader`'s daily-cap accounting); this is only the fallback for
/// a daemon that reported the label without that object.
pub const DAILY_BUDGET_TITLE: &str = "Today's upload limit is used up.";

/// **DRAFT, NEEDS APPROVAL.** The banner for a daemon that cannot be reached
/// at all -- no IPC call is answering, so there is no label to read and
/// nothing a shell's own liveness probe can do but say so. New wording: no
/// shell has shown a contributor-facing sentence for this condition before,
/// only the CLI's own operational text (`daemon: not reachable`).
pub fn core_down_copy() -> HealthLineCopy {
    HealthLineCopy::new(
        "Can't reach Trace Commons' background service.",
        "Nothing is being watched while it's down. Sessions already sent are safe, and \
         nothing new will be queued until it's running again.",
        None,
    )
}

/// The banner for one `last_error_label`, word for word from the macOS and
/// Windows shells' shared table. An unrecognised label is still a real
/// condition -- the daemon is free to grow labels this build has never heard
/// of -- so it gets the same fallback sentence both shells already draw
/// rather than inventing a cause or rendering the raw label, which would
/// break the never-name-the-mechanism rule by the most direct route there
/// is.
pub fn health_copy_for_label(label: &str) -> HealthLineCopy {
    use crate::daemon::health;

    match label {
        health::LABEL_NOT_LOGGED_IN => HealthLineCopy::new(
            "Not connected.",
            "Sessions are being queued, but nothing can be sent until you reconnect. Nothing \
             has been lost.",
            Some("Reconnect"),
        ),
        health::LABEL_NEAR_AI_NOTICE_PENDING => {
            // The recovery prompt's words are the core's own
            // (`privacy_scan_copy`), the same ones onboarding's scan screen
            // reads; this delegates rather than repeating them so the two
            // surfaces cannot drift apart.
            let copy = crate::privacy_scan_copy::privacy_scan_copy();
            HealthLineCopy::new(
                copy.recovery_title,
                copy.recovery_detail,
                Some(copy.recovery_action),
            )
        }
        health::LABEL_CANARY_FAILED => HealthLineCopy::new(
            "The privacy scan failed its own self-test,",
            "so nothing is being sent through it. This is deliberate -- a scan we can't verify \
             doesn't get used.",
            None,
        ),
        health::LABEL_PII_FILTER_UNAVAILABLE => HealthLineCopy::new(
            "The extra privacy scan isn't reachable.",
            "Your traces are waiting rather than going out unscanned. Retrying automatically.",
            None,
        ),
        health::LABEL_CLAIM_MINT_FAILED | health::LABEL_INGEST_UNREACHABLE => HealthLineCopy::new(
            "Can't reach Trace Commons right now.",
            "Your queue is safe; it'll retry on its own.",
            None,
        ),
        health::LABEL_QUEUE_FULL => HealthLineCopy::new(
            "Trace Commons has stopped queuing new sessions",
            "-- 500 are already waiting. Review or clear some to start again.",
            Some("Review"),
        ),
        health::LABEL_DAILY_CAP_REACHED => HealthLineCopy::new(
            DAILY_BUDGET_TITLE,
            "Approved traces are waiting. Nothing has been lost -- they go out when the limit \
             resets.",
            None,
        ),
        _ => HealthLineCopy::new(
            "Contributions are on hold.",
            "Something is stopping traces from being sent. Nothing has been lost, and nothing \
             has gone out.",
            None,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_down_states_the_data_consequence_and_is_marked_draft() {
        let copy = core_down_copy();
        assert_eq!(copy.title, "Can't reach Trace Commons' background service.");
        assert!(copy.detail.contains("Sessions already sent are safe"));
        assert!(copy.action.is_none());
    }

    #[test]
    fn every_known_label_has_words_and_names_no_mechanism() {
        use crate::daemon::health;
        let labels = [
            health::LABEL_NOT_LOGGED_IN,
            health::LABEL_NEAR_AI_NOTICE_PENDING,
            health::LABEL_CANARY_FAILED,
            health::LABEL_PII_FILTER_UNAVAILABLE,
            health::LABEL_CLAIM_MINT_FAILED,
            health::LABEL_INGEST_UNREACHABLE,
            health::LABEL_QUEUE_FULL,
            health::LABEL_DAILY_CAP_REACHED,
        ];
        for label in labels {
            let copy = health_copy_for_label(label);
            assert!(!copy.title.is_empty(), "{label}");
            assert!(!copy.detail.is_empty(), "{label}");
            for mechanism in ["privacy filter", "claim", "ingest", "canary", "PII"] {
                assert!(
                    !copy
                        .detail
                        .to_lowercase()
                        .contains(&mechanism.to_lowercase()),
                    "{label}'s detail names the mechanism `{mechanism}`"
                );
            }
        }
    }

    #[test]
    fn an_unknown_label_gets_the_generic_on_hold_sentence_not_the_raw_label() {
        let copy = health_copy_for_label("a-future-label-this-build-has-never-heard-of");
        assert_eq!(copy.title, "Contributions are on hold.");
        assert!(!copy.detail.contains("a-future-label"));
    }

    #[test]
    fn only_labels_with_a_real_recovery_step_carry_an_action() {
        use crate::daemon::health;
        for label in [
            health::LABEL_NOT_LOGGED_IN,
            health::LABEL_NEAR_AI_NOTICE_PENDING,
            health::LABEL_QUEUE_FULL,
        ] {
            assert!(
                health_copy_for_label(label).action.is_some(),
                "{label} should offer a recovery action"
            );
        }
        for label in [
            health::LABEL_CANARY_FAILED,
            health::LABEL_PII_FILTER_UNAVAILABLE,
            health::LABEL_CLAIM_MINT_FAILED,
            health::LABEL_INGEST_UNREACHABLE,
            health::LABEL_DAILY_CAP_REACHED,
        ] {
            assert!(
                health_copy_for_label(label).action.is_none(),
                "{label} clears on its own and must not offer a dead button"
            );
        }
    }

    #[test]
    fn the_near_ai_recovery_matches_the_privacy_scan_copy_source() {
        let copy = health_copy_for_label(crate::daemon::health::LABEL_NEAR_AI_NOTICE_PENDING);
        let source = crate::privacy_scan_copy::privacy_scan_copy();
        assert_eq!(copy.title, source.recovery_title);
        assert_eq!(copy.detail, source.recovery_detail);
        assert_eq!(copy.action.as_deref(), Some(source.recovery_action));
    }

    #[test]
    fn serialized_copy_carries_every_field() {
        let value = serde_json::to_value(core_down_copy()).unwrap();
        assert!(value["title"].as_str().is_some_and(|t| !t.is_empty()));
        assert!(value["detail"].as_str().is_some_and(|t| !t.is_empty()));
        assert!(value["action"].is_null());
    }
}
