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
/// The confirmation shown before a project is armed **from now**, which is
/// what `set_project_mode` `auto_upload` does by default (K5): new sessions
/// go without asking, and sessions already on this Mac keep waiting for the
/// contributor to pick them.
///
/// **DRAFT, NEEDS APPROVAL.** The first paragraph changed with the from-now
/// default; the second and third are the agreed text unchanged. It still
/// opens "Sessions from this project will be scrubbed", which
/// `arming_wording::project_arming_claim` depends on.
///
/// Each sentence is held to what the daemon does, because this is the only
/// thing a contributor reads before sessions start leaving without a look.
/// Before the default changed it had to say "including any already waiting",
/// because arming then approved every settled pending entry; that sentence
/// now belongs to [`ARMING_BODY_WITH_BACKLOG`] alone. It also used to say
/// three things that were not true:
///
/// - "Every future session" -- at the time, arming also sent sessions
///   already waiting.
/// - "A session is sent a day after you last work on it, so there is time to
///   change your mind" -- a waiting session already quiet for a day goes out
///   at once, and the settle window is not a control (`ARMED_SETTLE_SECS`).
///   What is true, and stated instead, is that nothing goes before it.
/// - "You can turn this off at any time", with nothing said about what that
///   does -- turning automatic off left every session it had approved still
///   uploading. It now returns them to waiting, and the sentence says so.
pub const ARMING_BODY: &str = "Sessions from this project will be scrubbed and contributed \
     without asking you, from now on. You won't review them first. Sessions already on this Mac \
     keep waiting for you to pick them.\n\nNo session is sent until it has been quiet for a \
     day.\n\nYou can turn this off at any time. Anything it hasn't sent yet goes back to waiting \
     for you, and anything already sent stays sent.";

/// The confirmation for arming **with the backlog**
/// (`set_project_mode` `include_backlog: true`): the agreed wording from
/// before the from-now default, which is still exactly what that call does.
pub const ARMING_BODY_WITH_BACKLOG: &str = "Sessions from this project will be scrubbed and \
     contributed without asking you, including any already waiting. You won't review them \
     first.\n\nNo session is sent until it has been quiet for a day.\n\nYou can turn this off \
     at any time. Anything it hasn't sent yet goes back to waiting for you, and anything already \
     sent stays sent.";

/// Customize's words for the from-now rule, the past-session picker, and
/// "Keep on this Mac" (K5), in one table so no shell writes its own.
///
/// **DRAFT, NEEDS APPROVAL**, every sentence. Each is held to what the daemon
/// does: see "Arming from now" and "`keep`: Keep on this Mac" in
/// `docs/contributor-daemon-ipc-v1_1.md`.
#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub struct CustomizeCopy {
    /// The rule's one-line description beside "Share automatically".
    pub share_automatically_rule: &'static str,
    /// What happens to the folder's backlog under that rule.
    pub backlog_rule: &'static str,
    /// The picker's heading.
    pub picker_heading: &'static str,
    /// The picker's explainer, above the folders.
    pub picker_explainer: &'static str,
    /// The picker's action.
    pub picker_include: &'static str,
    /// The sheet's button.
    pub keep: &'static str,
    /// Shown once a session is kept.
    pub kept_confirmation: &'static str,
    /// The heading over the kept list.
    pub kept_heading: &'static str,
    /// The undo on a kept row.
    pub undo_keep: &'static str,
    /// Shown when an undo is refused because the queue is full.
    pub undo_keep_queue_full: &'static str,
    /// Shown when an undo is refused because the folder is set to Never.
    pub undo_keep_folder_never: &'static str,
}

/// The one table of Customize copy. See [`CustomizeCopy`].
#[must_use]
pub fn customize_copy() -> CustomizeCopy {
    CustomizeCopy {
        share_automatically_rule: "New sessions from this folder are contributed without asking you.",
        backlog_rule: "Sessions already on this Mac keep waiting until you pick them below.",
        picker_heading: "Past sessions, by folder",
        picker_explainer: "Tick the sessions already on this Mac that you want to include. \
             Anything you leave unticked keeps waiting for you.",
        picker_include: "Include selected",
        keep: "Keep on this Mac",
        kept_confirmation: "Kept on this Mac. It won't be sent, and it won't expire. You can \
             offer it again later.",
        kept_heading: "Kept on this Mac",
        undo_keep: "Offer it again",
        undo_keep_queue_full: "Too many sessions are waiting. Decide some of them first, then \
             offer this one again.",
        undo_keep_folder_never: "This folder is set to Never. Change its rule first, then offer \
             this session again.",
    }
}

#[cfg(test)]
mod arming_body_tests {
    use super::{ARMING_BODY, ARMING_BODY_WITH_BACKLOG, customize_copy};

    /// Verbatim, because the macOS shell holds its own copy and pins it to
    /// this one; a change here is a change there.
    #[test]
    fn the_arming_body_is_exactly_what_was_agreed() {
        assert_eq!(
            ARMING_BODY,
            "Sessions from this project will be scrubbed and contributed without asking you, from now on. You won't review them first. Sessions already on this Mac keep waiting for you to pick them.\n\nNo session is sent until it has been quiet for a day.\n\nYou can turn this off at any time. Anything it hasn't sent yet goes back to waiting for you, and anything already sent stays sent."
        );
        assert_eq!(
            ARMING_BODY_WITH_BACKLOG,
            "Sessions from this project will be scrubbed and contributed without asking you, including any already waiting. You won't review them first.\n\nNo session is sent until it has been quiet for a day.\n\nYou can turn this off at any time. Anything it hasn't sent yet goes back to waiting for you, and anything already sent stays sent."
        );
    }

    /// The from-now default must not claim the backlog goes, and the
    /// backlog variant must say it does (K5).
    #[test]
    fn each_arming_body_says_what_happens_to_the_backlog() {
        assert!(!ARMING_BODY.contains("already waiting"));
        assert!(ARMING_BODY.contains("keep waiting for you"));
        assert!(ARMING_BODY_WITH_BACKLOG.contains("including any already waiting"));
    }

    /// The Customize table says what the daemon does: a kept session is
    /// neither sent nor expired, and the backlog waits under the rule.
    #[test]
    fn the_customize_copy_matches_the_keep_and_from_now_semantics() {
        let c = customize_copy();
        assert!(c.kept_confirmation.contains("won't be sent"));
        assert!(c.kept_confirmation.contains("won't expire"));
        assert!(c.backlog_rule.contains("keep waiting"));
        assert!(!c.share_automatically_rule.contains("already"));
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
        assert!(ARMING_BODY_WITH_BACKLOG.contains("including any already waiting"));
        assert!(ARMING_BODY.contains("goes back to waiting"));
        assert!(ARMING_BODY_WITH_BACKLOG.contains("goes back to waiting"));
    }
}
