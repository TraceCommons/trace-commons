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

#[cfg(test)]
mod tests {
    use super::*;

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
