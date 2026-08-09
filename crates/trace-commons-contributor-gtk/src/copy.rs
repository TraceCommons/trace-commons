//! The words, in one place.
//!
//! The shared design specifies copy rather than suggesting it, so it lives
//! here as constants instead of being scattered through widget
//! construction: a sentence that must not drift is easier to keep from
//! drifting when there is exactly one of it.
//!
//! Four rules bind everything below.
//!
//! * **Credit is a record, never a currency.** No currency symbol, no fiat
//!   estimate, no projection, no date, no gamification.
//! * **Quarantine is held, never rejected**, and never carries a turnaround
//!   time.
//! * **Never name the mechanism.** "Privacy filter", "claim", "ingest",
//!   "canary" are internal words.
//! * **Always state the data consequence.** "Nothing was sent unscanned",
//!   "your queue is safe", "nothing has been lost".

pub const APP_NAME: &str = "Trace Commons";

// --- Queue -------------------------------------------------------------

pub const RESIDUAL_RISK: &str =
    "Scrubbing is pattern-based. It misses things it hasn't seen before.";
pub const LOOK_INSIDE: &str = "Look inside";
pub const NOT_THIS_ONE: &str = "Not this one";
pub const NOT_THIS_ONE_TOOLTIP: &str =
    "Skips this session only. This project will keep being offered.";
pub const QUEUE_EMPTY_TITLE: &str = "Nothing waiting";
pub const QUEUE_EMPTY_BODY: &str = "When a session finishes and goes quiet, it shows up here. \
     Nothing is sent unless you say so.";
pub const CHECKING: &str = "Checking what would be sent…";

// --- Preview -----------------------------------------------------------

pub const TAB_SEARCH: &str = "Search";
pub const TAB_WHATS_IN_IT: &str = "What's in it";
pub const TAB_WOULD_BE_SENT: &str = "Exactly what would be sent";
pub const TAB_PERMISSIONS: &str = "Permissions";
pub const SEARCH_PROMPT: &str = "Search this trace for anything you need to be sure isn't in it.";
pub const CONTRIBUTE: &str = "Contribute";

/// Shown where the transcript would be when the shell is attached to a
/// daemon it does not host. The contract serves the full redacted body
/// in-process only; saying so plainly beats an empty box.
pub const BODY_NOT_AVAILABLE_HERE: &str = "The full text can only be shown by the copy of Trace Commons that is doing the watching. \
     A background watcher is running separately on this machine, so this window can show what \
     would be sent and what was scrubbed, but not the text itself. \
     `trace-commons-contributor daemon preview` shows the same summary from a terminal.";

pub const PERMISSIONS_INTRO: &str =
    "If you contribute this session, it will carry these permissions:";
pub const PERMISSIONS_REQUESTED_NOTE: &str = "These are the permissions this device requests. Trace Commons can narrow them, never widen them.";
pub const UNENROLLED_PREVIEW: &str = "This is an illustration. This device isn't connected yet, so this was built without your \
     identity and nothing here can be contributed.";

// --- Approving ---------------------------------------------------------

pub const SENDING: &str = "Sending…";
pub const UNDO: &str = "Undo";
/// Used when the daemon reports no hold, so no undo may be offered.
pub const APPROVED_NO_UNDO: &str = "Approved. It goes out on the next pass.";

// --- Credit ------------------------------------------------------------

pub const CREDIT_HEADING: &str = "About credit";
pub const CREDIT_BODY: &str = "Contributions earn credit points, scored on how novel and \
     information-rich a trace is. Today credit is a record, not a currency: there is no payout, \
     no token, no exchange rate, and no date. The intent is that credit eventually settles to \
     something real, and if it does it will settle from this record. Contribute because you want \
     the commons to exist.";
pub const NOT_SYNCED_YET: &str = "Not synced yet";

// --- History -----------------------------------------------------------

pub const HISTORY_IN_THE_COMMONS: &str = "In the commons";
pub const HISTORY_BEING_REVIEWED: &str = "Being reviewed for privacy";
pub const HISTORY_WAITING_TO_BE_SCORED: &str = "Waiting to be scored";
pub const QUARANTINE_HEADING: &str = "Held for privacy review";
pub const QUARANTINE_BODY: &str = "A person at Trace Commons reads these before they enter the \
     commons. It happens when automated checks see something that might be personal or sensitive \
     and can't decide on its own.\n\nThese have not been rejected, and they have not been shared \
     with anyone but the reviewer. They are sitting still.\n\nTypical wait: we don't have a \
     reliable number yet.";

// --- Arming ------------------------------------------------------------

pub fn arming_heading(project_label: &str) -> String {
    format!("Contribute from {project_label} automatically?")
}
pub const ARMING_BODY: &str = "Every future session in this project will be scrubbed and \
     contributed without asking you. You won't review them first.\n\nYou can turn this off at any \
     time.";
pub const ARMING_CANCEL: &str = "Not now";
pub const ARMING_CONFIRM: &str = "Turn on automatic contributing";

// --- Quitting ----------------------------------------------------------

/// The Linux wording, and it is the *second* of the two the shared spec
/// gives. It is true only where a separate daemon keeps running after the
/// window closes; where this application is itself the watcher, the first
/// wording applies. Which one is shown is decided at runtime by which of
/// those two this process actually is -- getting it wrong is a lie about
/// whether the machine is still watching. See `QUIT_HOSTING_BODY`.
pub const QUIT_ATTACHED_BODY: &str = "The background watcher keeps running and will keep queuing \
     sessions. Nothing will be sent while nobody's approving.";
pub const QUIT_ATTACHED_CONFIRM: &str = "Quit";
pub const QUIT_ATTACHED_ALSO_STOP: &str = "Quit and stop watching";

pub const QUIT_HOSTING_BODY: &str = "Quitting stops Trace Commons watching for finished sessions. \
     Nothing is queued or sent until you open it again. Anything already waiting stays waiting.";
pub const QUIT_HOSTING_CANCEL: &str = "Cancel";
pub const QUIT_HOSTING_CONFIRM: &str = "Quit";

// --- Notifications -----------------------------------------------------

pub const NOTIFY_REVIEW: &str = "Review";
pub const NOTIFY_NOT_NOW: &str = "Not now";
pub const NOTIFY_NOTHING_SENT: &str = "Nothing is sent until you review them.";

// --- Health ------------------------------------------------------------

/// The sentence to render for a `status.health.last_error_label`.
///
/// The daemon picks exactly one label by its own precedence order; a client
/// must not reconstruct that order or choose a different label to show. So
/// this is a lookup, not a decision.
pub fn health_sentence(label: &str) -> &'static str {
    match label {
        "not-logged-in" => {
            "Not connected. Sessions are being queued, but nothing can be sent until you \
             reconnect. Nothing has been lost."
        }
        "pii-filter-unavailable" => {
            "The extra privacy scan isn't reachable. Your traces are waiting rather than going \
             out unscanned. Retrying automatically."
        }
        "privacy-filter-canary-failed" => {
            "The privacy scan failed its own self-test, so nothing is being sent through it. \
             This is deliberate -- a scan we can't verify doesn't get used."
        }
        "near-ai-notice-not-acknowledged" => {
            "One thing to confirm. You chose the extra privacy scan, which sends message text to \
             NEAR AI. Confirm you're OK with that and contributions resume."
        }
        "claim-mint-failed" | "ingest-unreachable" => {
            "Can't reach Trace Commons right now. Your queue is safe; it'll retry on its own."
        }
        "daily-cap-reached" => "Daily limit reached. The rest goes out tomorrow.",
        "queue-full" => {
            "Trace Commons has stopped queuing new sessions -- 500 are already waiting. Review or \
             clear some to start again."
        }
        // An unrecognized label is still a real condition. Say the true
        // thing that holds for every blocking label rather than inventing a
        // mechanism name for it.
        _ => "Something is holding contributions up. Your queue is safe; nothing has been lost.",
    }
}

/// Whether a health label deserves an action button, and what it says.
pub fn health_action(label: &str) -> Option<&'static str> {
    match label {
        "not-logged-in" => Some("Reconnect"),
        "near-ai-notice-not-acknowledged" => Some("Review and confirm"),
        _ => None,
    }
}

/// Plain-language renderings of `reason_label`, for entries that are on the
/// queue but are not decisions owed.
pub fn reason_sentence(label: &str) -> &'static str {
    match label {
        "dismissed-by-contributor" => "You skipped this one.",
        "expired-without-decision" => "Dropped without a decision. Dropped means never sent.",
        "session-changed-after-offer" => {
            "The session changed after it was offered, so nothing was sent. It is being offered \
             again."
        }
        "consent-scopes-changed-after-approval" => {
            "Your permissions changed after you approved this, so nothing was sent. It is being \
             offered again."
        }
        "approval-inputs-changed" | "envelope-changed-after-approval" => {
            "What would be sent is not what you were shown, so nothing was sent. It is being \
             offered again."
        }
        _ => "Nothing was sent.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_health_sentence_names_an_internal_mechanism() {
        // "privacy filter", "claim", "ingest" and "canary" are internal
        // words; the contributor-facing sentence must not use them even
        // though the labels themselves do.
        for label in [
            "not-logged-in",
            "pii-filter-unavailable",
            "privacy-filter-canary-failed",
            "near-ai-notice-not-acknowledged",
            "claim-mint-failed",
            "ingest-unreachable",
            "daily-cap-reached",
            "queue-full",
            "something-nobody-has-written-yet",
        ] {
            let sentence = health_sentence(label).to_lowercase();
            for forbidden in ["privacy filter", "canary self", "claim", "ingest", "pii"] {
                assert!(
                    !sentence.contains(forbidden),
                    "{label} names the mechanism: {sentence}"
                );
            }
        }
    }

    #[test]
    fn credit_copy_carries_no_currency_projection_or_date() {
        for forbidden in ["$", "USD", "worth", "value of", "by 20", "payout of"] {
            assert!(
                !CREDIT_BODY.contains(forbidden),
                "credit copy must not imply a currency: {forbidden}"
            );
        }
    }

    #[test]
    fn quarantine_copy_never_says_rejected_and_never_promises_a_wait() {
        let text = format!("{QUARANTINE_HEADING} {QUARANTINE_BODY}").to_lowercase();
        // The word appears exactly once, and only in the sentence denying
        // it. Any other use is the reading this copy exists to prevent.
        assert_eq!(text.matches("rejected").count(), 1);
        assert!(text.contains("have not been rejected"));
        for forbidden in [
            "48 hours",
            "business days",
            "within a week",
            "usually takes",
        ] {
            assert!(
                !text.contains(forbidden),
                "no turnaround time may be stated"
            );
        }
    }
}
