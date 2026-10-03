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
//! `opencode-export-version-unsupported` is answered here too, by delegating
//! to `source_copy::source_settings_copy` (`tc_source_settings_copy`), the
//! words both shells already read for it, so a caller that asks this module
//! gets the version sentence rather than the generic on-hold banner.

use serde::Serialize;

/// How a banner sits: the same two kinds Swift's `HealthCopy.Severity` uses
/// for a banner (its third, `informational`, is a menu line, never one of
/// these).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthSeverity {
    /// Something the contributor can act on.
    Actionable,
    /// Ambient; it clears on its own.
    Waiting,
}

/// What the action button does, as a stable kind a shell switches on rather
/// than matching the button's words or the label. Present exactly when
/// `action` is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthActionKind {
    /// Sign in again.
    Reconnect,
    /// Show the privacy scan's first-use notice so it can be acknowledged.
    PrivacyScanNotice,
    /// Open the queue (Swift's `reviewsQueue`).
    ReviewQueue,
}

/// The health banner's words for one condition: a title, the sentence that
/// states what is held and the data consequence, the action button's label
/// where a condition has a real recovery step, and how the banner sits.
/// `action` is `None` for every condition that clears on its own -- a button
/// beside a banner that cannot change what it sits next to teaches a
/// contributor the buttons in this app do nothing.
///
/// `severity` cannot be rebuilt from `action`: an OpenCode export of the
/// wrong version is actionable (choose another folder) but has no button.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HealthLineCopy {
    pub title: String,
    pub detail: String,
    pub action: Option<String>,
    pub action_kind: Option<HealthActionKind>,
    pub severity: HealthSeverity,
}

impl HealthLineCopy {
    fn waiting(title: &str, detail: &str) -> Self {
        Self {
            title: title.to_string(),
            detail: detail.to_string(),
            action: None,
            action_kind: None,
            severity: HealthSeverity::Waiting,
        }
    }

    fn actionable(title: &str, detail: &str, action: Option<(&str, HealthActionKind)>) -> Self {
        Self {
            title: title.to_string(),
            detail: detail.to_string(),
            action: action.map(|(words, _)| words.to_string()),
            action_kind: action.map(|(_, kind)| kind),
            severity: HealthSeverity::Actionable,
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
///
/// The queue is persisted (`daemon-queue.jsonl`), and the daemon's first
/// poll after it starts is a full pass that re-lists every watched source
/// (`daemon::watcher::tick`; tokio's interval fires its first tick at once),
/// so sessions written while it was down are found then. That is what lets
/// the detail promise both. `actionable`, also draft: nothing in the core
/// restarts a stopped service, so this does not clear on its own.
pub fn core_down_copy() -> HealthLineCopy {
    HealthLineCopy::actionable(
        "Can't reach Trace Commons' background service.",
        "Nothing is being watched or sent while it's down. Your queue and the sessions \
         already sent are safe, and sessions from while it was down will be picked up when \
         it's running again.",
        None,
    )
}

/// The generic banner: a real condition this build has no sentence for.
pub fn on_hold_copy() -> HealthLineCopy {
    HealthLineCopy::waiting(
        "Contributions are on hold.",
        "Something is stopping traces from being sent. Nothing has been lost, and nothing has \
         gone out.",
    )
}

/// The banner for one `last_error_label`, word for word from the macOS and
/// Windows shells' shared table. An unrecognised label is still a real
/// condition -- the daemon is free to grow labels this build has never heard
/// of -- so it gets [`on_hold_copy`], the same fallback both shells already
/// draw, rather than inventing a cause or rendering the raw label, which
/// would break the never-name-the-mechanism rule by the most direct route
/// there is.
///
/// `max_queue_entries` is the daemon's configured queue limit
/// (`get_settings.max_queue_entries`), used only by `queue-full`'s count.
/// `None` when the caller does not know it; the sentence then names no
/// number rather than a default that may be wrong.
pub fn health_copy_for_label(label: &str, max_queue_entries: Option<u64>) -> HealthLineCopy {
    use crate::daemon::health;

    match label {
        health::LABEL_NOT_LOGGED_IN => HealthLineCopy::actionable(
            "Not connected.",
            "Sessions are being queued, but nothing can be sent until you reconnect. Nothing \
             has been lost.",
            Some(("Reconnect", HealthActionKind::Reconnect)),
        ),
        health::LABEL_NEAR_AI_NOTICE_PENDING => {
            // The recovery prompt's words are the core's own
            // (`privacy_scan_copy`), the same ones onboarding's scan screen
            // reads; this delegates rather than repeating them so the two
            // surfaces cannot drift apart.
            let copy = crate::privacy_scan_copy::privacy_scan_copy();
            HealthLineCopy::actionable(
                copy.recovery_title,
                copy.recovery_detail,
                Some((copy.recovery_action, HealthActionKind::PrivacyScanNotice)),
            )
        }
        health::LABEL_CANARY_FAILED => HealthLineCopy::waiting(
            "The privacy scan failed its own self-test,",
            "so nothing is being sent through it. This is deliberate -- a scan we can't verify \
             doesn't get used.",
        ),
        health::LABEL_PII_FILTER_UNAVAILABLE => HealthLineCopy::waiting(
            "The extra privacy scan isn't reachable.",
            "Your traces are waiting rather than going out unscanned. Retrying automatically.",
        ),
        health::LABEL_CLAIM_MINT_FAILED | health::LABEL_INGEST_UNREACHABLE => {
            HealthLineCopy::waiting(
                "Can't reach Trace Commons right now.",
                "Your queue is safe; it'll retry on its own.",
            )
        }
        health::LABEL_OPENCODE_EXPORT_VERSION_UNSUPPORTED => {
            // Delegated, not repeated: the same words the source settings
            // screen shows for it. Actionable (choose another exports
            // folder) with no button, as in Swift's table.
            let copy = crate::source_copy::source_settings_copy();
            HealthLineCopy::actionable(
                copy.opencode_version_title,
                copy.opencode_version_detail,
                None,
            )
        }
        health::LABEL_QUEUE_FULL => {
            let detail = match max_queue_entries {
                Some(max) => format!(
                    "-- {} are already waiting. Review or clear some to start again.",
                    format_count(max)
                ),
                None => "-- the queue is full. Review or clear some to start again.".to_string(),
            };
            HealthLineCopy::actionable(
                "Trace Commons has stopped queuing new sessions",
                &detail,
                Some(("Review", HealthActionKind::ReviewQueue)),
            )
        }
        health::LABEL_DAILY_CAP_REACHED => HealthLineCopy::waiting(
            DAILY_BUDGET_TITLE,
            "Approved traces are waiting. Nothing has been lost -- they go out when the limit \
             resets.",
        ),
        _ => on_hold_copy(),
    }
}

/// A count with thousands separators, `1,000` rather than `1000`.
fn format_count(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::health::{self, ALL_LABELS};

    /// Renamed from `..._is_marked_draft`: no property of the value says
    /// "draft" (the marking is the doc comment), so the test checks what it
    /// can -- that the detail states the data consequence.
    #[test]
    fn core_down_says_the_queue_is_safe_and_new_sessions_are_picked_up() {
        let copy = core_down_copy();
        assert_eq!(copy.title, "Can't reach Trace Commons' background service.");
        assert!(copy.detail.contains("Your queue"), "{}", copy.detail);
        assert!(
            copy.detail.contains("already sent are safe"),
            "{}",
            copy.detail
        );
        assert!(copy.detail.contains("picked up"), "{}", copy.detail);
        assert!(
            !copy.detail.contains("nothing new will be queued"),
            "must not imply new sessions are lost"
        );
        assert!(copy.action.is_none());
        assert!(copy.action_kind.is_none());
    }

    #[test]
    fn every_known_label_has_words_and_names_no_mechanism() {
        for label in ALL_LABELS {
            let copy = health_copy_for_label(label, Some(500));
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
            assert_eq!(
                copy.action.is_some(),
                copy.action_kind.is_some(),
                "{label}: an action and its kind come together"
            );
        }
    }

    #[test]
    fn an_unknown_label_gets_the_generic_on_hold_sentence_not_the_raw_label() {
        let copy = health_copy_for_label("a-future-label-this-build-has-never-heard-of", None);
        assert_eq!(copy, on_hold_copy());
        assert_eq!(copy.title, "Contributions are on hold.");
        assert!(!copy.detail.contains("a-future-label"));
    }

    #[test]
    fn only_labels_with_a_real_recovery_step_carry_an_action() {
        for (label, kind) in [
            (health::LABEL_NOT_LOGGED_IN, HealthActionKind::Reconnect),
            (
                health::LABEL_NEAR_AI_NOTICE_PENDING,
                HealthActionKind::PrivacyScanNotice,
            ),
            (health::LABEL_QUEUE_FULL, HealthActionKind::ReviewQueue),
        ] {
            let copy = health_copy_for_label(label, None);
            assert!(
                copy.action.is_some(),
                "{label} should offer a recovery action"
            );
            assert_eq!(copy.action_kind, Some(kind), "{label}");
        }
        for label in [
            health::LABEL_CANARY_FAILED,
            health::LABEL_PII_FILTER_UNAVAILABLE,
            health::LABEL_CLAIM_MINT_FAILED,
            health::LABEL_INGEST_UNREACHABLE,
            health::LABEL_DAILY_CAP_REACHED,
            health::LABEL_OPENCODE_EXPORT_VERSION_UNSUPPORTED,
        ] {
            assert!(
                health_copy_for_label(label, None).action.is_none(),
                "{label} has no button and must not offer a dead one"
            );
        }
    }

    /// Swift's `HealthCopy.forLabel` severities, label for label.
    #[test]
    fn severity_matches_the_swift_table() {
        use HealthSeverity::{Actionable, Waiting};
        for (label, severity) in [
            (health::LABEL_NOT_LOGGED_IN, Actionable),
            (health::LABEL_NEAR_AI_NOTICE_PENDING, Actionable),
            (health::LABEL_CANARY_FAILED, Waiting),
            (health::LABEL_PII_FILTER_UNAVAILABLE, Waiting),
            (health::LABEL_CLAIM_MINT_FAILED, Waiting),
            (health::LABEL_INGEST_UNREACHABLE, Waiting),
            (
                health::LABEL_OPENCODE_EXPORT_VERSION_UNSUPPORTED,
                Actionable,
            ),
            (health::LABEL_QUEUE_FULL, Actionable),
            (health::LABEL_DAILY_CAP_REACHED, Waiting),
            ("a-future-label", Waiting),
        ] {
            assert_eq!(
                health_copy_for_label(label, None).severity,
                severity,
                "{label}"
            );
        }
    }

    #[test]
    fn opencode_version_delegates_to_the_source_settings_words() {
        let copy = health_copy_for_label(health::LABEL_OPENCODE_EXPORT_VERSION_UNSUPPORTED, None);
        assert_eq!(copy.title, crate::source_copy::OPENCODE_VERSION_TITLE);
        assert_eq!(copy.detail, crate::source_copy::OPENCODE_VERSION_DETAIL);
        assert_ne!(copy, on_hold_copy());
    }

    #[test]
    fn queue_full_counts_from_the_configured_limit() {
        let detail = |max| health_copy_for_label(health::LABEL_QUEUE_FULL, max).detail;
        assert_eq!(
            detail(Some(500)),
            "-- 500 are already waiting. Review or clear some to start again."
        );
        assert!(detail(Some(2000)).contains("2,000 are already waiting"));
        assert!(detail(Some(1_234_567)).contains("1,234,567"));
        let unknown = detail(None);
        assert!(!unknown.chars().any(|c| c.is_ascii_digit()), "{unknown}");
    }

    #[test]
    fn the_near_ai_recovery_matches_the_privacy_scan_copy_source() {
        let copy = health_copy_for_label(health::LABEL_NEAR_AI_NOTICE_PENDING, None);
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
        assert!(value["action_kind"].is_null());
        assert_eq!(value["severity"], "actionable");
        let queue_full =
            serde_json::to_value(health_copy_for_label(health::LABEL_QUEUE_FULL, None)).unwrap();
        assert_eq!(queue_full["action_kind"], "review_queue");
        assert_eq!(queue_full["severity"], "actionable");
    }
}
