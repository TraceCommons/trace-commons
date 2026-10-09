//! Every word of the re-engagement nudges, in one place.
//!
//! The nudge design (revision 3.1, section 6) gives the daemon the job of
//! building every title, body and action label a re-engagement surface
//! shows -- the Traces and History cards, the menu-bar panel row, the news
//! mark's accessibility and tooltip clauses, the idle-session and verdict
//! notifications, the opt-in offers and the Settings rows. Shells render
//! these and never compose a sentence of their own.
//!
//! **Every string in this module is DRAFT, NEEDS APPROVAL.** None of it is
//! approved core copy. Nothing here reaches a shell yet: there is no C ABI
//! export, no IPC field and no caller. The export (`tc_nudge_copy_json`)
//! and the pins in the shells' copy-surface tests arrive with the approved
//! text, in a later slice.
//!
//! Every string says "trace", never "session" (Ron's #1303 design
//! review; the rest of the app's copy is renamed separately).
//!
//! Placeholders are `{name}`, filled by the daemon:
//!
//! - `{n}`, `{a}`, `{h}`, `{c}`, `{k}`: counts.
//! - `{x}` in an idle string: the idle threshold in days (owner decision
//!   26), never a session's own growing age.
//! - `{x}` in a credit string: credit to one decimal, half away from zero.
//! - `{date}`: a calendar date.
//! - `{tool}`: one tool's display name, or [`TOOL_LIST_TWO`] /
//!   [`TOOL_LIST_MANY`] filled. No folder labels and no paths, ever.
//! - `{hours}`: the digest interval in hours.
//! - `{m}`: how many matched missions a batch fits (nudge value addendum).
//! - `{low}`, `{high}`: a local credit estimate's band ends, to one decimal
//!   (multiples of 0.5). Every string carrying them says "estimate".
//! - `{known}`: how many of a batch have a local credit estimate.
//!
//! The spec gives plural forms only ("{a} sessions accepted"). A count of
//! one needs singular forms, which are not written yet and need the same
//! approval.
//!
//! The words a nudge must never use are pinned by this module's tests:
//! nothing that calls pending credit money ("earned", "paid", "settled",
//! "worth", "$"), no "private inference", no pressure ("unlock", "reward",
//! "missing out", "last chance", "streak", "in a row", "don't miss",
//! "hurry"), nothing about expiry ("expire", "dropped", "days left"), and no
//! exclamation marks. A folder mode is named only by its
//! [`crate::project_copy::ContributionModeCopy`] label.

use crate::project_copy::folder_mode_ask_label;

// ---------------------------------------------------------------------
// The unpurposed backlog (U1): Traces and History card, panel row.
// ---------------------------------------------------------------------

/// The U1 card's title. Owner decision 9 chose between Ron's first draft
/// ("{n} unpurposed traces are waiting") and a plain alternative; Ron's
/// #1303 design review (2026-10-09, adopted by the owner) replaced both
/// with this, so neither candidate is kept. DRAFT, NEEDS APPROVAL.
pub const NUDGE_BACKLOG_TITLE: &str = "{n} traces to review";
/// DRAFT, NEEDS APPROVAL. The mode is named by its one spelling.
pub const NUDGE_BACKLOG_BODY: &str = concat!(
    "They are in folders set to ",
    folder_mode_ask_label!(),
    " and have been through the preview. Nothing is sent until you decide on each one, \
     and keeping one on this computer is just as good an answer."
);
/// DRAFT, NEEDS APPROVAL.
pub const NUDGE_BACKLOG_REVIEW: &str = "Review";
/// Ron's draft (`native-app-design-data-audit.md:182`). DRAFT, NEEDS
/// APPROVAL.
pub const NUDGE_BACKLOG_REVIEW_IN_TRACES: &str = "Review the {n} in Traces";
/// Every card's dismissal. DRAFT, NEEDS APPROVAL.
pub const NUDGE_NOT_NOW: &str = "Not now";
/// The panel row for U1. Carries no number: the badge already does.
/// DRAFT, NEEDS APPROVAL.
pub const NUDGE_PANEL_BACKLOG: &str = "Previewed traces are waiting for a decision";

// ---------------------------------------------------------------------
// Idle sessions (U4): the primary phase-1 trigger.
// ---------------------------------------------------------------------

/// The panel row for U4. No number. DRAFT, NEEDS APPROVAL.
pub const NUDGE_PANEL_IDLE: &str = "Some traces have been idle for {x} days or more";
/// The Traces card's title. DRAFT, NEEDS APPROVAL.
pub const NUDGE_IDLE_TITLE: &str = "{n} traces from {tool} have been idle for {x} days or more";
/// DRAFT, NEEDS APPROVAL.
pub const NUDGE_IDLE_BODY: &str = "They look finished. Nothing is sent until you decide on each one, \
     and keeping one on this computer is just as good an answer.";
/// DRAFT, NEEDS APPROVAL.
pub const NUDGE_IDLE_REVIEW: &str = "Review";
/// The owner's draft of the N1 notification body (decision 28).
/// DRAFT, NEEDS APPROVAL.
pub const NOTIFY_IDLE_BODY_OWNER_DRAFT: &str =
    "{n} traces from {tool} have been idle for {x} days. Contribute them?";
/// The alternative to [`NOTIFY_IDLE_BODY_OWNER_DRAFT`]: a notification may
/// only open Review, so it names what Review offers -- send or keep --
/// instead of asking a question its one action cannot answer.
/// DRAFT, NEEDS APPROVAL.
pub const NOTIFY_IDLE_BODY_ALTERNATIVE: &str =
    "{n} traces from {tool} have been idle for {x} days or more. Review them to send or keep.";
/// The N1 notification body: the alternative. Owner decision 28 first
/// picked the owner's draft; Ron's #1303 design review (2026-10-09,
/// adopted by the owner) reversed that, because the notification's one
/// action opens Review. DRAFT, NEEDS APPROVAL.
pub const NOTIFY_IDLE_BODY: &str = NOTIFY_IDLE_BODY_ALTERNATIVE;
/// DRAFT, NEEDS APPROVAL.
pub const NOTIFY_ACTION_REVIEW_IDLE: &str = "Review";
/// Folded into a due digest after its first sentence, never a separate
/// notification while the digest is on. DRAFT, NEEDS APPROVAL.
pub const DIGEST_IDLE_SENTENCE: &str =
    "{n} of them, from {tool}, have been idle for {x} days or more.";
/// Appended to the badge's accessibility sentence. DRAFT, NEEDS APPROVAL.
pub const MARK_A11Y_IDLE: &str = "{n} of them have been idle for {x} days or more.";
/// The Windows tooltip clause. DRAFT, NEEDS APPROVAL.
pub const MARK_TOOLTIP_IDLE: &str = "{n} idle for {x}+ days.";
/// `{tool}` when a batch spans exactly two tools. DRAFT, NEEDS APPROVAL.
pub const TOOL_LIST_TWO: &str = "{a} and {b}";
/// `{tool}` when a batch spans three or more. DRAFT, NEEDS APPROVAL.
pub const TOOL_LIST_MANY: &str = "{k} tools";

// ---------------------------------------------------------------------
// Verdicts landed (U2).
// ---------------------------------------------------------------------

/// The History card's title. A zero clause is dropped. DRAFT, NEEDS
/// APPROVAL.
pub const NUDGE_VERDICTS_TITLE: &str = "Since {date}: {a} accepted, {h} held for privacy review";
/// Omitted when the credit is zero. "final", never "earned", "paid" or
/// "settled"; owner decision 12 asks whether "final" itself reads as
/// settled. DRAFT, NEEDS APPROVAL.
pub const NUDGE_VERDICTS_FINAL_CLAUSE: &str = "{x} credit is now final.";
/// DRAFT, NEEDS APPROVAL.
pub const NUDGE_VERDICTS_SEE: &str = "See history";
/// The panel row for U2. DRAFT, NEEDS APPROVAL.
pub const NUDGE_PANEL_VERDICTS: &str = "{a} accepted, {h} held since {date}";
/// The N2 notification body, followed by [`NUDGE_VERDICTS_FINAL_CLAUSE`]
/// when the credit is non-zero. Never reports accepted sessions without
/// the held count. DRAFT, NEEDS APPROVAL.
pub const NOTIFY_VERDICTS_BODY: &str = "{a} traces accepted and {h} held for privacy review.";
/// The verdict sentence folded into a due digest: [`NOTIFY_VERDICTS_BODY`]
/// and [`NUDGE_VERDICTS_FINAL_CLAUSE`]. Changes approved K9 digest copy
/// (owner decision 6). DRAFT, NEEDS APPROVAL.
pub const DIGEST_VERDICT_SENTENCE: &str =
    "{a} traces accepted and {h} held for privacy review. {x} credit is now final.";

// ---------------------------------------------------------------------
// The news mark.
// ---------------------------------------------------------------------

/// DRAFT, NEEDS APPROVAL.
pub const MARK_A11Y_VERDICTS: &str = "New: {a} accepted and {h} held for privacy review.";
/// Held with the weekly recap (owner decision 1). DRAFT, NEEDS APPROVAL.
pub const MARK_A11Y_RECAP: &str = "New: last week's summary.";
/// The Windows tooltip clause. DRAFT, NEEDS APPROVAL.
pub const MARK_TOOLTIP_VERDICTS: &str = "Verdicts are in.";

// ---------------------------------------------------------------------
// Notifications and the digest.
// ---------------------------------------------------------------------

/// Every re-engagement notification's title. DRAFT, NEEDS APPROVAL.
pub const NOTIFY_TITLE: &str = crate::app_name!();
/// The digest's title, moved from `Notifier.swift`. DRAFT, NEEDS APPROVAL.
pub const DIGEST_TITLE: &str = crate::app_name!();
/// Held with the weekly recap (owner decision 1). DRAFT, NEEDS APPROVAL.
pub const NOTIFY_RECAP_BODY: &str = "Last week: {c} contributed, {h} held for privacy review.";
/// Held with the weekly recap (owner decision 1). DRAFT, NEEDS APPROVAL.
pub const DIGEST_RECAP_SENTENCE: &str = "Last week: {c} contributed, {h} held for privacy review.";
/// DRAFT, NEEDS APPROVAL.
pub const NOTIFY_ACTION_SEE_HISTORY: &str = "See history";
/// DRAFT, NEEDS APPROVAL.
pub const NOTIFY_ACTION_REVIEW: &str = "Review";
/// Held with the weekly recap (owner decision 1). DRAFT, NEEDS APPROVAL.
pub const NOTIFY_ACTION_OPEN_RECAP: &str = "Open recap";
/// DRAFT, NEEDS APPROVAL.
pub const NOTIFY_ACTION_NOT_NOW: &str = "Not now";
/// Moved from `Notifier.swift`, word for word. DRAFT, NEEDS APPROVAL.
pub const DIGEST_PENDING_REASSURANCE: &str = "Nothing is sent until you review them.";
/// Moved from `Notifier.swift`. DRAFT, NEEDS APPROVAL.
pub const DIGEST_ACTION_REVIEW: &str = "Review";
/// Moved from `Notifier.swift`. DRAFT, NEEDS APPROVAL.
pub const DIGEST_ACTION_NOT_NOW: &str = "Not now";

// ---------------------------------------------------------------------
// Offers.
// ---------------------------------------------------------------------

/// The button beside `NOTIFICATION_PURPOSE` on the first U2 card.
/// DRAFT, NEEDS APPROVAL.
pub const OFFER_NOTIFY_VERDICTS: &str = "Tell me when traces are judged";
/// The one-time offer to existing installs (owner decision 3). "At most
/// twice a week" restates the N2 per-kind cap of owner decision 4 and
/// changes with it. DRAFT, NEEDS APPROVAL.
pub const OFFER_NOTIFY_VERDICTS_EXISTING: &str = "Traces you send are now judged in the background. \
     Want a notification when that happens? At most twice a week.";
/// The one-time offer to existing installs on the Traces card (owner
/// decision 7). "At most once a week" restates the N1 repeat interval of
/// owner decisions 4 and 26 at the default queue TTL and changes with it.
/// Wording from Ron's #1303 design review. DRAFT, NEEDS APPROVAL.
pub const OFFER_NOTIFY_IDLE_EXISTING: &str =
    "We can tell you when traces sit idle for a few days. At most once a week.";
/// Accepts an offer. "Enable", not "Turn on", per Ron's #1303 design
/// review. DRAFT, NEEDS APPROVAL.
pub const OFFER_ENABLE: &str = "Enable";
/// Declines an offer. DRAFT, NEEDS APPROVAL.
pub const OFFER_NO_THANKS: &str = "No thanks";

// ---------------------------------------------------------------------
// Settings.
// ---------------------------------------------------------------------

/// DRAFT, NEEDS APPROVAL.
pub const SETTING_SUGGESTIONS: &str = "Show suggestions in Traces, History and the menu bar";
/// DRAFT, NEEDS APPROVAL.
pub const SETTING_SUGGESTIONS_HELP: &str = "These suggestions are worked out on this computer. The counts behind them are not sent anywhere.";
/// DRAFT, NEEDS APPROVAL.
pub const SETTING_MARK: &str =
    "Show a small ring on the menu bar icon when there is something new to look at";
/// DRAFT, NEEDS APPROVAL.
pub const SETTING_MARK_HELP: &str =
    "It never appears while traces are waiting for your decision; the number does.";
/// DRAFT, NEEDS APPROVAL.
pub const SETTING_NOTIFY_MASTER: &str = concat!("Notifications from ", crate::app_name!());
/// DRAFT, NEEDS APPROVAL.
pub const SETTING_DIGEST: &str = "Waiting and contributed traces";
/// Under the Interval schedule. The same words, and the same `{hours}`
/// placeholder, as the Settings line it replaces
/// (`SettingsWords.at_most_one_notification`). DRAFT, NEEDS APPROVAL.
pub const SETTING_DIGEST_HELP_INTERVAL: &str =
    "At most one notification every {hours} hours, and none when nothing is waiting.";
/// Under the Evening schedule. DRAFT, NEEDS APPROVAL.
pub const SETTING_DIGEST_HELP_EVENING: &str =
    "At most one notification each evening, and none when nothing is waiting.";
/// DRAFT, NEEDS APPROVAL.
pub const SETTING_NOTIFY_VERDICTS: &str = "When traces you sent are judged";
/// DRAFT, NEEDS APPROVAL.
pub const SETTING_NOTIFY_IDLE: &str = "When traces have been idle for a few days";
/// DRAFT, NEEDS APPROVAL.
pub const SETTING_NOTIFY_IDLE_HELP: &str = "When the regular notification is on, this is one extra sentence in it, \
     not a separate notification.";
/// Held with the weekly recap (owner decision 1). DRAFT, NEEDS APPROVAL.
pub const SETTING_NOTIFY_RECAP: &str = "A summary of last week";
/// Restates the global caps of owner decision 4 (one a day, three a week,
/// a minimum gap) and the quiet hours of owner decision 5 (21:00-09:00),
/// and changes with them. DRAFT, NEEDS APPROVAL.
pub const SETTING_NOTIFY_BUDGET_HELP: &str = concat!(
    "Apart from the regular notification above, ",
    crate::app_name!(),
    " sends at most one a day and three a week, none between 9 pm and 9 am, \
     and never two close together."
);
/// DRAFT, NEEDS APPROVAL.
pub const ABOUT_SUGGESTIONS: &str =
    "These counts are kept on this computer only, and are removed when you sign out.";

// ---------------------------------------------------------------------
// Mission fit and the suggested order (nudge value addendum, revision 1).
// ---------------------------------------------------------------------

/// Appended to the U4/U1 card body and the digest's idle sentence, only
/// when `mission_fit` is above zero; never "0 of them". DRAFT, NEEDS
/// APPROVAL.
pub const NUDGE_MISSION_FIT_CLAUSE: &str = "{m} of them fit a mission.";
/// [`NUDGE_MISSION_FIT_CLAUSE`] for exactly one. DRAFT, NEEDS APPROVAL.
pub const NUDGE_MISSION_FIT_CLAUSE_ONE: &str = "1 of them fits a mission.";
/// The Traces sort control, `list_pending {order: "suggested"}`. DRAFT,
/// NEEDS APPROVAL.
pub const LIST_ORDER_SUGGESTED: &str = "Suggested first";
/// The Traces sort control, queue order. DRAFT, NEEDS APPROVAL.
pub const LIST_ORDER_QUEUE: &str = "Oldest first";
/// A row tag, only when the entry's `mission_fit` is above zero. DRAFT,
/// NEEDS APPROVAL.
pub const ENTRY_MISSION_FIT: &str = "Fits a mission";

// ---------------------------------------------------------------------
// The local credit estimate (nudge value addendum, revision 2).
// ---------------------------------------------------------------------

/// Appended to the U4/U1 card body when `credit_estimate.known` equals the
/// count. Absent with no `credit_estimate`; never "about 0". DRAFT, NEEDS
/// APPROVAL.
pub const NUDGE_ESTIMATE_CLAUSE: &str = "Estimated credit before scoring: about {low} to {high}.";
/// [`NUDGE_ESTIMATE_CLAUSE`] when only some of the batch have an estimate.
/// DRAFT, NEEDS APPROVAL.
pub const NUDGE_ESTIMATE_CLAUSE_PARTIAL: &str =
    "Estimated credit for {known} of them, before scoring: about {low} to {high}.";
/// A row's detail line. DRAFT, NEEDS APPROVAL.
pub const ENTRY_ESTIMATE_BAND: &str = "Estimate: about {low} to {high} credit";
/// A row tag, multi-tier tables only. DRAFT, NEEDS APPROVAL.
pub const ENTRY_ESTIMATE_TIER_HIGHER: &str = "Higher estimate";
/// A row tag, multi-tier tables only. DRAFT, NEEDS APPROVAL.
pub const ENTRY_ESTIMATE_TIER_MIDDLE: &str = "Typical estimate";
/// A row tag, multi-tier tables only. DRAFT, NEEDS APPROVAL.
pub const ENTRY_ESTIMATE_TIER_LOWER: &str = "Lower estimate";
/// The estimate's info popover. DRAFT, NEEDS APPROVAL.
pub const ESTIMATE_EXPLAINER: &str = "Made on this device from the trace's size and makeup. \
     Nothing about the trace is sent to make it. The credit a trace gets is set when \
     the commons scores it, and can differ, including 0 when the server reads it as a repeat.";

/// Every nudge string, as `(key, text)`, including both candidates of each
/// owner pick still open so the word rules hold whichever is chosen. The later C ABI
/// export is built from this table.
pub const NUDGE_COPY: &[(&str, &str)] = &[
    ("NUDGE_BACKLOG_TITLE", NUDGE_BACKLOG_TITLE),
    ("NUDGE_BACKLOG_BODY", NUDGE_BACKLOG_BODY),
    ("NUDGE_BACKLOG_REVIEW", NUDGE_BACKLOG_REVIEW),
    (
        "NUDGE_BACKLOG_REVIEW_IN_TRACES",
        NUDGE_BACKLOG_REVIEW_IN_TRACES,
    ),
    ("NUDGE_NOT_NOW", NUDGE_NOT_NOW),
    ("NUDGE_PANEL_BACKLOG", NUDGE_PANEL_BACKLOG),
    ("NUDGE_PANEL_IDLE", NUDGE_PANEL_IDLE),
    ("NUDGE_IDLE_TITLE", NUDGE_IDLE_TITLE),
    ("NUDGE_IDLE_BODY", NUDGE_IDLE_BODY),
    ("NUDGE_IDLE_REVIEW", NUDGE_IDLE_REVIEW),
    ("NOTIFY_IDLE_BODY", NOTIFY_IDLE_BODY),
    ("NOTIFY_IDLE_BODY_OWNER_DRAFT", NOTIFY_IDLE_BODY_OWNER_DRAFT),
    ("NOTIFY_IDLE_BODY_ALTERNATIVE", NOTIFY_IDLE_BODY_ALTERNATIVE),
    ("NOTIFY_ACTION_REVIEW_IDLE", NOTIFY_ACTION_REVIEW_IDLE),
    ("DIGEST_IDLE_SENTENCE", DIGEST_IDLE_SENTENCE),
    ("MARK_A11Y_IDLE", MARK_A11Y_IDLE),
    ("MARK_TOOLTIP_IDLE", MARK_TOOLTIP_IDLE),
    ("TOOL_LIST_TWO", TOOL_LIST_TWO),
    ("TOOL_LIST_MANY", TOOL_LIST_MANY),
    ("NUDGE_VERDICTS_TITLE", NUDGE_VERDICTS_TITLE),
    ("NUDGE_VERDICTS_FINAL_CLAUSE", NUDGE_VERDICTS_FINAL_CLAUSE),
    ("NUDGE_VERDICTS_SEE", NUDGE_VERDICTS_SEE),
    ("NUDGE_PANEL_VERDICTS", NUDGE_PANEL_VERDICTS),
    ("NOTIFY_VERDICTS_BODY", NOTIFY_VERDICTS_BODY),
    ("DIGEST_VERDICT_SENTENCE", DIGEST_VERDICT_SENTENCE),
    ("MARK_A11Y_VERDICTS", MARK_A11Y_VERDICTS),
    ("MARK_A11Y_RECAP", MARK_A11Y_RECAP),
    ("MARK_TOOLTIP_VERDICTS", MARK_TOOLTIP_VERDICTS),
    ("NOTIFY_TITLE", NOTIFY_TITLE),
    ("DIGEST_TITLE", DIGEST_TITLE),
    ("NOTIFY_RECAP_BODY", NOTIFY_RECAP_BODY),
    ("DIGEST_RECAP_SENTENCE", DIGEST_RECAP_SENTENCE),
    ("NOTIFY_ACTION_SEE_HISTORY", NOTIFY_ACTION_SEE_HISTORY),
    ("NOTIFY_ACTION_REVIEW", NOTIFY_ACTION_REVIEW),
    ("NOTIFY_ACTION_OPEN_RECAP", NOTIFY_ACTION_OPEN_RECAP),
    ("NOTIFY_ACTION_NOT_NOW", NOTIFY_ACTION_NOT_NOW),
    ("DIGEST_PENDING_REASSURANCE", DIGEST_PENDING_REASSURANCE),
    ("DIGEST_ACTION_REVIEW", DIGEST_ACTION_REVIEW),
    ("DIGEST_ACTION_NOT_NOW", DIGEST_ACTION_NOT_NOW),
    ("OFFER_NOTIFY_VERDICTS", OFFER_NOTIFY_VERDICTS),
    (
        "OFFER_NOTIFY_VERDICTS_EXISTING",
        OFFER_NOTIFY_VERDICTS_EXISTING,
    ),
    ("OFFER_NOTIFY_IDLE_EXISTING", OFFER_NOTIFY_IDLE_EXISTING),
    ("OFFER_ENABLE", OFFER_ENABLE),
    ("OFFER_NO_THANKS", OFFER_NO_THANKS),
    ("SETTING_SUGGESTIONS", SETTING_SUGGESTIONS),
    ("SETTING_SUGGESTIONS_HELP", SETTING_SUGGESTIONS_HELP),
    ("SETTING_MARK", SETTING_MARK),
    ("SETTING_MARK_HELP", SETTING_MARK_HELP),
    ("SETTING_NOTIFY_MASTER", SETTING_NOTIFY_MASTER),
    ("SETTING_DIGEST", SETTING_DIGEST),
    ("SETTING_DIGEST_HELP_INTERVAL", SETTING_DIGEST_HELP_INTERVAL),
    ("SETTING_DIGEST_HELP_EVENING", SETTING_DIGEST_HELP_EVENING),
    ("SETTING_NOTIFY_VERDICTS", SETTING_NOTIFY_VERDICTS),
    ("SETTING_NOTIFY_IDLE", SETTING_NOTIFY_IDLE),
    ("SETTING_NOTIFY_IDLE_HELP", SETTING_NOTIFY_IDLE_HELP),
    ("SETTING_NOTIFY_RECAP", SETTING_NOTIFY_RECAP),
    ("SETTING_NOTIFY_BUDGET_HELP", SETTING_NOTIFY_BUDGET_HELP),
    ("ABOUT_SUGGESTIONS", ABOUT_SUGGESTIONS),
    ("NUDGE_MISSION_FIT_CLAUSE", NUDGE_MISSION_FIT_CLAUSE),
    ("NUDGE_MISSION_FIT_CLAUSE_ONE", NUDGE_MISSION_FIT_CLAUSE_ONE),
    ("LIST_ORDER_SUGGESTED", LIST_ORDER_SUGGESTED),
    ("LIST_ORDER_QUEUE", LIST_ORDER_QUEUE),
    ("ENTRY_MISSION_FIT", ENTRY_MISSION_FIT),
    ("NUDGE_ESTIMATE_CLAUSE", NUDGE_ESTIMATE_CLAUSE),
    (
        "NUDGE_ESTIMATE_CLAUSE_PARTIAL",
        NUDGE_ESTIMATE_CLAUSE_PARTIAL,
    ),
    ("ENTRY_ESTIMATE_BAND", ENTRY_ESTIMATE_BAND),
    ("ENTRY_ESTIMATE_TIER_HIGHER", ENTRY_ESTIMATE_TIER_HIGHER),
    ("ENTRY_ESTIMATE_TIER_MIDDLE", ENTRY_ESTIMATE_TIER_MIDDLE),
    ("ENTRY_ESTIMATE_TIER_LOWER", ENTRY_ESTIMATE_TIER_LOWER),
    ("ESTIMATE_EXPLAINER", ESTIMATE_EXPLAINER),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_copy::{
        CONTRIBUTION_MODE_ASK_LABEL, CONTRIBUTION_MODE_AUTO_LABEL, CONTRIBUTION_MODE_NEVER_LABEL,
        contribution_mode_copy,
    };

    /// Every key spec section 6 names, the combined rows split into one key
    /// each. The offer buttons the spec gives as words only are
    /// `OFFER_ENABLE` and `OFFER_NO_THANKS`.
    const SPEC_KEYS: &[&str] = &[
        "NUDGE_BACKLOG_TITLE",
        "NUDGE_BACKLOG_BODY",
        "NUDGE_BACKLOG_REVIEW",
        "NUDGE_BACKLOG_REVIEW_IN_TRACES",
        "NUDGE_NOT_NOW",
        "NUDGE_PANEL_BACKLOG",
        "NUDGE_PANEL_IDLE",
        "NUDGE_VERDICTS_TITLE",
        "NUDGE_VERDICTS_FINAL_CLAUSE",
        "NUDGE_VERDICTS_SEE",
        "NUDGE_PANEL_VERDICTS",
        "MARK_A11Y_VERDICTS",
        "MARK_A11Y_RECAP",
        "MARK_TOOLTIP_VERDICTS",
        "NOTIFY_TITLE",
        "NOTIFY_VERDICTS_BODY",
        "NOTIFY_IDLE_BODY",
        "NOTIFY_ACTION_REVIEW_IDLE",
        "NUDGE_IDLE_TITLE",
        "NUDGE_IDLE_BODY",
        "NUDGE_IDLE_REVIEW",
        "DIGEST_IDLE_SENTENCE",
        "MARK_A11Y_IDLE",
        "MARK_TOOLTIP_IDLE",
        "TOOL_LIST_TWO",
        "TOOL_LIST_MANY",
        "NOTIFY_RECAP_BODY",
        "NOTIFY_ACTION_SEE_HISTORY",
        "NOTIFY_ACTION_REVIEW",
        "NOTIFY_ACTION_OPEN_RECAP",
        "NOTIFY_ACTION_NOT_NOW",
        "OFFER_NOTIFY_VERDICTS",
        "OFFER_NOTIFY_VERDICTS_EXISTING",
        "OFFER_NOTIFY_IDLE_EXISTING",
        "OFFER_ENABLE",
        "OFFER_NO_THANKS",
        "DIGEST_PENDING_REASSURANCE",
        "DIGEST_VERDICT_SENTENCE",
        "DIGEST_RECAP_SENTENCE",
        "DIGEST_TITLE",
        "DIGEST_ACTION_REVIEW",
        "DIGEST_ACTION_NOT_NOW",
        "SETTING_SUGGESTIONS",
        "SETTING_SUGGESTIONS_HELP",
        "SETTING_MARK",
        "SETTING_MARK_HELP",
        "SETTING_NOTIFY_MASTER",
        "SETTING_DIGEST",
        "SETTING_DIGEST_HELP_INTERVAL",
        "SETTING_DIGEST_HELP_EVENING",
        "SETTING_NOTIFY_VERDICTS",
        "SETTING_NOTIFY_IDLE",
        "SETTING_NOTIFY_IDLE_HELP",
        "SETTING_NOTIFY_RECAP",
        "SETTING_NOTIFY_BUDGET_HELP",
        "ABOUT_SUGGESTIONS",
    ];

    /// The longest a notification body may be: Windows clamps a tray
    /// balloon's text at 255 characters (`TrayIcon.cs`), and a body that
    /// is cut there loses a figure.
    const NOTIFICATION_BODY_MAX_CHARS: usize = 255;

    /// The widest count a body can carry: every count is at most a `u64`.
    const MAX_COUNT: &str = "18446744073709551615";
    /// The widest credit figure: a `u64` worth of credit to one decimal.
    const MAX_CREDIT: &str = "18446744073709551615.0";
    /// The widest idle threshold. Owner decision 26 keeps it at or below
    /// 7 days at any queue TTL; two digits leave room.
    const MAX_IDLE_DAYS: &str = "99";
    /// A tool display name at the 64-character cap the daemon applies to
    /// every label it puts in notification text (spec section 4.2).
    const MAX_TOOL: &str = "TTTTTTTTTTTTTTTTTTTTTTTTTTTTTTTTTTTTTTTTTTTTTTTTTTTTTTTTTTTTTTTT";
    /// A long spelled-out date, wider than any the daemon writes.
    const MAX_DATE: &str = "Wednesday, 30 September 2026";
    /// The widest estimate band end: `ESTIMATE_MAX_DISPLAYED_CREDIT` bounds
    /// a single band, and a summed band over a large batch is wider, so
    /// this leaves room for both (nudge value addendum, 3.3).
    const MAX_ESTIMATE: &str = "999.5";
    /// The widest `{known}` the estimate clause is drawn with.
    const MAX_KNOWN: &str = "9999";

    /// `{tool}` at its widest: two capped names, which is longer than
    /// [`TOOL_LIST_MANY`] with any count.
    fn max_tool() -> String {
        let two = TOOL_LIST_TWO
            .replace("{a}", MAX_TOOL)
            .replace("{b}", MAX_TOOL);
        let many = TOOL_LIST_MANY.replace("{k}", MAX_COUNT);
        if two.chars().count() >= many.chars().count() {
            two
        } else {
            many
        }
    }

    /// `template` with every placeholder at its widest.
    fn fill_max(template: &str, credit_x: bool) -> String {
        let x = if credit_x { MAX_CREDIT } else { MAX_IDLE_DAYS };
        let filled = template
            .replace("{n}", MAX_COUNT)
            .replace("{a}", MAX_COUNT)
            .replace("{h}", MAX_COUNT)
            .replace("{c}", MAX_COUNT)
            .replace("{k}", MAX_COUNT)
            .replace("{hours}", MAX_COUNT)
            .replace("{m}", MAX_COUNT)
            .replace("{low}", MAX_ESTIMATE)
            .replace("{high}", MAX_ESTIMATE)
            .replace("{known}", MAX_KNOWN)
            .replace("{date}", MAX_DATE)
            .replace("{tool}", &max_tool())
            .replace("{x}", x);
        assert!(
            !filled.contains('{'),
            "unfilled placeholder in {template}: {filled}"
        );
        filled
    }

    #[test]
    fn the_table_holds_every_section_6_key() {
        for key in SPEC_KEYS {
            assert!(
                NUDGE_COPY.iter().any(|(k, _)| k == key),
                "missing key {key}"
            );
        }
    }

    #[test]
    fn every_key_is_listed_once_with_non_blank_text() {
        let mut seen = std::collections::BTreeSet::new();
        for (key, text) in NUDGE_COPY {
            assert!(seen.insert(*key), "duplicate key {key}");
            assert!(!text.trim().is_empty(), "{key} is blank");
        }
    }

    /// Credit is pending or final, never money: the `notify.rs` ban list.
    /// And nothing that leans on the contributor: no pressure words, no
    /// countdown, nothing about expiry (constraint 20), no exclamation.
    #[test]
    fn no_string_uses_a_banned_word() {
        const BANNED: &[&str] = &[
            "earned",
            "paid",
            "settled",
            "worth",
            "$",
            "private inference",
            "unlock",
            "reward",
            "missing out",
            "last chance",
            "streak",
            "in a row",
            "don't miss",
            "don\u{2019}t miss",
            "hurry",
            "expire",
            "dropped",
            "days left",
            "!",
        ];
        for (key, text) in NUDGE_COPY {
            let lower = text.to_lowercase();
            for word in BANNED {
                assert!(!lower.contains(word), "{key} says {word:?}: {text}");
            }
        }
    }

    /// A folder mode is named only by its `ContributionModeCopy` label: no
    /// wire value, no lower-case spelling and no older name.
    #[test]
    fn mode_words_come_only_from_contribution_mode_copy() {
        const WRONG_SPELLINGS: &[&str] = &[
            "notify_only",
            "auto_upload",
            "ignore",
            "ask me",
            "automatic",
            "auto-upload",
            "auto upload",
            "notify only",
            "review each",
            "ask first",
        ];
        let labels = [
            CONTRIBUTION_MODE_ASK_LABEL,
            CONTRIBUTION_MODE_AUTO_LABEL,
            CONTRIBUTION_MODE_NEVER_LABEL,
        ];
        for (key, text) in NUDGE_COPY {
            let mut rest = (*text).to_string();
            for label in labels {
                rest = rest.replace(label, "");
            }
            let rest = rest.to_lowercase();
            for word in WRONG_SPELLINGS {
                assert!(!rest.contains(word), "{key} spells a mode {word:?}: {text}");
            }
        }
        let ask = contribution_mode_copy()
            .choices
            .into_iter()
            .find(|choice| choice.mode == "notify_only")
            .expect("the Ask me choice")
            .label;
        assert!(NUDGE_BACKLOG_BODY.contains(ask), "{NUDGE_BACKLOG_BODY}");
    }

    /// Every notification body, composed as the daemon sends it, fits the
    /// Windows balloon with every figure at its widest. Each folded digest
    /// sentence fits on its own too, since it is what Windows drops first.
    #[test]
    fn every_notification_body_fits_the_windows_clamp_at_maximal_counts() {
        let bodies = [
            (
                "NOTIFY_VERDICTS_BODY + NUDGE_VERDICTS_FINAL_CLAUSE",
                format!(
                    "{} {}",
                    fill_max(NOTIFY_VERDICTS_BODY, true),
                    fill_max(NUDGE_VERDICTS_FINAL_CLAUSE, true)
                ),
            ),
            (
                "NOTIFY_IDLE_BODY_OWNER_DRAFT",
                fill_max(NOTIFY_IDLE_BODY_OWNER_DRAFT, false),
            ),
            (
                "NOTIFY_IDLE_BODY_ALTERNATIVE",
                fill_max(NOTIFY_IDLE_BODY_ALTERNATIVE, false),
            ),
            ("NOTIFY_RECAP_BODY", fill_max(NOTIFY_RECAP_BODY, false)),
            (
                "DIGEST_IDLE_SENTENCE",
                fill_max(DIGEST_IDLE_SENTENCE, false),
            ),
            (
                "DIGEST_VERDICT_SENTENCE",
                fill_max(DIGEST_VERDICT_SENTENCE, true),
            ),
            (
                "DIGEST_RECAP_SENTENCE",
                fill_max(DIGEST_RECAP_SENTENCE, false),
            ),
            // The digest's idle sentence carries the mission clause when
            // sessions fit one (nudge value addendum, 3.3).
            (
                "DIGEST_IDLE_SENTENCE + NUDGE_MISSION_FIT_CLAUSE",
                format!(
                    "{} {}",
                    fill_max(DIGEST_IDLE_SENTENCE, false),
                    fill_max(NUDGE_MISSION_FIT_CLAUSE, false)
                ),
            ),
            // The estimate clauses, each at its widest.
            (
                "NUDGE_ESTIMATE_CLAUSE",
                fill_max(NUDGE_ESTIMATE_CLAUSE, false),
            ),
            (
                "NUDGE_ESTIMATE_CLAUSE_PARTIAL",
                fill_max(NUDGE_ESTIMATE_CLAUSE_PARTIAL, false),
            ),
            ("ENTRY_ESTIMATE_BAND", fill_max(ENTRY_ESTIMATE_BAND, false)),
        ];
        for (key, body) in bodies {
            let len = body.chars().count();
            assert!(
                len < NOTIFICATION_BODY_MAX_CHARS,
                "{key} is {len} characters at its widest: {body}"
            );
        }
    }

    /// One spelling for each shared phrase, so a shell that shows two of
    /// them side by side never shows two wordings.
    #[test]
    fn shared_phrases_have_one_spelling() {
        assert_eq!(NOTIFY_TITLE, crate::app_name!());
        assert_eq!(DIGEST_TITLE, NOTIFY_TITLE);
        assert_eq!(
            DIGEST_VERDICT_SENTENCE,
            format!("{NOTIFY_VERDICTS_BODY} {NUDGE_VERDICTS_FINAL_CLAUSE}")
        );
        assert_eq!(DIGEST_RECAP_SENTENCE, NOTIFY_RECAP_BODY);
        assert_eq!(
            SETTING_DIGEST_HELP_INTERVAL,
            crate::shell_words_copy::settings_words().at_most_one_notification
        );
        for review in [
            NUDGE_BACKLOG_REVIEW,
            NUDGE_IDLE_REVIEW,
            NOTIFY_ACTION_REVIEW,
            NOTIFY_ACTION_REVIEW_IDLE,
            DIGEST_ACTION_REVIEW,
        ] {
            assert_eq!(review, "Review");
        }
        assert_eq!(NUDGE_NOT_NOW, NOTIFY_ACTION_NOT_NOW);
        assert_eq!(DIGEST_ACTION_NOT_NOW, NOTIFY_ACTION_NOT_NOW);
        assert_eq!(NUDGE_VERDICTS_SEE, NOTIFY_ACTION_SEE_HISTORY);
    }

    /// The owner picks resolve to one of their candidates, and the idle
    /// strings state the threshold, never a session's own age.
    #[test]
    fn owner_picks_select_a_candidate_and_idle_strings_name_the_threshold() {
        assert!(
            [NOTIFY_IDLE_BODY_OWNER_DRAFT, NOTIFY_IDLE_BODY_ALTERNATIVE]
                .contains(&NOTIFY_IDLE_BODY)
        );
        for (key, text) in NUDGE_COPY {
            if text.contains("idle for") && !text.contains("a few days") {
                assert!(
                    text.contains("{x}"),
                    "{key} must name the threshold: {text}"
                );
            }
        }
    }

    /// Mission wording is count-based: no mission string carries a credit
    /// placeholder, because the catalogue has no amount and the session
    /// estimate has its own strings (nudge value addendum, 3.2). Scoped to
    /// the mission keys, since the estimate strings carry one by design.
    #[test]
    fn no_mission_string_carries_a_credit_placeholder() {
        let mission: Vec<_> = NUDGE_COPY
            .iter()
            .filter(|(key, _)| key.contains("MISSION"))
            .collect();
        assert_eq!(mission.len(), 3, "{mission:?}");
        for (key, text) in mission {
            for placeholder in ["{x}", "{low}", "{high}"] {
                assert!(!text.contains(placeholder), "{key}: {text}");
            }
        }
    }

    /// Every string with an estimate figure says it is an estimate, so it
    /// can never be read as scored credit (nudge value addendum, 4.7).
    #[test]
    fn every_estimate_figure_is_labelled_an_estimate() {
        let mut figures = 0;
        for (key, text) in NUDGE_COPY {
            if text.contains("{low}") || text.contains("{high}") {
                figures += 1;
                assert!(
                    text.to_lowercase().contains("estimate"),
                    "{key} carries an estimate figure without saying so: {text}"
                );
            }
        }
        assert_eq!(figures, 3);
    }

    /// Ron's #1303 design review, item 8: the backlog title names the
    /// action, and no nudge calls a trace "unpurposed".
    #[test]
    fn the_backlog_title_is_traces_to_review() {
        assert_eq!(NUDGE_BACKLOG_TITLE, "{n} traces to review");
        for (key, text) in NUDGE_COPY {
            assert!(!text.to_lowercase().contains("unpurposed"), "{key}: {text}");
        }
    }

    /// Ron's #1303 design review, item 11: the offer buttons.
    #[test]
    fn the_offer_buttons_are_enable_and_no_thanks() {
        let key = |want: &str| {
            NUDGE_COPY
                .iter()
                .find(|(k, _)| *k == want)
                .map(|(_, text)| *text)
        };
        assert_eq!(key("OFFER_ENABLE"), Some("Enable"));
        assert_eq!(key("OFFER_NO_THANKS"), Some("No thanks"));
        assert_eq!(key("OFFER_TURN_ON"), None);
    }

    /// Ron's #1303 design review, item 12: the idle offer's text.
    #[test]
    fn the_idle_offer_says_what_it_will_tell_you() {
        assert_eq!(
            OFFER_NOTIFY_IDLE_EXISTING,
            "We can tell you when traces sit idle for a few days. At most once a week."
        );
    }

    /// Ron's #1303 design review, item 13: a nudge says "trace", never
    /// "session". The rest of the app's copy is renamed separately.
    #[test]
    fn no_nudge_string_says_session() {
        for (key, text) in NUDGE_COPY {
            assert!(
                !text.to_lowercase().contains("session"),
                "{key} says session: {text}"
            );
        }
        assert_eq!(
            NUDGE_IDLE_TITLE
                .replace("{n}", "2")
                .replace("{tool}", "Claude Code")
                .replace("{x}", "3"),
            "2 traces from Claude Code have been idle for 3 days or more"
        );
    }

    /// Ron's #1303 design review, item 10, reversing owner decision 28:
    /// the idle notification names what Review offers.
    #[test]
    fn the_idle_notification_uses_the_alternative() {
        assert_eq!(NOTIFY_IDLE_BODY, NOTIFY_IDLE_BODY_ALTERNATIVE);
        assert!(NOTIFY_IDLE_BODY.ends_with("Review them to send or keep."));
    }

    /// No path, folder label or title placeholder can reach a nudge.
    #[test]
    fn no_string_has_a_path_or_folder_placeholder() {
        for (key, text) in NUDGE_COPY {
            for placeholder in ["{path}", "{project}", "{folder}", "{title}", "{label}"] {
                assert!(!text.contains(placeholder), "{key}: {text}");
            }
        }
    }
}
