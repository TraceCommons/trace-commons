//! Shared project controls shown beside the contribution queue.

pub const IGNORE_PROJECT: &str = "Ignore project";
pub const IGNORE_PROJECT_TOOLTIP: &str = "Stops this project being offered and clears what it has waiting. \
     Anything already submitted is unaffected, and you can undo this in Settings.";

pub fn ignore_project_title(project: &str) -> String {
    format!("Ignore {project}?")
}

pub fn ignore_project_body(pending: usize) -> String {
    let tail = "Nothing already submitted is affected. You can undo this in Settings.";
    if pending == 0 {
        return format!("Stops this project being offered. {tail}");
    }
    let noun = if pending == 1 { "trace" } else { "traces" };
    format!("This removes {pending} waiting {noun} and stops this project being offered. {tail}")
}

pub fn ignore_project_reconciled(project: &str, promised: usize, purged: u64) -> Option<String> {
    if purged == promised as u64 {
        return None;
    }
    let clause = if purged == 1 {
        "1 waiting trace was removed".to_string()
    } else {
        format!("{purged} waiting traces were removed")
    };
    Some(format!(
        "Ignored {project}. The queue changed while you were deciding: {clause}, not {promised}."
    ))
}

pub fn arming_offer_evidence(project_label: &str, count: u32) -> String {
    let times = if count == 1 {
        "once".to_string()
    } else {
        format!("{count} times")
    };
    format!("You've contributed from {project_label} {times}.")
}

pub fn arming_offer_question(project_label: &str) -> String {
    format!("Contribute from {project_label} automatically?")
}

pub const ARMING_OFFER_CONFIRM: &str = "Turn on automatic contributing";
pub const ARMING_OFFER_DECLINE: &str = "Not now";
/// The confirmation shown before a project is armed.
///
/// Each sentence is held to what the daemon does, because this is the only
/// thing a contributor reads before sessions start leaving without a look.
/// It used to say three things that were not true:
///
/// - "Every future session" -- arming also sends sessions already waiting,
///   because the watcher approves every settled pending entry of an armed
///   project on its next poll.
/// - "A session is sent a day after you last work on it, so there is time to
///   change your mind" -- a waiting session already quiet for a day goes out
///   at once, and the settle window is not a control (`ARMED_SETTLE_SECS`).
///   What is true, and stated instead, is that nothing goes before it.
/// - "You can turn this off at any time", with nothing said about what that
///   does -- turning automatic off left every session it had approved still
///   uploading. It now returns them to waiting, and the sentence says so.
pub const ARMING_BODY: &str = "Sessions from this project will be scrubbed and contributed \
     without asking you, including any already waiting. You won't review them first.\n\nNo \
     session is sent until it has been quiet for a day.\n\nYou can turn this off at any time. \
     Anything it hasn't sent yet goes back to waiting for you, and anything already sent stays \
     sent.";

#[cfg(test)]
mod arming_body_tests {
    use super::ARMING_BODY;

    /// Verbatim, because the macOS shell holds its own copy and pins it to
    /// this one; a change here is a change there.
    #[test]
    fn the_arming_body_is_exactly_what_was_agreed() {
        assert_eq!(
            ARMING_BODY,
            "Sessions from this project will be scrubbed and contributed without asking you, including any already waiting. You won't review them first.\n\nNo session is sent until it has been quiet for a day.\n\nYou can turn this off at any time. Anything it hasn't sent yet goes back to waiting for you, and anything already sent stays sent."
        );
    }

    /// The three claims review found untrue may not come back.
    #[test]
    fn the_arming_body_makes_none_of_the_retracted_claims() {
        for claim in [
            "Every future session",
            "change your mind",
            "a day after you last work",
        ] {
            assert!(!ARMING_BODY.contains(claim), "{claim:?} is not true");
        }
        assert!(ARMING_BODY.contains("including any already waiting"));
        assert!(ARMING_BODY.contains("goes back to waiting"));
    }
}
