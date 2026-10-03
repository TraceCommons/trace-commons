//! Copy for decisions made from the scrubbed contribution preview.

pub const NOTHING_MATCHED: &str = "nothing matched";
pub const REDACTION_CATEGORY_LOCAL_PATH: &str = "File paths from this machine.";
pub const REDACTION_CATEGORY_SECRET: &str =
    "API keys, tokens, private keys, and high-entropy strings found next to credential words.";
pub const REDACTION_CATEGORY_PRIVACY_FILTER: &str =
    "Names, emails, and other personal details found in prose.";
pub const REDACTION_CATEGORY_SENSITIVE_FIELD: &str =
    "Fields whose name marks them sensitive, like password or authorization.";
pub const REDACTION_CATEGORY_TOOL_SENSITIVE_FIELD: &str =
    "Tool-call arguments whose name marks them sensitive.";
pub const REDACTION_CATEGORY_RESIDUAL: &str = "Found, and still in what would be sent. Either a credential inside a correction \
     you wrote, which is kept on purpose, or a field scrubbing does not reach.";
pub const REDACTION_CATEGORY_UNKNOWN: &str =
    "Removed by a pattern this version has no description for.";

pub fn redaction_row_counts(occurrences: u32, distinct: u32) -> String {
    if distinct > 0 && distinct < occurrences {
        format!("{occurrences} ({distinct} distinct)")
    } else {
        format!("{occurrences}")
    }
}

/// A detection that survived scrubbing remains in the outgoing envelope.
/// `count` measures detection sites, not the number of secret values.
pub fn residual_secret_line(count: u32, sites: &[String]) -> String {
    let head = if count == 1 {
        "A secret found here is still in what would be sent".to_string()
    } else {
        format!("Secrets found in {count} places are still in what would be sent")
    };
    if sites.is_empty() {
        return head;
    }
    format!("{head} ({})", sites.join(", "))
}

// ---------------------------------------------------------------------------
// Flow 2 states (#1118 K3): the scrub state, "worth a second look", and the
// per-line unsure hints. Worded from the design's review-sheet mock.
//
// Every sentence in this section is DRAFT, NEEDS APPROVAL.

/// **DRAFT, NEEDS APPROVAL.** The heading over a session with a
/// `second_look` reason.
pub const SECOND_LOOK_HEADING: &str = "Worth a second look";

/// **DRAFT, NEEDS APPROVAL.** A session no preview has scrubbed yet. Not
/// "0 marks": nobody has counted.
pub const NOT_YET_SCRUBBED: &str = "Not yet scrubbed";

/// **DRAFT, NEEDS APPROVAL.** The row's scrub state: `Scrubbed · 7 marks`,
/// `Scrubbed · 1 mark`, or [`NOT_YET_SCRUBBED`] for `None` (the absent
/// `marks` key). `None` is never rendered as zero.
pub fn scrub_state_line(marks: Option<u32>) -> String {
    match marks {
        None => NOT_YET_SCRUBBED.to_string(),
        Some(1) => "Scrubbed \u{00b7} 1 mark".to_string(),
        Some(n) => format!("Scrubbed \u{00b7} {n} marks"),
    }
}

/// **DRAFT, NEEDS APPROVAL.** Why one `second_look` reason waits, or `None`
/// for a label this build does not know.
pub fn second_look_line(reason: &str) -> Option<&'static str> {
    match reason {
        crate::daemon::second_look::REASON_NOTHING_MATCHED => Some(
            "No personal details matched, only file paths or nothing at all; that is why this one waits.",
        ),
        crate::daemon::second_look::REASON_LOOKS_UNSURE => Some(
            "Something here looks like an email, phone number or key that was not matched; that is why this one waits.",
        ),
        crate::daemon::second_look::REASON_TRIMMED_TO_FIT => Some(
            "Trimmed to fit the upload limit, so part of this session is not in what would be sent. That is why this one waits.",
        ),
        _ => None,
    }
}

/// The line for a `second_look` reason
/// [`second_look_line`] has no sentence for. Only reachable if a reason is
/// added without one, which `every_second_look_reason_has_its_own_line`
/// fails on; it exists so `second_look_lines` stays index-for-index with
/// `second_look` even then, rather than silently dropping a line.
pub const SECOND_LOOK_FALLBACK_LINE: &str = "This one waits for you to look before it goes.";

/// [`second_look_line`], made total: the reason's own sentence, or
/// [`SECOND_LOOK_FALLBACK_LINE`].
#[must_use]
pub fn second_look_line_or_fallback(reason: &str) -> &'static str {
    second_look_line(reason).unwrap_or(SECOND_LOOK_FALLBACK_LINE)
}

/// **DRAFT, NEEDS APPROVAL.** The hint under an unsure span, or `None` for a
/// label this build does not know.
pub fn unsure_hint_line(label: &str) -> Option<&'static str> {
    match label {
        crate::daemon::unsure_spans::LABEL_LOOKS_LIKE_EMAIL => {
            Some("Looks like an email. Not matched. Your call.")
        }
        crate::daemon::unsure_spans::LABEL_LOOKS_LIKE_PHONE => {
            Some("Looks like a phone number. Not matched. Your call.")
        }
        crate::daemon::unsure_spans::LABEL_LOOKS_LIKE_KEY => {
            Some("Looks like a key. Not matched. Your call.")
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// The glass monitor's Traces tab (#1173 R6/R7): the session inspector's row
// labels, the review's actions, the Traces badge's text equivalent, and what
// the tab says when the core does not answer. One table, so the monitor
// writes none of its own.

/// What the Traces tab says when the core does
/// not answer. The tree below it is the last one the core reported.
pub const MONITOR_CORE_UNREACHABLE: &str =
    "The watcher isn't answering. This is what it last reported.";

/// What the Traces tab says when a request the
/// core answered failed (a refused write, or a reply this build could not
/// read).
pub const MONITOR_REQUEST_FAILED: &str = "That didn't go through. Try again.";

/// Every fixed word of the monitor's Traces tab. The row labels are single
/// words; `keep` and `undo_keep` are Customize's (K5) so the two surfaces
/// cannot drift.
#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub struct MonitorTracesCopy {
    /// A session's pill, which opens its review.
    pub review: &'static str,
    /// The inspector's rows.
    pub tool: &'static str,
    pub folder: &'static str,
    pub started: &'static str,
    pub length: &'static str,
    pub prompts: &'static str,
    pub size: &'static str,
    pub sends: &'static str,
    pub marks: &'static str,
    pub unsure: &'static str,
    /// The inspector's rows for whether the session may be contributed and
    /// what the privacy witness attested; their values are the core's own
    /// state and reason lines.
    pub eligibility: &'static str,
    pub attestation: &'static str,
    /// A row's hold flag in words, so the amber flag is never colour only.
    pub held: &'static str,
    /// The marker a debug build shows over sample data.
    pub sample: &'static str,
    /// The inspector's rows for the full preview's residual-risk label and
    /// the personal-information categories it saw (categories only).
    pub residual_risk: &'static str,
    pub personal_information: &'static str,
    /// Added to the Traces badge's text equivalent when a waiting session
    /// is worth a second look (nothing matched, or trimmed to fit).
    pub second_look_waiting: &'static str,
    /// The review's actions.
    pub contribute: &'static str,
    pub keep: &'static str,
    pub dismiss: &'static str,
    /// Taking back a Contribute while it is still held.
    pub undo_contribute: &'static str,
    /// Taking back a Keep.
    pub undo_keep: &'static str,
    /// The core did not answer; see [`MONITOR_CORE_UNREACHABLE`].
    pub core_unreachable: &'static str,
    /// A request failed; see [`MONITOR_REQUEST_FAILED`].
    pub request_failed: &'static str,
}

/// The one table of the monitor's Traces words. See [`MonitorTracesCopy`].
#[must_use]
pub fn monitor_traces_copy() -> MonitorTracesCopy {
    let customize = crate::project_copy::customize_copy();
    MonitorTracesCopy {
        review: "Review",
        tool: "Tool",
        folder: "Folder",
        started: "Started",
        length: "Length",
        prompts: "Prompts",
        size: "Size",
        sends: "Sends",
        marks: "Marks",
        unsure: "Unsure",
        eligibility: "Eligibility",
        attestation: "Attestation",
        held: "Held",
        sample: "Sample",
        residual_risk: "Residual risk",
        personal_information: "Personal information",
        second_look_waiting: "some worth a second look",
        contribute: "Contribute",
        keep: customize.keep,
        dismiss: "Not this one",
        undo_contribute: "Undo",
        undo_keep: customize.undo_keep,
        core_unreachable: MONITOR_CORE_UNREACHABLE,
        request_failed: MONITOR_REQUEST_FAILED,
    }
}

/// The Traces badge's text equivalent, for assistive tech: the badge is a
/// bare number, or a dash when the count is unknown.
///
/// `None` (the daemon did not report `decisions_owed`) is "unavailable",
/// never zero. Zero is the empty string: no badge, nothing to say.
#[must_use]
pub fn decisions_owed_text(decisions_owed: Option<u64>) -> String {
    match decisions_owed {
        None => "Decision count unavailable".to_string(),
        Some(0) => String::new(),
        Some(1) => "1 decision waiting".to_string(),
        Some(n) => format!("{n} decisions waiting"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_monitor_traces_copy_is_whole_and_shares_customize_words() {
        let copy = monitor_traces_copy();
        let value = serde_json::to_value(&copy).unwrap();
        for (key, word) in value.as_object().unwrap() {
            assert!(!word.as_str().unwrap().is_empty(), "{key} is empty");
        }
        let customize = crate::project_copy::customize_copy();
        assert_eq!(copy.keep, customize.keep);
        assert_eq!(copy.undo_keep, customize.undo_keep);
        assert_eq!(copy.core_unreachable, MONITOR_CORE_UNREACHABLE);
    }

    #[test]
    fn an_unknown_decision_count_is_never_zero() {
        assert_eq!(decisions_owed_text(None), "Decision count unavailable");
        assert_eq!(decisions_owed_text(Some(0)), "");
        assert_eq!(decisions_owed_text(Some(1)), "1 decision waiting");
        assert_eq!(decisions_owed_text(Some(120)), "120 decisions waiting");
        assert_ne!(decisions_owed_text(None), decisions_owed_text(Some(0)));
    }

    #[test]
    fn every_flow2_label_has_words_and_unscrubbed_is_not_zero() {
        for reason in crate::daemon::second_look::SECOND_LOOK_REASONS {
            assert!(second_look_line(reason).is_some(), "{reason}");
        }
        for label in crate::daemon::unsure_spans::UNSURE_LABELS {
            assert!(unsure_hint_line(label).is_some(), "{label}");
        }
        assert_eq!(scrub_state_line(Some(7)), "Scrubbed \u{00b7} 7 marks");
        assert_eq!(scrub_state_line(Some(1)), "Scrubbed \u{00b7} 1 mark");
        assert_eq!(scrub_state_line(None), NOT_YET_SCRUBBED);
        assert_ne!(scrub_state_line(None), scrub_state_line(Some(0)));
    }
}
