//! The words the macOS shell still wrote in Swift, now the core's.
//!
//! Until #1146 parity (2026-10-07) the native app held five tables of its
//! own: withdrawal (`WithdrawalCopy.swift`), the public profile
//! (`PublicProfileCopy.swift`), the legacy queue and History words
//! (`QueueLegacyWords`, `HistoryLegacyWords`), the scrubbing caveat
//! (`ScrubbingCaveat.swift`) and the Settings sections' sentences
//! (`SettingsLegacyWords`). A sentence written in one shell survives a
//! rename in the others, so they live here now and cross the C ABI as one
//! JSON object (`tc_shell_words_copy_json`).
//!
//! Every number or name a sentence carries is a `{name}` hole from
//! [`SHELL_WORDS_PLACEHOLDERS`]; the shell fills it and adds nothing else.
//!
//! The wording follows Ron's #1146 frontend
//! (`origin/claude/tc-monitor-frontend-refactor-8ecf22`) where #1146 has a
//! counterpart, by the owner's ruling (2026-10-06: #1146's wording wins),
//! with these held back:
//!
//! - The three canonical withdrawal confirmation bodies are
//!   `docs/contributor-daemon-ipc-v1_1.md`'s "Canonical confirmation copy",
//!   verbatim. #1146 labels the dialog; the tiers keep their semantics.
//! - The undo card counts up ("Approved 7s ago"), never down: the deadline
//!   is the daemon's next upload sweep, which nothing in a shell can see
//!   (owner ruling: undo count-up stays).
//! - A withdrawal that did not happen says "Nothing was withdrawn" as a
//!   sentence of its own, and the signed-out one, sent before any request,
//!   never claims an answer from the server. A profile change that did not
//!   happen says so first.
//!
//! Consent and disclosure wording changed for #1146 parity, or for accuracy
//! after the #1273 review, is marked "Approved 2026-10-07" once the owner
//! approved it; a sentence still marked "DRAFT, NEEDS APPROVAL" is not yet
//! approved.

use std::collections::BTreeMap;

use serde::Serialize;

/// Every `{name}` hole the tables below use.
pub const SHELL_WORDS_PLACEHOLDERS: &[&str] = &[
    "count", "total", "seconds", "date", "reason", "hours", "days", "message", "relative", "title",
];

// ---------------------------------------------------------------------------
// Withdrawal

/// `not_distributed`, verbatim from the IPC contract's canonical table.
pub const WITHDRAWAL_NOT_DISTRIBUTED: &str = concat!(
    "This trace never entered the commons. Withdrawing deletes it. Nothing was ",
    "distributed and nothing needs recalling."
);

/// `commons_not_distributed`, verbatim from the IPC contract's canonical table.
pub const WITHDRAWAL_COMMONS_NOT_DISTRIBUTED: &str = concat!(
    "This trace is in the commons but has not been included in any published export ",
    "or benchmark yet. Withdrawing deletes it and excludes it from everything ",
    "published from here on."
);

/// `commons_distributed`, verbatim from the IPC contract's canonical table.
/// The clause from "but copies" onward must never be softened, shortened or
/// dropped.
pub const WITHDRAWAL_COMMONS_DISTRIBUTED: &str = concat!(
    "This trace has already been included in a published export or benchmark. ",
    "Withdrawing deletes our copy and excludes it from everything published from ",
    "here on, but copies that have already been distributed cannot be recalled. ",
    "Withdrawing does not undo that."
);

/// Settled credit is not clawed back; pending credit is forfeited. The same
/// sentence `withdraw::confirmation_prompt` ends with.
pub const WITHDRAWAL_CREDIT_NOTE: &str =
    "Credit that has already settled stays. Credit still pending is forfeited.";

/// Withdrawal's words: the confirmation, the result, and the refusals.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct WithdrawalWords {
    /// A row's Withdraw button. Approved 2026-10-07.
    pub withdraw: &'static str,
    /// The confirmation's title (#1146 `withdrawal-control.tsx`). Approved
    /// 2026-10-07 with the parity set; it was "Withdraw this trace?".
    pub confirm_title: &'static str,
    /// The confirmation's description (#1146). Approved 2026-10-07: new; there was none.
    pub confirm_description: &'static str,
    /// The confirmation's way back.
    pub keep: &'static str,
    /// The confirmation's action, for every tier: "Withdraw". Approved
    /// 2026-10-07 (owner ruling; #1146 said "Confirm withdrawal").
    pub confirm: &'static str,
    pub withdrawing: &'static str,
    /// No confirmation could be worded, so withdrawal is not offered.
    pub disclosure_unavailable: &'static str,
    /// An accepted trace may be in either commons tier, and this machine
    /// cannot tell which; said before both canonical bodies.
    pub ambiguity: &'static str,
    pub not_distributed: &'static str,
    pub commons_not_distributed: &'static str,
    pub commons_distributed: &'static str,
    pub credit_note: &'static str,
    /// Over a completed withdrawal's result.
    pub result_heading: &'static str,
    /// What the tier the server applied did. Approved 2026-10-07: they
    /// were "Withdrawn. " + the canonical body.
    pub result_not_distributed: &'static str,
    pub result_commons_not_distributed: &'static str,
    pub result_commons_distributed: &'static str,
    /// The server did not say which tier applied, so the furthest is not
    /// ruled out. Approved 2026-10-07.
    pub result_unknown: &'static str,
    /// No usable account session on this machine, so no request was made.
    /// Approved 2026-10-07.
    pub account_session_required: &'static str,
    /// No such submission under this account; never says which of "not
    /// yours" and "does not exist".
    pub not_found: &'static str,
    /// The daemon's labels for [`Self::not_found`].
    pub not_found_labels: &'static [&'static str],
    /// Any other failure. Approved 2026-10-07.
    pub failed: &'static str,
    pub try_again: &'static str,
    /// Why there is no bulk withdrawal.
    pub no_bulk_action: &'static str,
    /// The defect notice when the shell's withdrawal checks fail.
    pub wording_defect: &'static str,
}

#[must_use]
pub fn withdrawal_words() -> WithdrawalWords {
    WithdrawalWords {
        withdraw: "Withdraw",
        confirm_title: "Confirm withdrawal",
        confirm_description: "Review what withdrawal changes before continuing.",
        keep: "Keep it",
        confirm: "Withdraw",
        withdrawing: "Withdrawing\u{2026}",
        disclosure_unavailable: "Withdrawal disclosure unavailable. Withdrawal is disabled.",
        ambiguity: concat!(
            "This trace is in the commons. Whether it has already gone into a published ",
            "export or benchmark is decided on the server, and this app cannot tell from ",
            "here which of these two applies:"
        ),
        not_distributed: WITHDRAWAL_NOT_DISTRIBUTED,
        commons_not_distributed: WITHDRAWAL_COMMONS_NOT_DISTRIBUTED,
        commons_distributed: WITHDRAWAL_COMMONS_DISTRIBUTED,
        credit_note: WITHDRAWAL_CREDIT_NOTE,
        result_heading: "Withdrawn by you",
        // Approved 2026-10-07.
        result_not_distributed: "Content deleted. It was never shared beyond review.",
        result_commons_not_distributed: "Content deleted and excluded from future exports.",
        result_commons_distributed: concat!(
            "Content deleted from managed stores. Previously distributed copies cannot be ",
            "recalled."
        ),
        result_unknown: concat!(
            "Withdrawal completed. Distribution reach is unavailable. If copies were already ",
            "distributed, they cannot be recalled."
        ),
        // Approved 2026-10-07. Sent before any request, when the stored
        // session is absent or expired (`daemon/withdraw.rs`), so it never
        // claims a server answer.
        account_session_required: concat!(
            "You're not signed in, or your sign-in has expired. Nothing was withdrawn. ",
            "Sign in again to retry."
        ),
        not_found: concat!(
            "Nothing was withdrawn and nothing was deleted. There is no trace with that id ",
            "under your account."
        ),
        not_found_labels: &["not-found", "not_found", "submission-not-found"],
        failed: "Withdrawal failed. Nothing was withdrawn and nothing was deleted.",
        try_again: "Try again",
        no_bulk_action: "Withdraw traces individually to see the result for each one.",
        wording_defect: "Do not trust the withdrawal wording on this screen.",
    }
}

// ---------------------------------------------------------------------------
// Public profile

/// The public profile's words: the Settings section, the go-public dialog,
/// and what a claim or a withdrawal did.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct PublicProfileWords {
    pub heading: &'static str,
    pub list_handle_publicly: &'static str,
    /// Cut: empty. Approved by the owner 2026-11-04 (Settings copy pass 2, P142).
    pub footnote: &'static str,
    pub handle: &'static str,
    pub bio: &'static str,
    /// Saves an edit to a profile already on the roster.
    pub update_profile: &'static str,
    /// Takes the handle off the roster.
    pub withdraw: &'static str,
    /// Withdraw's accessible name, naming what it withdraws, apart from a
    /// trace's Withdraw (copy-guide rule 5). DRAFT, NEEDS APPROVAL
    /// (2026-11-04).
    pub withdraw_accessibility_name: &'static str,
    /// `{date}`: when the handle went public.
    pub on_roster_since: &'static str,
    pub go_public_headline: &'static str,
    /// Approved by the owner 2026-11-04 (Settings copy pass 2, P150).
    pub go_public_description: &'static str,
    /// The dialog's confirm button.
    pub go_public: &'static str,
    /// The card's button that opens the dialog: the same words as the
    /// confirm button, its own key (copy-guide rule 4).
    pub go_public_open: &'static str,
    pub going_public: &'static str,
    pub not_now: &'static str,
    pub published_heading: &'static str,
    /// Approved by the owner 2026-11-04 (Settings copy pass 2, P159).
    pub published_lines: &'static [&'static str],
    pub never_heading: &'static str,
    /// Approved by the owner 2026-11-04 (Settings copy pass 2, P151).
    pub never_lines: &'static [&'static str],
    pub acknowledgement: &'static str,
    /// Cut: empty. Approved by the owner 2026-11-04 (Settings copy pass 2, P144).
    pub go_public_footnote: &'static str,
    pub go_public_handle: &'static str,
    pub go_public_bio: &'static str,
    /// A claim the server accepted.
    pub published: &'static str,
    /// A claim the server accepted that this device could not write down:
    /// public all the same. Approved by the owner 2026-11-04 (Settings copy pass 2, P145).
    pub published_not_cached: &'static str,
    pub left_roster: &'static str,
    pub left_roster_not_cached: &'static str,
    /// A refused claim: `{reason}` from [`Self::failure_reasons`], or
    /// [`Self::failure_default`] whole for a label with no reason.
    pub failure: &'static str,
    pub failure_default: &'static str,
    /// The daemon's refusal labels, each to its reason. Never echoes the
    /// label itself: an unknown one gets [`Self::failure_default`].
    pub failure_reasons: BTreeMap<&'static str, &'static str>,
    /// A refused withdrawal: still on the roster. `{reason}` is
    /// [`Self::leave_not_connected`] for `not-logged-in`, else empty.
    pub leave_failure: &'static str,
    pub leave_not_connected: &'static str,
    /// The defect notice when the shell's profile checks fail.
    pub wording_defect: &'static str,
}

#[must_use]
pub fn public_profile_words() -> PublicProfileWords {
    // Approved by the owner 2026-11-04 (Settings copy pass 2, P165). Approved by the owner 2026-11-04 (Settings copy pass 2, P166).
    let not_connected = "This computer isn't connected.";
    PublicProfileWords {
        heading: "Public profile",
        list_handle_publicly: "List my handle publicly",
        footnote: "",
        handle: "Handle",
        bio: "Bio",
        // Approved by the owner 2026-11-04 (Settings copy pass 2, P162).
        update_profile: "Save",
        withdraw: "Withdraw",
        withdraw_accessibility_name: "Withdraw public profile",
        on_roster_since: "On the roster since {date}",
        go_public_headline: "Put your handle on the public roster?",
        go_public_description: "Going public grants no use of your traces.",
        go_public: "Go public",
        go_public_open: "Go public",
        going_public: "Going public\u{2026}",
        not_now: "Not now",
        // Approved by the owner 2026-11-04 (Settings copy pass 2, P158).
        published_heading: "Published",
        published_lines: &["Your handle, total counts, public date and bio."],
        // Approved by the owner 2026-11-04 (Settings copy pass 2, P163).
        never_heading: "Never published",
        never_lines: &["Your traces or anything about a single trace."],
        // Approved by the owner 2026-11-04 (Settings copy pass 2, P160).
        acknowledgement: concat!(
            "I understand my handle and counts become public. Leaving removes me from ",
            "future snapshots only."
        ),
        go_public_footnote: "",
        // Approved by the owner 2026-11-04 (Settings copy pass 2, P156).
        go_public_handle: "Handle",
        // Approved by the owner 2026-11-04 (Settings copy pass 2, P147).
        go_public_bio: "Bio (optional)",
        // Approved by the owner 2026-11-04 (Settings copy pass 2, P152).
        published: "Your handle and counts are public now.",
        published_not_cached: "Profile is public, but may not show here after a restart.",
        // Approved by the owner 2026-11-04 (Settings copy pass 2, P143).
        left_roster: "You've left the roster.",
        // Approved by the owner 2026-11-04 (Settings copy pass 2, P141).
        left_roster_not_cached: concat!(
            "You've left the roster. This computer may still show your old handle for a ",
            "while."
        ),
        failure: "{reason} Profile was not published.",
        // Approved by the owner 2026-11-04 (Settings copy pass 2, P149).
        failure_default: "Profile was not published.",
        failure_reasons: BTreeMap::from([
            ("handle-required", "Handle is required."),
            ("handle-too-short", "Handle must be at least 3 characters."),
            ("handle-too-long", "Handle must be 32 characters or fewer."),
            // Approved by the owner 2026-11-04 (Settings copy pass 2, P157).
            (
                "handle-invalid-character",
                "Use letters, numbers, hyphens or underscores.",
            ),
            // Approved by the owner 2026-11-04 (Settings copy pass 2, P153).
            (
                "handle-invalid-boundary",
                "Start and end with a letter or number.",
            ),
            // Approved by the owner 2026-11-04 (Settings copy pass 2, P161).
            (
                "handle-consecutive-separators",
                "Don't put two hyphens or underscores together.",
            ),
            ("handle-reserved", "That handle is reserved."),
            ("bio-too-long", "Bio must be 280 bytes or fewer."),
            // Approved by the owner 2026-11-04 (Settings copy pass 2, P164).
            (
                "bio-invalid-character",
                "Bio contains an unsupported character.",
            ),
            // Approved by the owner 2026-11-04 (Settings copy pass 2, P154).
            ("bio-required-or-null", "The roster didn't accept the bio."),
            // Approved by the owner 2026-11-04 (Settings copy pass 2, P155).
            ("bio-invalid", "The roster didn't accept the bio."),
            ("not-logged-in", not_connected),
        ]),
        // Approved by the owner 2026-11-04 (Settings copy pass 2, P148).
        leave_failure: "{reason}Couldn't withdraw. You're still on the roster.",
        leave_not_connected: not_connected,
        wording_defect: "Do not trust the public-profile wording on this screen.",
    }
}

// ---------------------------------------------------------------------------
// The queue, History and the scrubbing caveat

/// The Traces tab's words the legacy queue held (`QueueLegacyWords`).
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct QueueWords {
    pub nothing_waiting: &'static str,
    pub nothing_waiting_detail: &'static str,
    pub undo_will_send: &'static str,
    pub close_notice_still_sends: &'static str,
    pub close_notice: &'static str,
    pub not_offered_scope: &'static str,
    pub undo: &'static str,
    pub agent_setup: &'static str,
    /// `{seconds}` since the approval, counting up.
    pub approved_ago: &'static str,
    /// The count has reached the ceiling: `{seconds}` is the ceiling.
    pub approved_ago_ceiling: &'static str,
    pub no_longer_waiting: &'static str,
}

#[must_use]
pub fn queue_words() -> QueueWords {
    QueueWords {
        nothing_waiting: "Nothing is waiting.",
        nothing_waiting_detail: concat!(
            "When a trace finishes and goes quiet, it shows up here. Nothing is sent unless ",
            "you say so."
        ),
        undo_will_send: "Approved traces will send automatically. You can undo until uploading starts.",
        close_notice_still_sends: "Close this notice. Approved traces will still send automatically.",
        close_notice: "Close this notice.",
        not_offered_scope: concat!(
            "This covers traces that reached the queue. Traces that were never queued at ",
            "all are not counted here."
        ),
        undo: "Undo",
        agent_setup: "Agent setup",
        approved_ago: "Approved {seconds}s ago",
        approved_ago_ceiling: "Approved {seconds}s+ ago",
        no_longer_waiting: "Traces no longer waiting ({count})",
    }
}

/// History's words the legacy screen held (`HistoryLegacyWords`).
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct HistoryWords {
    pub typical_wait: &'static str,
}

#[must_use]
pub fn history_words() -> HistoryWords {
    HistoryWords {
        typical_wait: "Typical wait: we don't have a reliable number yet.",
    }
}

/// The limits of automatic scrubbing (`ScrubbingCaveat.swift`). The
/// canonical sentence is the shared design spec's, unchanged; it is shown
/// under the list and again, verbatim, against Contribute.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ScrubbingWords {
    pub canonical: &'static str,
    /// A session where no pattern matched: the one most worth a look.
    pub nothing_matched: &'static str,
    pub removed: &'static str,
    /// Leads the canonical sentence's accessible name at Contribute.
    pub before_you_contribute: &'static str,
}

#[must_use]
pub fn scrubbing_words() -> ScrubbingWords {
    ScrubbingWords {
        canonical: "Scrubbing is pattern-based. It misses things it hasn't seen before.",
        nothing_matched: concat!(
            "Nothing matched a pattern. That is not the same as nothing being there -- search ",
            "it for anything you need to be sure isn't in it."
        ),
        removed: "Removed by pattern matching. Anything the patterns don't know is still in there.",
        before_you_contribute: "Before you contribute.",
    }
}

// ---------------------------------------------------------------------------
// Settings

/// The Settings sections' sentences the legacy screen held
/// (`SettingsLegacyWords`).
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct SettingsWords {
    pub consent_heading: &'static str,
    pub connected: &'static str,
    pub not_connected: &'static str,
    /// Approved by the owner 2026-11-04 (Settings copy pass 2, P26).
    pub queued_nothing_sent: &'static str,
    pub extra_scan_configured: &'static str,
    pub session_finished_after: &'static str,
    /// Cut: empty. The notification schedule's own help
    /// (`nudge_copy::SETTING_DIGEST_HELP_INTERVAL`) says it.
    /// Approved by the owner 2026-11-04 (Settings copy pass 2, P235).
    pub at_most_one_notification: &'static str,
    /// [`Self::at_most_one_notification`] at one hour. Cut: empty.
    /// Approved by the owner 2026-11-04 (Settings copy pass 2, P236).
    pub at_most_one_notification_one: &'static str,
    pub undecided_dropped: &'static str,
    /// A state row's accessible name: `{title}` then the value.
    pub state_yes: &'static str,
    pub state_no: &'static str,
    pub waiting_on_approval: &'static str,
    pub turn_on_in_system_settings: &'static str,
    pub could_not_turn_on: &'static str,
    pub could_not_turn_off: &'static str,
    pub version: &'static str,
    pub check_now: &'static str,
    pub copy: &'static str,
    pub checks_daily: &'static str,
    pub checks_automatically: &'static str,
    pub managed_by_homebrew: &'static str,
    pub homebrew_replaces: &'static str,
    pub updates_unavailable: &'static str,
    pub not_checked_yet: &'static str,
    pub last_checked: &'static str,
    pub no_feed: &'static str,
    pub insecure_feed: &'static str,
    pub updates_off: &'static str,
    /// Cut: empty. Approved by the owner 2026-11-04 (Settings copy pass 2, P244).
    pub notifications_rendered_here: &'static str,
    pub paused_nothing_sent: &'static str,
    /// Approved by the owner 2026-11-04 (Settings copy pass 2, P138).
    pub applies_from_now: &'static str,
    pub always_included: &'static str,
    /// The optional scopes' heading. Approved 2026-10-07: it was "Optional \u{2014} each one lets your traces do
    /// more".
    pub optional_data_use: &'static str,
    pub reward_opportunities: &'static str,
    /// The tooltip on the Train coding models row, beside [`Self::reward_opportunities`].
    /// Approved by the owner 2026-11-04 (Settings copy pass 2, P137).
    pub reward_opportunities_tooltip: &'static str,
    pub credit: &'static str,
    /// The tag on the always-on scope: the core's one spelling
    /// (`consent_copy::SCOPE_TAG_REQUIRED`, "always on"), matching the
    /// first run's "Always on" line, so the two never differ.
    pub required: &'static str,
    pub nothing_preselected: &'static str,
    pub nothing_changed: &'static str,
    /// A folder's mode change the daemon refused: the folder's setting did
    /// not change. Approved by the owner 2026-11-04 (Settings copy pass 2, P136).
    pub project_mode_failed: &'static str,
    /// The change log: each fixed action label to its sentence, which the
    /// shell follows with the project's name when there is one. An action
    /// this build does not know still gets [`Self::audit_changed`].
    pub audit_actions: BTreeMap<&'static str, &'static str>,
    pub audit_changed: &'static str,
}

#[must_use]
pub fn settings_words() -> SettingsWords {
    SettingsWords {
        consent_heading: "Data uses",
        connected: "Connected",
        not_connected: "Not connected",
        queued_nothing_sent: "Nothing is sent until you set up this app.",
        extra_scan_configured: "Extra privacy scan configured",
        session_finished_after: "A trace counts as finished after {seconds} seconds of quiet.",
        at_most_one_notification: "",
        at_most_one_notification_one: "",
        // Approved by the owner 2026-11-04 (Settings copy pass 2, P86).
        undecided_dropped: "Undecided traces are dropped after {days} days and never sent.",
        state_yes: "{title}: yes",
        state_no: "{title}: no",
        // Approved by the owner 2026-11-04 (Settings copy pass 2, P256).
        waiting_on_approval: "Waiting for approval in System Settings.",
        // Approved by the owner 2026-11-04 (Settings copy pass 2, P243).
        turn_on_in_system_settings: "Turn it on in System Settings > General > Login Items.",
        could_not_turn_on: "Couldn't turn this on: {message}",
        could_not_turn_off: "Couldn't turn this off: {message}",
        version: "Version",
        check_now: "Check Now",
        copy: "Copy",
        checks_daily: "Checks daily",
        // Approved by the owner 2026-11-04 (Settings copy pass 2, P259).
        checks_automatically: "Asks before installing an update.",
        managed_by_homebrew: "Updates managed by Homebrew",
        // Approved by the owner 2026-11-04 (Settings copy pass 2, P258).
        homebrew_replaces: "To update, run this in a terminal:",
        updates_unavailable: "Updates unavailable",
        // Approved by the owner 2026-11-04 (Settings copy pass 2, P260).
        not_checked_yet: "Not checked yet.",
        last_checked: "Last checked {relative}.",
        // Approved by the owner 2026-11-04 (Settings copy pass 2, P257).
        no_feed: "This build doesn't check for updates. Install a release DMG to get them.",
        // Approved by the owner 2026-11-04 (Settings copy pass 2, P261).
        insecure_feed: concat!(
            "This build's update feed isn't secure, so it's refused. Reinstall from a ",
            "release DMG."
        ),
        updates_off: "Updates are turned off for this build.",
        notifications_rendered_here: "",
        paused_nothing_sent: PAUSED_NOTHING_SENT,
        applies_from_now: "Changes apply to traces sent from now on.",
        always_included: "Always included",
        optional_data_use: "Optional data use",
        reward_opportunities: crate::consent_copy::DATA_USE_REWARD_OPPORTUNITIES,
        reward_opportunities_tooltip: crate::consent_copy::DATA_USE_REWARD_OPPORTUNITIES_TOOLTIP,
        credit: "Credit",
        required: crate::consent_copy::SCOPE_TAG_REQUIRED,
        nothing_preselected: "Nothing here is pre-selected on your behalf.",
        // Approved by the owner 2026-11-04 (Settings copy pass 2, P169).
        nothing_changed: "No changes yet.",
        // Approved by the owner 2026-11-04 (Settings copy pass 2, P136).
        project_mode_failed: "Couldn't change this folder's setting.",
        audit_actions: BTreeMap::from([
            // Approved by the owner 2026-11-04 (Settings copy pass 2, P170).
            ("armed-auto-upload", "Automatic turned on for"),
            // Approved by the owner 2026-11-04 (Settings copy pass 2, P171).
            ("disarmed-auto-upload", "Automatic turned off for"),
            // Approved by the owner 2026-11-04 (Settings copy pass 2, P167).
            ("queue-bulk-approved", "Whole queue approved"),
            // Approved by the owner 2026-11-04 (Settings copy pass 2, P172).
            ("consent-scopes-changed", "Data uses changed"),
            // Approved by the owner 2026-11-04 (Settings copy pass 2, P168).
            (
                "near-ai-notice-acknowledged",
                "Extra privacy scan confirmed",
            ),
        ]),
        audit_changed: "Changed",
    }
}

/// Uploads are paused by the person. One sentence, read by Settings
/// (`SettingsWords::paused_nothing_sent`) and by the Traces tab's resume
/// notice (`PausedNoticeWords::body`).
pub const PAUSED_NOTHING_SENT: &str = "Paused. Nothing is being queued or sent.";

// ---------------------------------------------------------------------------
// The Traces tab's resume notice

/// The notice the Traces tab shows each time it is opened while the person
/// has paused uploads, and only then: never while uploads run, nor while
/// they are held for another reason (the account needs attention, automatic
/// contribution is held, or this device is not enrolled). The notice and
/// its words were approved by the owner 2026-11-04.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct PausedNoticeWords {
    /// [`PAUSED_NOTHING_SENT`], the same sentence Settings says.
    pub body: &'static str,
    /// Approved 2026-11-04. Resumes uploads (the daemon's `resume`).
    pub resume: &'static str,
    /// Approved 2026-11-04. Hides the notice until the Traces tab is next
    /// opened.
    pub not_now: &'static str,
}

#[must_use]
pub fn paused_notice_words() -> PausedNoticeWords {
    PausedNoticeWords {
        body: PAUSED_NOTHING_SENT,
        resume: "Resume",
        not_now: "Not now",
    }
}

// ---------------------------------------------------------------------------
// The table

/// Every table above, as the one object `tc_shell_words_copy_json` returns.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ShellWordsCopy {
    pub withdrawal: WithdrawalWords,
    pub public_profile: PublicProfileWords,
    pub queue: QueueWords,
    pub history: HistoryWords,
    pub scrubbing: ScrubbingWords,
    pub settings: SettingsWords,
    pub paused_notice: PausedNoticeWords,
}

#[must_use]
pub fn shell_words_copy() -> ShellWordsCopy {
    ShellWordsCopy {
        withdrawal: withdrawal_words(),
        public_profile: public_profile_words(),
        queue: queue_words(),
        history: history_words(),
        scrubbing: scrubbing_words(),
        settings: settings_words(),
        paused_notice: paused_notice_words(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The resume notice says Settings' own paused sentence, not a copy of
    /// it, with the two labels approved 2026-11-04.
    #[test]
    fn the_paused_notice_reuses_the_settings_sentence() {
        let copy = shell_words_copy();
        assert_eq!(copy.paused_notice.body, copy.settings.paused_nothing_sent);
        assert_eq!(
            copy.paused_notice.body,
            "Paused. Nothing is being queued or sent."
        );
        assert_eq!(copy.paused_notice.resume, "Resume");
        assert_eq!(copy.paused_notice.not_now, "Not now");
    }

    fn strings(value: &serde_json::Value, out: &mut Vec<String>) {
        match value {
            serde_json::Value::String(text) => out.push(text.clone()),
            serde_json::Value::Array(items) => items.iter().for_each(|v| strings(v, out)),
            serde_json::Value::Object(map) => map.values().for_each(|v| strings(v, out)),
            _ => {}
        }
    }

    #[test]
    fn no_string_is_empty_and_every_hole_is_known() {
        let mut value = serde_json::to_value(shell_words_copy()).unwrap();
        // Owner dropped these lines 2026-11-04 (Settings copy pass 2, P142,
        // P144, P235, P236, P244): each is empty, and the only words allowed
        // to be. The fields stay because the shells decode them.
        for pointer in [
            "/public_profile/footnote",
            "/public_profile/go_public_footnote",
            "/settings/at_most_one_notification",
            "/settings/at_most_one_notification_one",
            "/settings/notifications_rendered_here",
        ] {
            let slot = value.pointer_mut(pointer).expect(pointer);
            assert_eq!(slot.as_str(), Some(""), "{pointer} was cut");
            *slot = serde_json::Value::Null;
        }
        let mut all = Vec::new();
        strings(&value, &mut all);
        assert!(
            all.len() > 100,
            "the table is not being read: {}",
            all.len()
        );
        for text in &all {
            assert!(!text.trim().is_empty(), "an empty string in the table");
            let mut rest = text.as_str();
            while let Some(open) = rest.find('{') {
                let close = rest[open..].find('}').expect("a hole closes") + open;
                let name = &rest[open + 1..close];
                assert!(
                    SHELL_WORDS_PLACEHOLDERS.contains(&name),
                    "unknown hole {{{name}}} in {text:?}"
                );
                rest = &rest[close + 1..];
            }
        }
    }

    #[test]
    fn the_canonical_withdrawal_bodies_keep_their_tiers() {
        let w = withdrawal_words();
        assert!(
            w.commons_distributed
                .contains("copies that have already been distributed cannot be recalled")
        );
        assert!(w.commons_distributed.contains("does not undo that"));
        assert!(!w.not_distributed.contains("excludes it from everything"));
        assert!(
            w.commons_not_distributed
                .contains("has not been included in any published export")
        );
        // The core's own prompts end with the same credit sentence.
        assert!(
            crate::withdraw::confirmation_prompt_unknown().ends_with(w.credit_note),
            "the unknown-reach prompt and the table disagree about credit"
        );
    }

    /// Each tier's result, as approved (2026-10-07). A per-tier pin rather
    /// than only "the outcomes differ": a softened or swapped sentence
    /// passes distinctness and fails here.
    #[test]
    fn each_withdrawal_result_is_its_tiers_approved_sentence() {
        let w = withdrawal_words();
        assert_eq!(
            w.result_not_distributed,
            "Content deleted. It was never shared beyond review."
        );
        assert_eq!(
            w.result_commons_not_distributed,
            "Content deleted and excluded from future exports."
        );
        assert_eq!(
            w.result_commons_distributed,
            "Content deleted from managed stores. Previously distributed copies cannot be recalled."
        );
        assert_eq!(
            w.result_unknown,
            "Withdrawal completed. Distribution reach is unavailable. If copies were already \
             distributed, they cannot be recalled."
        );
        // And each still agrees with its tier's canonical body: the furthest
        // tier is never reported as recallable, the middle one keeps the
        // exclusion its body promises, and the nearest claims neither.
        assert!(w.commons_distributed.contains("cannot be recalled"));
        assert!(w.result_commons_distributed.contains("cannot be recalled"));
        assert!(
            w.commons_not_distributed
                .contains("excludes it from everything")
        );
        assert!(
            w.result_commons_not_distributed
                .contains("excluded from future exports")
        );
        assert!(w.not_distributed.contains("Nothing was distributed"));
        for claim in ["excluded", "recalled", "distributed"] {
            assert!(
                !w.result_not_distributed.contains(claim),
                "{claim:?} in the not-distributed result"
            );
        }
    }

    #[test]
    fn a_withdrawal_result_never_claims_more_than_its_tier() {
        let w = withdrawal_words();
        let results = [
            w.result_not_distributed,
            w.result_commons_not_distributed,
            w.result_commons_distributed,
            w.result_unknown,
        ];
        // Never a generic "withdrawn": each tier reads differently.
        for (i, a) in results.iter().enumerate() {
            for b in &results[i + 1..] {
                assert_ne!(a, b);
            }
        }
        // The furthest tier, and the unknown one, never imply a recall.
        assert!(w.result_commons_distributed.contains("cannot be recalled"));
        assert!(w.result_unknown.contains("cannot be recalled"));
        assert!(!w.result_not_distributed.contains("excluded"));
    }

    /// Every did-not-happen line says "Nothing was withdrawn" as a sentence
    /// of its own: at the start, or right after a full stop.
    #[test]
    fn a_withdrawal_that_did_not_happen_says_so_in_its_own_sentence() {
        let w = withdrawal_words();
        for sentence in [w.account_session_required, w.not_found, w.failed] {
            assert!(
                sentence.starts_with("Nothing was withdrawn")
                    || sentence.contains(". Nothing was withdrawn"),
                "{sentence:?} does not say, in a sentence of its own, that nothing happened"
            );
        }
        let lower = w.not_found.to_lowercase();
        assert!(!lower.contains("belongs to") && !lower.contains("does not exist"));
    }

    /// The signed-out line is sent before any request, when the stored
    /// session is absent or expired (`daemon/withdraw.rs`), so Commons never
    /// saw it; a real refusal comes back as `withdraw-failed`. Neither line
    /// claims a server answer.
    #[test]
    fn a_withdrawal_refused_here_never_claims_a_server_answer() {
        let w = withdrawal_words();
        assert_eq!(
            w.account_session_required,
            "You're not signed in, or your sign-in has expired. Nothing was withdrawn. \
             Sign in again to retry."
        );
        for sentence in [w.account_session_required, w.failed] {
            let lower = sentence.to_lowercase();
            for claim in ["rejected", "refused", "server", "commons", "declined"] {
                assert!(!lower.contains(claim), "{claim:?} in {sentence:?}");
            }
        }
    }

    #[test]
    fn a_published_profile_is_never_reported_as_a_failure() {
        let p = public_profile_words();
        for sentence in [p.published, p.published_not_cached] {
            let lower = sentence.to_lowercase();
            assert!(
                lower.contains("public"),
                "{sentence:?} does not say it is public"
            );
            for forbidden in [
                "couldn't publish",
                "failed",
                "wasn't published",
                "not published",
            ] {
                assert!(
                    !lower.contains(forbidden),
                    "{sentence:?} reads as a failure"
                );
            }
        }
        assert_ne!(p.published, p.published_not_cached);
        for sentence in [p.left_roster, p.left_roster_not_cached] {
            assert!(sentence.starts_with("You've left the roster"));
        }
    }

    #[test]
    fn a_refused_profile_change_says_what_did_not_happen() {
        let p = public_profile_words();
        assert!(p.failure_default.contains("was not published"));
        assert!(p.failure.contains("was not published"));
        for reason in p.failure_reasons.values() {
            assert!(!reason.contains("https://"));
        }
        // "Not published" is false comfort after a failed withdrawal.
        assert!(!p.leave_failure.contains("not published"));
        assert!(p.leave_failure.contains("still on the roster"));
    }
}
