//! Shared contributor-facing history copy.
//!
//! The one table of contribution-status labels every shell's History reads
//! ([`STATUS_LABELS`], looked up through [`status_label`]). A shell does not
//! type its own word for a status: GTK reads this module directly, and the
//! others read the same table across the C ABI -- macOS from
//! `tc_public_run_copy` (`history_status_labels`), Windows and Tauri from
//! `tc_contributor_disclosure_copy_json` (`history_ui.status_labels`).

pub const HELD_ROW_BODY: &str = "Automated checks saw something that might be personal and \
     couldn't decide on their own. It has not been rejected, and it has not been shared with \
     anyone but the agent that inspects it.";

/// The label on a contribution whose status this build does not recognise.
///
/// Every shell shows this, and only this, for a status it has no label for:
/// it is never "Waiting to be scored", "Not in the commons" or the raw wire
/// token, each of which asserts a state the shell does not know. An
/// unrecognised status is also treated as terminal, so no shell offers
/// Withdraw on it.
///
/// Carried to the shells by `public_run_copy().contribution_status_unavailable`
/// (`tc_public_run_copy`) and by the disclosure bundle's
/// `history_ui.status_unavailable` (`tc_contributor_disclosure_copy_json`).
pub const STATUS_UNAVAILABLE: &str = "Status unavailable";

/// `submitted` (and the versioned pipeline's `processing`): uploaded, not
/// yet scored. Never "Submitted", which reads as done. Also
/// `preview_copy::monitor_screens_copy().history_submitted`, which is this
/// constant, not a second spelling of it.
pub const WAITING_TO_BE_SCORED: &str = "Waiting to be scored";

/// `accepted`.
pub const IN_THE_COMMONS: &str = "In the commons";

/// `quarantined`: held, never rejected. Also
/// `preview_copy::monitor_screens_copy().held_for_review` and the public-run
/// table's `quarantined` label.
pub const HELD_FOR_PRIVACY_REVIEW: &str = "Held for privacy review";

/// `withdrawn`: taken back from this machine (the daemon's own stamp).
pub const WITHDRAWN_BY_YOU: &str = "Withdrawn by you";

/// `received`. Also the public-run table's label.
pub const RECEIVED: &str = "Received";

/// `awaiting_pii_backstop`: a pre-acceptance hold. Also the public-run
/// table's label.
pub const WAITING_FOR_PRIVACY_REVIEW: &str = "Waiting for privacy review";

/// `rejected`. Also the public-run table's label.
pub const REJECTED: &str = "Rejected";

/// `revoked`: the server's word for a withdrawal this machine did not make
/// (on the web, or by a revocation). Not "by you": a revocation need not
/// have been the contributor's. Also the public-run table's label.
pub const WITHDRAWN: &str = "Withdrawn";

/// `expired`. Also the public-run table's label.
pub const EXPIRED: &str = "Expired";

/// `purged`. Also the public-run table's label.
pub const PURGED: &str = "Purged";

/// One row of [`STATUS_LABELS`]: a wire status and History's word for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct StatusLabel {
    pub status: &'static str,
    pub label: &'static str,
}

/// Every contribution status the daemon or the server can report, and the
/// word History shows for it.
///
/// - `submitted`, `withdrawn`: the daemon's own stamps
///   (`daemon::history::STATUS_SUBMITTED`, `STATUS_WITHDRAWN`).
/// - `processing`: the versioned pipeline's upload receipt
///   (`daemon::history::STATUS_PROCESSING`). The daemon reads it as
///   `submitted` -- uploaded, no verdict yet (#1169) -- and so does History.
/// - `received`, `accepted`, `quarantined`, `awaiting_pii_backstop`,
///   `rejected`, `revoked`, `expired`, `purged`: the server's
///   submission-status read-back (`TraceCorpusStatus` in
///   `trace-commons-server`).
///
/// A status not in this table is [`STATUS_UNAVAILABLE`], and terminal.
pub const STATUS_LABELS: [StatusLabel; 11] = [
    StatusLabel {
        status: "submitted",
        label: WAITING_TO_BE_SCORED,
    },
    StatusLabel {
        status: "processing",
        label: WAITING_TO_BE_SCORED,
    },
    StatusLabel {
        status: "received",
        label: RECEIVED,
    },
    StatusLabel {
        status: "accepted",
        label: IN_THE_COMMONS,
    },
    StatusLabel {
        status: "quarantined",
        label: HELD_FOR_PRIVACY_REVIEW,
    },
    StatusLabel {
        status: "awaiting_pii_backstop",
        label: WAITING_FOR_PRIVACY_REVIEW,
    },
    StatusLabel {
        status: "rejected",
        label: REJECTED,
    },
    StatusLabel {
        status: "revoked",
        label: WITHDRAWN,
    },
    StatusLabel {
        status: "withdrawn",
        label: WITHDRAWN_BY_YOU,
    },
    StatusLabel {
        status: "expired",
        label: EXPIRED,
    },
    StatusLabel {
        status: "purged",
        label: PURGED,
    },
];

/// History's word for a contribution status: its [`STATUS_LABELS`] entry,
/// or [`STATUS_UNAVAILABLE`] for a status this build does not recognise.
#[must_use]
pub fn status_label(status: &str) -> &'static str {
    STATUS_LABELS
        .iter()
        .find(|row| row.status == status)
        .map_or(STATUS_UNAVAILABLE, |row| row.label)
}

/// One row of [`CREDIT_OUTCOME_LABELS`]: a server reason label and the
/// short words a History row shows for it, after "no credit".
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct CreditOutcomeLabel {
    pub reason: &'static str,
    pub label: &'static str,
}

/// Why a decided trace earned no credit, one line per server reason label
/// (`trace_commons_protocol::credit_outcome`). A History row shows it
/// beside the row's "no credit" line
/// (`preview_copy::MonitorHomeHistoryCopy::row_no_credit`) where a credited
/// row shows its points. Carried to macOS by `tc_public_run_copy`
/// (`credit_outcome_labels`).
///
/// Owner-approved 2026-10-28: every label in this table and
/// [`NO_CREDIT_REASON_UNAVAILABLE`].
pub const CREDIT_OUTCOME_LABELS: [CreditOutcomeLabel; 8] = [
    CreditOutcomeLabel {
        reason: "policy_mismatch",
        label: "Needs Train consent",
    },
    CreditOutcomeLabel {
        reason: "novelty_failed",
        label: "Too similar",
    },
    CreditOutcomeLabel {
        reason: "perplexity_failed",
        label: "Too predictable",
    },
    CreditOutcomeLabel {
        reason: "duplicate",
        label: "Duplicate",
    },
    CreditOutcomeLabel {
        reason: "submission_not_accepted",
        label: "Not accepted",
    },
    CreditOutcomeLabel {
        reason: "insufficient_coverage",
        label: "Too long",
    },
    CreditOutcomeLabel {
        reason: "credit_not_issued",
        label: "Server error",
    },
    CreditOutcomeLabel {
        reason: "other",
        label: NO_CREDIT_REASON_UNAVAILABLE,
    },
];

/// The words for a reason label this build does not know, and for `other`.
/// Owner-approved 2026-10-28.
pub const NO_CREDIT_REASON_UNAVAILABLE: &str = "No reason given";

/// The words for a no-credit reason: its [`CREDIT_OUTCOME_LABELS`] entry, or
/// [`NO_CREDIT_REASON_UNAVAILABLE`] for a label this build does not know.
#[must_use]
pub fn credit_outcome_label(reason: &str) -> &'static str {
    CREDIT_OUTCOME_LABELS
        .iter()
        .find(|row| row.reason == reason)
        .map_or(NO_CREDIT_REASON_UNAVAILABLE, |row| row.label)
}

/// What a credit figure above zero and under 0.05 reads as: it rounds to
/// zero tenths, and a "0" beside "credit pending" says nothing is pending
/// when something is. DRAFT, awaiting owner approval.
pub const CREDIT_UNDER_A_TENTH: &str = "<0.1";

/// A credit figure as every shell's History and Home tiles draw it, so one
/// contribution reads the same on every machine.
///
/// - Rounded once to tenths, half away from zero (4.25 is 4.3, never 4.2),
///   in whole millionths first so a sum that should be 0.35 but reads
///   0.34999999999999998 is still 0.4: the rounding of the daemon's digest
///   sentences (`daemon::nudge::tenths_of`). The digest keeps its own
///   one-decimal form ("6.0 credit pending."), the design's wording.
/// - A whole figure is whole ("0", "48"), anything else has one decimal
///   ("12.5"): "Pending 0.0" and "48.0" beside a "48" are what this
///   replaces.
/// - Above zero and under 0.05 is [`CREDIT_UNDER_A_TENTH`], never "0".
/// - No thousands separator ("1234.5"), the same as every count beside it.
/// - Credit is never negative; a figure below zero that rounds to zero is
///   "0", never "-0".
/// - Not finite is a dash, the sign every shell uses for a figure it was
///   not given.
#[must_use]
pub fn credit_figure(value: f64) -> String {
    if !value.is_finite() {
        return "\u{2014}".to_string();
    }
    let millionths = (value * 1e6).round();
    // Past what an i64 of tenths holds is not a credit figure anybody has;
    // say so rather than print a saturated number.
    if millionths.abs() >= 9.0e18 {
        return "\u{2014}".to_string();
    }
    let millionths = millionths as i64;
    let magnitude = (millionths.unsigned_abs() + 50_000) / 100_000;
    if magnitude == 0 {
        return if millionths > 0 {
            CREDIT_UNDER_A_TENTH.to_string()
        } else {
            "0".to_string()
        };
    }
    let sign = if millionths < 0 { "-" } else { "" };
    if magnitude.is_multiple_of(10) {
        format!("{sign}{}", magnitude / 10)
    } else {
        format!("{sign}{}.{}", magnitude / 10, magnitude % 10)
    }
}

#[cfg(test)]
mod tests {

    /// The cases the review ran through the old Swift formatter, and what
    /// each now reads on every shell.
    #[test]
    fn credit_figure_rounds_half_away_and_never_reads_zero_for_some() {
        for (value, want) in [
            (0.0, "0"),
            (48.0, "48"),
            (12.5, "12.5"),
            (0.05, "0.1"),
            (0.04, "<0.1"),
            (0.000_001, "<0.1"),
            (-0.04, "0"),
            (4.25, "4.3"),
            (12.25, "12.3"),
            (0.35, "0.4"),
            // The float just below 0.35, as a sum can land.
            (f64::from_bits(0.35_f64.to_bits() - 1), "0.4"),
            (1234.5, "1234.5"),
            (1234.0, "1234"),
            (9.96, "10"),
            (-1.25, "-1.3"),
        ] {
            assert_eq!(credit_figure(value), want, "{value}");
        }
    }

    /// The f32 the rollup carries, as the decimal the server wrote: 0.35f32
    /// widened is 0.3499999940..., which the millionths step still reads as
    /// 0.35.
    #[test]
    fn credit_figure_reads_a_widened_f32_as_its_decimal() {
        assert_eq!(credit_figure(f64::from(0.35_f32)), "0.4");
        assert_eq!(credit_figure(f64::from(4.25_f32)), "4.3");
    }

    #[test]
    fn credit_figure_is_a_dash_for_no_number() {
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 1e300] {
            assert_eq!(credit_figure(value), "\u{2014}", "{value}");
        }
    }

    use super::*;

    /// Every reason the server may send has words of its own, in the
    /// protocol's order, and nothing else does.
    #[test]
    fn every_credit_outcome_reason_has_a_label() {
        let reasons: Vec<&str> = CREDIT_OUTCOME_LABELS.iter().map(|row| row.reason).collect();
        assert_eq!(
            reasons,
            trace_commons_protocol::credit_outcome::CREDIT_OUTCOME_REASONS.to_vec()
        );
        for row in CREDIT_OUTCOME_LABELS {
            assert!(!row.label.is_empty(), "{} has no words", row.reason);
            assert_eq!(credit_outcome_label(row.reason), row.label);
        }
    }

    /// A reason from a newer server reads the fallback, never the raw label,
    /// and never claims the trace is still pending.
    #[test]
    fn an_unknown_credit_outcome_reason_reads_the_fallback() {
        assert_eq!(
            credit_outcome_label("a_label_from_later"),
            NO_CREDIT_REASON_UNAVAILABLE
        );
        for row in CREDIT_OUTCOME_LABELS {
            let lower = row.label.to_lowercase();
            assert!(!lower.contains("pending"), "{} says pending", row.reason);
            assert!(!lower.contains('_'), "{} reads as a token", row.reason);
        }
    }

    /// An unrecognised status asserts nothing about where the contribution
    /// is: not waiting, not in the commons, not out of it.
    #[test]
    fn an_unrecognised_status_claims_no_state() {
        assert_eq!(STATUS_UNAVAILABLE, "Status unavailable");
        let lower = STATUS_UNAVAILABLE.to_lowercase();
        for claim in ["waiting", "scored", "commons", "withdrawn", "rejected"] {
            assert!(
                !lower.contains(claim),
                "{STATUS_UNAVAILABLE} claims {claim}"
            );
        }
    }

    /// The server's submission-status wire values, from
    /// `TraceCorpusStatus::as_str` in
    /// `crates/trace-commons-ingest/src/bin/trace-commons-ingest.rs` and
    /// `TraceCorpusStatus` in `crates/trace-commons-server/src/trace_corpus_storage.rs`.
    /// Copied, not imported: this crate is MIT/Apache and may not depend on
    /// the AGPL server, not even for a test (`license_boundary.rs`).
    const SERVER_STATUSES: [&str; 8] = [
        "received",
        "accepted",
        "quarantined",
        "awaiting_pii_backstop",
        "rejected",
        "revoked",
        "expired",
        "purged",
    ];

    /// Every status the shells switch on: the withdraw allowlist they all
    /// share (macOS `ContributionStatusPresentation.openValues`, GTK
    /// `WITHDRAWABLE_STATUSES`, Windows `WithdrawableStatuses`, Tauri
    /// `withdrawableStatuses`), the closed ones they name beside it, and the
    /// rows their status words and tones match on.
    const SHELL_STATUSES: [&str; 10] = [
        "submitted",
        "received",
        "accepted",
        "quarantined",
        "awaiting_pii_backstop",
        "rejected",
        "withdrawn",
        "revoked",
        "purged",
        "expired",
    ];

    /// Completeness: every status the daemon, the server, the public-run
    /// table or a shell knows has a label of its own, and none of them
    /// falls through to "Status unavailable".
    #[test]
    fn every_known_status_has_a_label() {
        use crate::daemon::history::{
            STATUS_ACCEPTED, STATUS_PROCESSING, STATUS_QUARANTINED, STATUS_REVOKED,
            STATUS_SUBMITTED, STATUS_WITHDRAWN,
        };
        let daemon = [
            STATUS_SUBMITTED,
            STATUS_ACCEPTED,
            STATUS_QUARANTINED,
            STATUS_WITHDRAWN,
            STATUS_REVOKED,
            // The versioned pipeline's receipt (#1169).
            STATUS_PROCESSING,
        ];
        let public_run = crate::public_run::public_run_copy()
            .contribution_status_choices
            .map(|choice| choice.value);
        for status in daemon
            .iter()
            .chain(&SERVER_STATUSES)
            .chain(&SHELL_STATUSES)
            .chain(&public_run)
        {
            let label = status_label(status);
            assert_ne!(label, STATUS_UNAVAILABLE, "{status} has no label");
            assert!(!label.is_empty(), "{status} has an empty label");
        }
    }

    #[test]
    fn the_table_names_each_status_once() {
        let mut seen = std::collections::BTreeSet::new();
        for row in STATUS_LABELS {
            assert!(seen.insert(row.status), "{} is listed twice", row.status);
        }
    }

    #[test]
    fn an_unknown_status_reads_status_unavailable() {
        for status in ["future_state", "", "Accepted", "pending"] {
            assert_eq!(status_label(status), STATUS_UNAVAILABLE, "{status:?}");
        }
    }

    /// Owner rulings: a submission reads as waiting (one string, shared with
    /// the glass monitor's table); `processing` is a submission; held is
    /// never a refusal; a withdrawal the server reports is not claimed as
    /// the contributor's own.
    #[test]
    fn the_rulings_hold() {
        let monitor = crate::preview_copy::monitor_screens_copy();
        assert_eq!(status_label("submitted"), monitor.history_submitted);
        assert_eq!(status_label("submitted"), "Waiting to be scored");
        assert_eq!(status_label("processing"), status_label("submitted"));
        assert_eq!(status_label("quarantined"), monitor.held_for_review);
        assert!(
            !status_label("quarantined")
                .to_lowercase()
                .contains("reject")
        );
        assert_eq!(status_label("accepted"), "In the commons");
        assert_eq!(status_label("withdrawn"), "Withdrawn by you");
        assert!(!status_label("revoked").contains("by you"));
    }

    /// Where the session-detail table (`public_run_copy`) and History agree
    /// on a word, it is one string, not two spellings of it.
    #[test]
    fn shared_words_are_the_public_run_tables_words() {
        let public_run = crate::public_run::public_run_copy();
        for status in [
            "received",
            "quarantined",
            "awaiting_pii_backstop",
            "rejected",
            "revoked",
            "expired",
            "purged",
        ] {
            let detail = public_run
                .contribution_status_choices
                .iter()
                .find(|choice| choice.value == status)
                .map(|choice| choice.label);
            assert_eq!(detail, Some(status_label(status)), "{status}");
        }
    }
}
