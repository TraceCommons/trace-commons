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

/// The concession, in full. Shown once per decision, in the preview sheet,
/// where a person is reading rather than scanning.
pub const RESIDUAL_RISK: &str =
    "Scrubbing is pattern-based. It misses things it hasn't seen before.";

/// The same concession on a queue row, said in terms of what scrubbing
/// actually did to *this* session.
///
/// The constant above used to be printed verbatim on every card. Repeated
/// down a column, identical each time, it stops being read -- which is how
/// a warning becomes wallpaper, and the warning it becomes is the one this
/// product most needs someone to take seriously. Splitting it across
/// several places only makes several pieces of wallpaper.
///
/// So the row carries a line that changes with the count. "Scrubbing
/// matched nothing" and "scrubbing removed 4 things" are different
/// sentences describing different situations, and a person reads the second
/// one because it is not the one they read on the card above. The zero case
/// is also the one worth weighing -- a session that obviously touched a
/// `.env` and reports nothing removed is a signal -- so it is the case that
/// carries the attention tone and, on the card, the gold rule.
///
/// The full sentence is never dropped: it is restated in the preview sheet
/// under "Residual risk", which is the screen a person is on when they
/// actually decide.
pub fn residual_risk_line(total_redactions: u32) -> String {
    match total_redactions {
        0 => "Scrubbing matched nothing here. That is not the same as there being nothing to \
              find -- it only recognises patterns it has seen before."
            .to_string(),
        1 => "Scrubbing removed 1 thing it recognised. It works from patterns, so it misses \
              what it hasn't seen before."
            .to_string(),
        n => format!(
            "Scrubbing removed {n} things it recognised. It works from patterns, so it misses \
             what it hasn't seen before."
        ),
    }
}
pub const LOOK_INSIDE: &str = "Look inside";
pub const NOT_THIS_ONE: &str = "Not this one";
pub const NOT_THIS_ONE_TOOLTIP: &str =
    "Skips this session only. This project will keep being offered.";
pub const QUEUE_EMPTY_TITLE: &str = "Nothing waiting";
pub const QUEUE_EMPTY_BODY: &str = "When a session finishes and goes quiet, it shows up here. \
     Nothing is sent unless you say so.";
pub const CHECKING: &str = "Checking what would be sent…";

/// The standing concession, under the column rather than on every card.
/// Distinct on purpose from [`residual_risk_line`], which says what
/// scrubbing did to *one* session: this one says what scrubbing *is*.
pub const STANDING_DISCLAIMER: &str = "Scrubbing is local and pattern-based. It is good and it is \
     not perfect -- which is why you look before anything is sent.";

/// The manifest pair labels, §5.1 item 6. "Removed by pattern" names the
/// mechanism's *limit* in the label itself, which is the point: it is what
/// pattern matching found, not what was in there.
pub const WOULD_SEND: &str = "Would send";
pub const REMOVED_BY_PATTERN: &str = "Removed by pattern";

/// §6.2's attention chip, in both places it is earned: a session where
/// scrubbing removed nothing, and a search that found nothing. Neither is a
/// reassurance, which is why they share a wording that concedes rather than
/// one that congratulates.
pub const NOTHING_MATCHED: &str = "nothing matched";

/// The eyebrow over the count of things that did go out this week.
pub const CONTRIBUTED: &str = "Contributed";

/// The week band's heading.
pub const THIS_WEEK: &str = "This week";

pub fn waiting_heading(waiting: usize) -> String {
    match waiting {
        1 => "1 session waiting for your decision".to_string(),
        n => format!("{n} sessions waiting for your decision"),
    }
}

pub fn no_longer_waiting(count: u64) -> String {
    format!("Sessions no longer waiting ({count})")
}

/// The bound on what [`no_longer_waiting`] can account for, stated rather
/// than left to be assumed.
///
/// `queue_outcome_counts` counts entries that reached the queue. It cannot
/// explain a session the watcher discarded before an entry existed -- an
/// ineligible verdict, or a project set to be ignored -- and a contributor
/// who read this list as complete would come away believing sessions had
/// been accounted for that were never counted at all.
pub const NOT_OFFERED_BOUND: &str = "This covers sessions that reached the queue. Sessions that were never queued at all are not \
     counted here.";

// --- Preview -----------------------------------------------------------

/// The tab, which names a place. [`SEARCH_SUBMIT`] is the button, which
/// names an action; they are the same word today and are not the same
/// string, because only one of them is a verb.
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

/// The sheet's title, before the project label the call site appends.
pub const SHEET_TITLE_PREFIX: &str = "Look inside";

/// §6.2's locked chip, and the sentence beside it. Both say the same thing
/// the whole sheet exists to say: this is a rehearsal.
pub const NOTHING_SENT_YET: &str = "nothing sent yet";
pub const NOTHING_SENT_REASSURANCE: &str = "Nothing has been sent. This is what would be.";

/// The search button. See [`TAB_SEARCH`].
pub const SEARCH_SUBMIT: &str = "Search";
pub const RECENT_LABEL: &str = "Recent:";

/// What an empty search result says. A search that found nothing is not
/// evidence that nothing is there, and this is where that is said rather
/// than implied.
pub const NOTHING_MATCHED_BODY: &str = "A search only finds what is written the way you typed it. \
     If it matters, try the other spellings you would worry about -- a hostname, an internal \
     code name, an address.";

pub const TRANSCRIPT_CAPTION: &str = "These are the exact bytes an approval covers. Marks like \
     <PRIVATE_SECRET_1> show where scrubbing fired -- legible as chips, not holes.";

/// The read gate's two halves and its footnote. The footnote concedes what
/// the gate cannot check, because a gate that overstated what it verified
/// would be worse than no gate.
pub const GATE_OPENED: &str = "You have opened \"Exactly what would be sent\".";
pub const GATE_ACKNOWLEDGED: &str = "I have looked at what would be sent, and I understand \
     scrubbing is pattern-based and may have missed something.";
pub const GATE_FOOTNOTE: &str = "Contribute stays off until both are done. Looking at the first \
     screen is what this checks -- it cannot check that you read all of it, and it does not \
     claim to.";

pub const CLOSE: &str = "Close";

// --- Approving ---------------------------------------------------------

pub const SENDING: &str = "Sending…";
pub const UNDO: &str = "Undo";
/// Used when the daemon reports no hold, so no undo may be offered.
pub const APPROVED_NO_UNDO: &str = "Approved. It goes out on the next pass.";

pub fn undo_headline(project_label: &str) -> String {
    format!("Approved {project_label}. Still on this machine.")
}

/// The undo bar's body. The Linux wording, which drops the shared spec's
/// middle clause ("This app cannot see when that lands, so it does not
/// pretend to count it down") because the bar has less room and the
/// remaining sentence already makes the promise the clause was defending.
pub const UNDO_BODY: &str = "The watcher sends approved sessions on its next sweep. Undo works \
     until the sweep starts, and says so plainly if it is already too late.";

/// The other half of the undo bar's pair. Not "Dismiss": what this button
/// does is let the send happen, and it should say so.
pub const LET_IT_SEND: &str = "Let it send";

// --- Credit ------------------------------------------------------------

pub const CREDIT_HEADING: &str = "About credit";
/// §5.3's eyebrow over the credit card. [`CREDIT_HEADING`] reads "About
/// credit", which titles a paragraph rather than labelling a figure; the
/// section rule beneath it wants the shorter word.
pub const CREDIT_SECTION: &str = "Credit";
pub const CREDIT_BODY: &str = "Contributions earn credit points, scored on how novel and \
     information-rich a trace is. Today credit is a record, not a currency: there is no payout, \
     no token, no exchange rate, and no date. The intent is that credit eventually settles to \
     something real, and if it does it will settle from this record. Contribute because you want \
     the commons to exist.";
pub const NOT_SYNCED_YET: &str = "Not synced yet";

// --- History -----------------------------------------------------------

pub const HISTORY_IN_THE_COMMONS: &str = "In the commons";
pub const HISTORY_WAITING_TO_BE_SCORED: &str = "Waiting to be scored";

/// §5.3's section heading over the record rows.
pub const EVERYTHING_CONTRIBUTED: &str = "Everything you've contributed";

/// §5.3's chip on a withdrawn record. The record stays on the list and
/// reads as withdrawn (§7.3); it is never dropped and never re-labelled as
/// something that failed.
pub const WITHDRAWN_BY_YOU: &str = "Withdrawn by you";

/// §5.3's row-level explanation on a held record, used only when the server
/// sent no explanation of its own. It says the same three things
/// [`QUARANTINE_BODY`] says -- automated, not rejected, not shared -- at row
/// length rather than at section length.
pub const HELD_ROW_BODY: &str = "Automated checks saw something that might be personal and \
     couldn't decide on their own. It has not been rejected, and it has not been shared with \
     anyone but the reviewer.";
pub const QUARANTINE_HEADING: &str = "Held for privacy review";
pub const QUARANTINE_BODY: &str = "A person at Trace Commons reads these before they enter the \
     commons. It happens when automated checks see something that might be personal or sensitive \
     and can't decide on its own.\n\nThese have not been rejected, and they have not been shared \
     with anyone but the reviewer. They are sitting still.\n\nTypical wait: we don't have a \
     reliable number yet.";

// --- Withdrawal --------------------------------------------------------
//
// Withdrawal is the one place in this product where a plausible-sounding
// phrase becomes a false promise about erasure, so the three confirmation
// bodies are NOT this shell's to write. They are fixed in
// `docs/contributor-daemon-ipc-v1_1.md`'s "Canonical confirmation copy"
// table, reproduced here word for word, and the tests at the foot of this
// file fail if they are paraphrased, shortened, or "tightened".
//
// Five rules come with them, and each is honoured somewhere below:
//
// 1. Never a generic "withdrawn" -- [`withdraw_result_sentence`] always
//    names what the tier that actually applied did.
// 2. Never claim more erasure than the tier achieved -- which is why
//    [`withdraw_confirmation`] shows an `accepted` trace BOTH commons
//    bodies rather than picking the gentler one.
// 3. Withdrawal does not reverse settled credit -- [`WITHDRAW_CREDIT_NOTE`],
//    and nothing here implies otherwise.
// 4. `not_found` must not disclose which -- [`WITHDRAW_NOT_FOUND`].
// 5. Bulk withdrawal spans tiers -- [`WITHDRAW_NO_BULK`] says why this
//    shell does not offer it.
//
// ## Why the confirmation cannot simply state the tier
//
// The server computes `distribution_reach` *during* the withdrawal, from
// live export membership. It arrives in the response, and the confirmation
// has to be shown before that response exists. All this machine holds is
// the record's `status`, so the confirmation is keyed on that instead --
// see [`WithdrawStage`].

/// `distribution_reach` as the server spells it. Wire strings rather than a
/// typed enum: the shell only ever looks a tier up to find its sentence,
/// and an unrecognised one is reported as unrecognised (see
/// [`withdraw_result_sentence`]) rather than failing to parse.
pub const REACH_NOT_DISTRIBUTED: &str = "not_distributed";
pub const REACH_COMMONS_NOT_DISTRIBUTED: &str = "commons_not_distributed";
pub const REACH_COMMONS_DISTRIBUTED: &str = "commons_distributed";

/// Canonical copy for `not_distributed`, verbatim.
pub const WITHDRAW_BODY_NOT_DISTRIBUTED: &str = "This trace never entered the commons. Withdrawing deletes it. Nothing was distributed and \
     nothing needs recalling.";

/// Canonical copy for `commons_not_distributed`, verbatim.
pub const WITHDRAW_BODY_COMMONS_NOT_DISTRIBUTED: &str = "This trace is in the commons but has not been included in any published export or benchmark \
     yet. Withdrawing deletes it and excludes it from everything published from here on.";

/// Canonical copy for `commons_distributed`, verbatim. The clause from "but
/// copies" onward is the one sentence in this feature that must never be
/// softened, shortened, or quietly dropped.
pub const WITHDRAW_BODY_COMMONS_DISTRIBUTED: &str = "This trace has already been included in a published export or benchmark. Withdrawing \
     deletes our copy and excludes it from everything published from here on, but copies that \
     have already been distributed cannot be recalled. Withdrawing does not undo that.";

/// Credit is not clawed back, and this says only that -- nothing about how
/// much, when it settles, or what it is worth.
pub const WITHDRAW_CREDIT_NOTE: &str = "Credit already recorded stays.";

/// The canonical body for a tier, or `None` for a tier this build has never
/// heard of.
pub fn withdraw_canonical_body(reach: &str) -> Option<&'static str> {
    match reach {
        REACH_NOT_DISTRIBUTED => Some(WITHDRAW_BODY_NOT_DISTRIBUTED),
        REACH_COMMONS_NOT_DISTRIBUTED => Some(WITHDRAW_BODY_COMMONS_NOT_DISTRIBUTED),
        REACH_COMMONS_DISTRIBUTED => Some(WITHDRAW_BODY_COMMONS_DISTRIBUTED),
        _ => None,
    }
}

/// What this machine can honestly say about how far a trace got, read off
/// the history record's `status`. Not the server's tier: this is the weaker
/// thing the client knows before it asks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WithdrawStage {
    /// `submitted` or `quarantined`. `not_distributed`, exactly -- that is
    /// the server's own rule.
    NotInTheCommons,
    /// `accepted`. One of the two commons tiers, and not knowable which.
    InTheCommons,
    /// Any other status this build does not recognise. Treated as the worst
    /// case, because the furthest reach cannot be ruled out.
    Unknown,
}

impl WithdrawStage {
    pub fn of_status(status: &str) -> Self {
        match status {
            "submitted" | "quarantined" => Self::NotInTheCommons,
            "accepted" => Self::InTheCommons,
            _ => Self::Unknown,
        }
    }
}

/// The confirmation as parts rather than one blob, so the dialog can weight
/// the body carrying the cannot-be-recalled clause and leave the rest as
/// ordinary body copy.
pub struct WithdrawConfirmation {
    pub question: &'static str,
    /// Present only where the tier is ambiguous: says so, in this shell's
    /// own words, before the canonical bodies it cannot choose between.
    pub ambiguity: Option<&'static str>,
    /// Canonical bodies that may apply, in order. One when the tier is
    /// known, two when it is not.
    pub bodies: &'static [&'static str],
    /// Index into `bodies` of the one carrying the cannot-be-recalled
    /// clause, so the dialog can weight it. `None` when none does.
    pub gravest: Option<usize>,
    pub credit: &'static str,
    /// "Withdraw" where the outcome is unambiguous, "Withdraw anyway" where
    /// the contributor is being asked to accept a limit.
    pub confirm_label: &'static str,
}

pub const WITHDRAW_QUESTION: &str = "Withdraw this trace?";
pub const WITHDRAW: &str = "Withdraw";
pub const WITHDRAW_ANYWAY: &str = "Withdraw anyway";
pub const WITHDRAW_CANCEL: &str = "Keep it";

pub fn withdraw_confirmation(stage: WithdrawStage) -> WithdrawConfirmation {
    match stage {
        WithdrawStage::NotInTheCommons => WithdrawConfirmation {
            question: WITHDRAW_QUESTION,
            ambiguity: None,
            bodies: &[WITHDRAW_BODY_NOT_DISTRIBUTED],
            gravest: None,
            credit: WITHDRAW_CREDIT_NOTE,
            confirm_label: WITHDRAW,
        },
        WithdrawStage::InTheCommons => WithdrawConfirmation {
            question: WITHDRAW_QUESTION,
            ambiguity: Some(
                "This trace is in the commons. Whether it has already gone into a published \
                 export or benchmark is decided on the server, and this window cannot tell from \
                 here which of these two applies:",
            ),
            bodies: &[
                WITHDRAW_BODY_COMMONS_NOT_DISTRIBUTED,
                WITHDRAW_BODY_COMMONS_DISTRIBUTED,
            ],
            gravest: Some(1),
            credit: WITHDRAW_CREDIT_NOTE,
            confirm_label: WITHDRAW_ANYWAY,
        },
        WithdrawStage::Unknown => WithdrawConfirmation {
            question: WITHDRAW_QUESTION,
            ambiguity: Some(
                "This window does not recognise what stage this trace reached, so it cannot rule \
                 out the furthest one:",
            ),
            bodies: &[WITHDRAW_BODY_COMMONS_DISTRIBUTED],
            gravest: Some(0),
            credit: WITHDRAW_CREDIT_NOTE,
            confirm_label: WITHDRAW_ANYWAY,
        },
    }
}

/// What actually happened, from the tier the server applied. Never a
/// generic "withdrawn": the canonical body for the tier that applied is
/// what says which of the three outcomes this was.
pub fn withdraw_result_sentence(reach: Option<&str>) -> String {
    match reach.and_then(withdraw_canonical_body) {
        Some(body) => format!("Withdrawn. {body}"),
        // The daemon sent a tier this build does not know. The withdrawal
        // happened; what cannot be stated is how far the trace had
        // travelled -- so the furthest tier is not ruled out.
        None => "Withdrawn, but the server did not report which of the three tiers applied, so \
                 this window cannot tell you whether it had already been included in a published \
                 export or benchmark. If it had, copies that have already been distributed \
                 cannot be recalled."
            .to_string(),
    }
}

/// Withdrawal is authenticated by an account session, which this build has
/// no way to obtain. Leads with the fact that nothing happened: a
/// contributor must not walk away from a failed withdrawal believing their
/// trace was taken back.
pub const WITHDRAW_ACCOUNT_SESSION_REQUIRED: &str = "Nothing was withdrawn and nothing was deleted. Withdrawal is an account-level act, so it is \
     authenticated by your Trace Commons account rather than by this device -- that is what lets \
     you withdraw a trace after losing the machine that sent it. This build has no account \
     sign-in yet, so it cannot make the request.";

/// The daemon's label for "the server has no record of this submission for
/// this account".
///
/// The server answers identically whether the submission belongs to someone
/// else or does not exist at all, so that accounts cannot be enumerated,
/// and this window must not undo that by guessing out loud. So this sentence
/// says neither.
pub const WITHDRAW_NOT_FOUND: &str = "Nothing was withdrawn and nothing was deleted. There is no trace with that id under your \
     account.";

/// The daemon labels that mean not-found. Unreachable today --
/// `daemon/withdraw.rs` collapses every failure into `withdraw-failed` --
/// and handled anyway, because the day that label is passed through is not
/// the day to be inventing this sentence.
pub const WITHDRAW_NOT_FOUND_LABELS: [&str; 3] = ["not-found", "not_found", "submission-not-found"];

/// Any other failure. Same first clause, for the same reason.
///
/// The label is the daemon's fixed, content-free error message, which by
/// contract is never a path, a token, or a response body -- so printing it
/// cannot leak one, and leaving it out would make two different failures
/// indistinguishable to whoever is asked to help.
pub fn withdraw_failure_sentence(label: &str) -> String {
    if label == "account-session-required" {
        return WITHDRAW_ACCOUNT_SESSION_REQUIRED.to_string();
    }
    if WITHDRAW_NOT_FOUND_LABELS.contains(&label) {
        return WITHDRAW_NOT_FOUND.to_string();
    }
    format!(
        "Nothing was withdrawn and nothing was deleted. The request did not go through \
         ({label}). You can try again."
    )
}

/// Why the held group has no "withdraw all of these" button, even though
/// the shared design draws one.
///
/// Rule 6 permits bulk only if the confirmation can say the selected traces
/// may fall into different tiers and that some may already have been
/// distributed. There is a second problem on top of that one, and it is the
/// reason bulk is left out rather than worded around: `withdraw_bulk`
/// reports only `withdrawn` and `failed` counts, so afterwards there is no
/// per-trace tier to report and rule 1 cannot be honoured at all.
pub const WITHDRAW_NO_BULK: &str = "There is no button here that withdraws all of them at once. The bulk call reports only how \
     many succeeded, never what happened to any one trace, and it chooses what to withdraw from \
     this machine's copy of your history, which can be out of date -- so it could not tell you \
     afterwards which of these had already been distributed. Withdraw them one at a time below \
     and each one tells you what it actually did.";

/// The row-level progress label while a withdrawal is in flight. Present
/// tense, because nothing has happened yet.
pub const WITHDRAWING: &str = "Withdrawing…";

// --- Checking for updates -----------------------------------------------

/// The History button behind `refresh_history`.
pub const CHECK_FOR_UPDATES: &str = "Check for updates";

/// What `refresh_history` actually achieved, said accurately.
///
/// The daemon answers `requested: true` and nothing else: the background
/// poller owns the network call, and this only asks it to run sooner. So
/// this sentence says the ask landed, never that anything was fetched --
/// "Updated" would be a claim about a round trip that has not happened yet.
pub const CHECK_FOR_UPDATES_ASKED: &str =
    "Asked for an update. New results appear here as they arrive.";

// --- Community ---------------------------------------------------------
//
// §5.5's panel in History and §5.6's block in Settings are the two public
// surfaces. They share their words as well as their stylesheet: the link
// out of both is the same link.

pub const COMMUNITY_HEADING: &str = "Community";

/// The way out to the public page, from either surface. The arrow is part
/// of the wording, not decoration: it is what says the destination is not
/// in this window.
pub const VIEW_PUBLIC_PROFILE: &str = "View public profile \u{2197}";

/// §7.3: analytics that are withheld are stated in words, never as an empty
/// chart.
pub const COMMUNITY_ANALYTICS_WITHHELD: &str = "Corpus analytics are withheld. The server \
     publishes the roster on consent, but will not publish aggregates without an approved noise \
     mechanism -- so nothing is charted here either.";

/// The footnote below the panel, in native type: the section is a
/// consequence of one setting, and says which one.
pub const COMMUNITY_FOOTNOTE: &str = "Shown only while \"List my handle publicly\" is on. Turn it \
     off in Settings and this section disappears with it.";

// --- Settings: connection ------------------------------------------------

pub const CONNECTION_HEADING: &str = "Connection";
pub const CONNECTED: &str = "Connected";
pub const NOT_CONNECTED: &str = "Not connected";
pub const CHECK_CLAUDE_SET: &str = "Claude Code sessions folder set";
pub const CHECK_CLAUDE_DEFAULT: &str = "Claude Code sessions read from the usual place";
pub const CHECK_CODEX_SET: &str = "Codex sessions folder set";
pub const CHECK_CODEX_DEFAULT: &str = "Codex sessions read from the usual place";
pub const CHECK_SCAN_SET: &str = "Extra privacy scan configured";
pub const CHECK_SCAN_UNSET: &str = "No extra privacy scan";

// --- Settings: how it behaves --------------------------------------------
//
// The three timing knobs `set_settings` accepts, as a title and a unit
// each. Every one of them is a promise the daemon keeps -- how long a
// session must be quiet, how long an approval is held, how often a
// contributor may be interrupted -- so each label says what the number does
// to the contributor rather than naming the setting.

pub const KNOB_QUIESCENCE_TITLE: &str = "Quiet time before a session counts as finished";
pub const KNOB_QUIESCENCE_UNIT: &str = "minutes";
pub const KNOB_HOLD_TITLE: &str = "How long you can take something back";
pub const KNOB_HOLD_UNIT: &str = "seconds after you approve";

/// A hold of zero is not a smaller undo window, it is no undo window at
/// all, and the row says so rather than showing a bare `0`.
pub const KNOB_HOLD_ZERO: &str = "No undo window. Approving sends on the next pass.";
pub const KNOB_DIGEST_TITLE: &str = "How often you can be interrupted";
pub const KNOB_DIGEST_UNIT: &str = "hours between notifications, at most";

/// Where a change made here lands, said once under the three of them.
///
/// On Linux the watcher is usually a separate process, so "this window" is
/// the wrong mental model for what was just changed -- and a contributor
/// who thought these were window preferences would be surprised by them
/// still holding after the window closed.
pub const KNOBS_NOTE: &str = "These govern the background watcher, not this window, and take \
     effect as soon as they are changed. The same values are readable and settable from the \
     command line.";

/// A refused write. States the data consequence -- nothing changed -- since
/// a knob that silently snapped back would otherwise look like a value that
/// had been accepted.
pub const KNOB_NOT_CHANGED: &str = "That couldn't be changed just now. Nothing was changed.";

// --- Settings: the public profile, §5.6 ----------------------------------

pub const PUBLIC_HEADING: &str = "Your public profile";
pub const LIST_HANDLE_PUBLICLY: &str = "List my handle publicly";
pub const PUBLIC_FOOTNOTE: &str = "Attribution only -- being listed grants no data use at all. \
     Leaving the roster removes you from future snapshots.";
/// The date is the daemon's, formatted at the call site; only the sentence
/// around it lives here.
pub fn on_roster_since(date: &str) -> String {
    format!("On the roster since {date}")
}
pub const HANDLE_LABEL: &str = "Handle";
pub const BIO_LABEL: &str = "Bio -- 280 bytes, plaintext, no HTML";
pub const SAVE_PROFILE: &str = "Save profile";
pub const LEAVE_ROSTER: &str = "Leave the roster";

// --- The go-public dialog, §5.7 ------------------------------------------

pub const GO_PUBLIC_TITLE: &str = "Go public?";
pub const GO_PUBLIC_HEADLINE: &str = "Put your handle on the public roster?";
pub const PUBLISHED_HEADING: &str = "What gets published";
pub const PUBLISHED_BODY: &str = "Your handle -- real handles only, no pseudonyms. Aggregate \
     counts: accepted, novelty credit, accept rate. The date you went public. Your bio, if you \
     write one.";
pub const NEVER_HEADING: &str = "What never does";
pub const NEVER_BODY: &str = "Your traces or anything in them. Per-trace data of any kind. \
     Anything about sessions you didn't send.";
pub const GO_PUBLIC_ACKNOWLEDGEMENT: &str = "I understand my handle and aggregate counts become \
     public. Leaving the roster removes me from future snapshots.";
pub const GO_PUBLIC_CONFIRM: &str = "Go public";
pub const GO_PUBLIC_FOOTNOTE: &str = "Nothing is pre-checked, and Go public stays off until the \
     acknowledgement is on. This changes attribution only -- it grants no data use.";
/// The handle field inside the dialog. The panel's `HANDLE_LABEL` names
/// the same thing, so the same constant would do -- except that here the
/// field is empty and has to say what to put in it, and "Handle" over an
/// empty box does not.
pub const GO_PUBLIC_HANDLE_LABEL: &str = "The handle to publish";
/// The optional bio, said as optional. `BIO_LABEL` carries the budget and
/// the format; what it cannot carry is that leaving this empty is a
/// complete answer rather than an unfinished form.
pub const GO_PUBLIC_BIO_LABEL: &str = "Bio, if you want one -- 280 bytes, plaintext, no HTML";

// --- What claiming and leaving actually did, §5.6 ------------------------
//
// Every sentence below states what is true of the *public* surface first,
// because that is the thing the contributor just changed and the thing
// they cannot inspect from this window. What this device managed to write
// down about it is a second, lesser fact and is worded as one.

/// A claim the server accepted.
pub const PROFILE_PUBLISHED: &str =
    "You're on the roster. Your handle and aggregate counts are public now.";

/// A claim the server accepted and this device then failed to write down.
///
/// This is what `handle_persisted: false` means, and it is emphatically
/// not a failed claim: the server has taken the handle, so the profile is
/// public whatever happened on this machine afterwards. Telling a
/// contributor their handle did not go up when it did is the one error
/// this surface must never make -- it is a false statement about a public,
/// outward-facing act, and they would walk away believing they are
/// unlisted. So the sentence leads with the publication, and describes the
/// local loss for exactly what it is: this window will misreport the state
/// until the next successful save, and nothing public changes either way.
pub const PROFILE_PUBLISHED_NOT_CACHED: &str = "You're on the roster -- your handle and aggregate counts are public now. This device \
     couldn't keep its own copy of the profile, so this window will show you as unlisted again \
     until you save it once more. That doesn't change anything about what is public.";

/// A withdrawal the server accepted.
pub const PROFILE_LEFT_ROSTER: &str = "You've left the roster. Your handle isn't published any \
     more, and future snapshots won't include you.";

/// A withdrawal the server accepted and this device then failed to write
/// down. The mirror of `PROFILE_PUBLISHED_NOT_CACHED`, and stated for the
/// same reason: the row is gone from the server regardless, so the
/// withdrawal is not in doubt -- only what this window will show next.
pub const PROFILE_LEFT_ROSTER_NOT_CACHED: &str = "You've left the roster -- your handle isn't published any more, and future snapshots won't \
     include you. This device couldn't clear its own copy of the profile, so this window may show \
     the old handle again until it can.";

/// A claim the server or the daemon refused, from the daemon's fixed
/// label.
///
/// Every branch says that nothing was published, because in every one of
/// them nothing was: the refusal happens before or instead of the `PUT`.
/// The rules themselves are not re-implemented here -- the daemon and the
/// server share one copy of them in `community_handle`, and a second copy
/// in this window is how a handle this shell accepts becomes a handle the
/// server refuses. These sentences only translate the verdict.
pub fn profile_failure_sentence(label: &str) -> String {
    let reason = match label {
        "handle-required" => "There's no handle in the box yet.",
        "handle-too-short" => "That handle is too short -- it needs at least 3 characters.",
        "handle-too-long" => "That handle is too long -- 32 characters at most.",
        "handle-invalid-character" => {
            "A handle can only use letters, numbers, hyphens and underscores."
        }
        "handle-invalid-boundary" => "A handle has to start and end with a letter or a number.",
        "handle-consecutive-separators" => {
            "A handle can't have two hyphens or underscores in a row."
        }
        "handle-reserved" => "That handle is reserved and can't be claimed.",
        "bio-too-long" => "That bio is over the 280-byte budget.",
        "bio-invalid-character" => "That bio has a character the roster doesn't take.",
        // Not reachable from this window -- it always sends a bio key, null
        // or a string -- and handled anyway, so a contract change surfaces
        // as a sentence rather than as the fallback below.
        "bio-required-or-null" | "bio-invalid" => "The bio wasn't sent in a form the roster takes.",
        "not-logged-in" => "This device isn't connected to Trace Commons.",
        // The underlying failure is never forwarded by the daemon -- it can
        // carry a server response body or a URL -- so there is nothing more
        // specific to say than that it did not go through.
        "profile-update-failed" | "profile-withdraw-failed" | "daemon-not-running" => {
            "The request didn't go through."
        }
        _ => "The request didn't go through.",
    };
    format!("{reason} Nothing was published and nothing changed. You can try again.")
}

/// The same, for a withdrawal: "nothing was published" is the wrong second
/// clause when what failed was an attempt to *un*-publish, and a
/// contributor who read it could conclude they had been taken off the
/// roster when they are still on it.
pub fn roster_leave_failure_sentence(label: &str) -> String {
    let reason = match label {
        "not-logged-in" => "This device isn't connected to Trace Commons.",
        _ => "The request didn't go through.",
    };
    format!(
        "{reason} You're still on the roster and your handle is still published. You can try \
             again."
    )
}

// --- Declining -----------------------------------------------------------

/// The one way this product declines to do something now: "Not now", never
/// "Cancel" and never "No". It is one constant rather than one per dialog
/// because the word is a stance, not a label -- nothing here is ever
/// refused, only not done yet, and three copies of the sentence are three
/// chances for one of them to stop saying that. Used by the arming dialog
/// (§5.1), the go-public dialog (§5.7) and the desktop notification.
pub const NOT_NOW: &str = "Not now";

// --- Arming ------------------------------------------------------------

pub fn arming_heading(project_label: &str) -> String {
    format!("Contribute from {project_label} automatically?")
}
pub const ARMING_BODY: &str = "Every future session in this project will be scrubbed and \
     contributed without asking you. You won't review them first.\n\nYou can turn this off at any \
     time.";
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
pub const NOTIFY_NOTHING_SENT: &str = "Nothing is sent until you review them.";

// --- Background portal ---------------------------------------------------

/// Shown to the desktop's own permission dialog, not to a widget in this
/// window -- `org.freedesktop.portal.Background`'s `reason` option is
/// rendered by the portal implementation itself (GNOME Shell, Plasma, ...).
pub const PORTAL_BACKGROUND_REASON: &str =
    "Trace Commons reviews new sessions and uploads only what you approve.";

// --- Autostart -----------------------------------------------------------

pub const AUTOSTART_HEADING: &str = "Starting automatically";
/// Shown when the systemd user unit is doing the job. The service name is
/// not a filesystem path, so naming it here does not violate the no-paths
/// rule.
pub const AUTOSTART_SYSTEMD_BODY: &str = "A background service you installed already starts \
     Trace Commons at login. Manage it with systemctl --user, not from here, so this window and \
     that service never disagree about whether it's running.";
pub const AUTOSTART_XDG_LABEL: &str = "Start Trace Commons when you log in";
pub const AUTOSTART_XDG_BODY: &str =
    "No background service is installed, so this switch is the other way to do it.";

// --- Background portal probe ----------------------------------------------

/// Shown while `portal::spawn_request`'s classification is still in
/// flight. Replaced by `portal_status_line` once it lands.
pub const PORTAL_STATUS_CHECKING: &str = "Checking whether this desktop can list background apps…";

/// The background-registration row, chosen from both of the two things
/// that actually decide it: whether this desktop has a `Background` portal
/// backend at all (`state`), and whether the systemd user unit -- not this
/// window, and not the portal -- is what really keeps Trace Commons running
/// (`systemd_unit_installed`, from `autostart::detect`).
///
/// The portal is not what keeps the process alive on any desktop; systemd
/// is, with `loginctl enable-linger` needed to survive logout, and no
/// portal can do that on any desktop either. The portal's only job here is
/// being listed in GNOME's or Plasma's own "Background Apps" UI and not
/// being treated as a rogue process. So a desktop with no such backend
/// (XFCE, Cinnamon, MATE, Budgie, Sway, and other wlroots compositors) is
/// not a degraded product when the systemd unit is doing the real work --
/// and it is a real, nameable gap when nothing is.
pub fn portal_status_line(
    state: crate::portal::BackendState,
    systemd_unit_installed: bool,
) -> &'static str {
    use crate::portal::BackendState::{Absent, Present, Unknown};
    match (state, systemd_unit_installed) {
        (Present, true) => {
            "This desktop can list Trace Commons as a background app. The systemd service you \
             installed is what actually keeps it running."
        }
        (Present, false) => {
            "This desktop can list Trace Commons as a background app. That listing alone \
             doesn't keep it running past login -- the switch above does."
        }
        (Absent, true) => {
            "This desktop has no background-app list to register with. Nothing is wrong: the \
             systemd service you installed is what keeps Trace Commons running here, the same \
             as it would anywhere else."
        }
        (Absent, false) => {
            "This desktop has no background-app list to register with, and no systemd service \
             is installed either, so Trace Commons only runs while this window is open. Turn on \
             the switch above, or install the service, to change that."
        }
        (Unknown, true) => {
            "Couldn't tell whether this desktop can list background apps. Either way, the \
             systemd service you installed is what keeps Trace Commons running."
        }
        (Unknown, false) => {
            "Couldn't tell whether this desktop can list background apps. No systemd service is \
             installed, so right now Trace Commons only runs while this window is open."
        }
    }
}

// --- Flatpak session-root access (for onboarding, not yet built) ---------

/// The Linux design spec's exact wording for why a confined build asks for
/// two specific folders rather than the whole home directory. Onboarding
/// does not exist yet (see the report), so nothing renders this today; it
/// is pinned here so the string is ready and cannot drift from the spec
/// when onboarding is built.
pub const FLATPAK_SESSION_ROOTS_EXPLANATION: &str = "Trace Commons needs to read your Claude \
     Code and Codex session files. It asks for access to those folders only.";

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

// --- Updating ------------------------------------------------------------
//
// The one platform this app is forbidden to update itself on, so a flatpak
// portal does the work and this window only ever narrates it -- see
// `update.rs`. Nothing below names the portal, D-Bus, or a monitor; every
// sentence says what happens to the machine and the queue instead.

/// The offer, with the commit a person is being moved to.
///
/// The commit is named because "an update is available" with nothing else
/// is unfalsifiable -- there is no way for a contributor to check they got
/// what they were shown. Twelve characters of an ostree commit is enough to
/// compare against `flatpak info ai.tracecommons.Contributor` and short
/// enough to read.
pub fn update_offer_line(short_commit: &str) -> String {
    format!(
        "A newer Trace Commons is available ({short_commit}). Installing it replaces this app; \
         your queue and everything already waiting in it are untouched."
    )
}

/// The banner's button while an update is merely offered.
pub const UPDATE_AVAILABLE_ACTION: &str = "Install";

/// Kept as a constant so the banner body and the dialog body cannot drift
/// apart, since the dialog is the second time a person reads the same fact.
pub const UPDATE_AVAILABLE_BODY: &str = "A newer Trace Commons is available.";

/// The confirmation, which is where the actual decision is made.
pub const UPDATE_CONFIRM_HEADING: &str = "Install the newer version?";
pub const UPDATE_CONFIRM_BODY: &str = "Flatpak installs it. This app does not change while it is open -- you keep running this \
     version until you quit and reopen. Nothing in your queue is sent, removed or re-scanned.";
pub const UPDATE_CONFIRM_ACCEPT: &str = "Install";
pub const UPDATE_CONFIRM_CANCEL: &str = "Not now";

/// Progress. One sentence, because a progress bar carries the rest.
pub fn update_installing_line(percent: u32) -> String {
    format!("Installing the update -- {percent}% done. You can keep using this window.")
}

/// Installed but not yet running.
pub const UPDATE_READY_BODY: &str = "The update is installed. Quit and reopen Trace Commons to start using it. Your queue stays \
     exactly where it is.";
pub const UPDATE_READY_ACTION: &str = "Quit now";

/// Refused or failed. States the data consequence, names no mechanism, and
/// does not ask anyone to retry -- the portal re-checks on its own.
pub const UPDATE_FAILED_BODY: &str = "The update did not install. This copy is unchanged and nothing in your queue was affected. \
     It will be offered again.";

/// Built from source, so nothing here manages it. Honest about the fact
/// that this app is not checking anything in that case -- there is no
/// version check pending, only one that will never happen on this build.
pub const UPDATE_UNMANAGED_BODY: &str = "This copy was built from source, so updates are not managed here and nothing is being \
     checked. Rebuild from the repository to move to a newer version.";

/// Under flatpak, but nothing answered.
pub const UPDATE_UNAVAILABLE_BODY: &str = "Updates cannot be offered here: this desktop's Flatpak service did not answer. Use your \
     software centre, or run flatpak update, to move to a newer version.";

// --- Onboarding --------------------------------------------------------
//
// Six screens, one decision each. Every string below is verbatim from
// `docs/superpowers/specs/2026-08-08-contributor-shell-shared-design.md`,
// "## Onboarding" -- that document specifies the copy for every shell, so
// this is transcription, not authorship. If a sentence here reads oddly,
// change it there first.

pub const ONBOARD_WELCOME_TITLE: &str = "Trace Commons";
pub const ONBOARD_WELCOME_BODY_1: &str = "Coding agents get better when there are real transcripts to learn from. Almost all of that \
     data is locked inside companies. Trace Commons is a shared pool that isn't.";
/// The bold half of screen 1. Split from the paragraph around it because it
/// is the promise the whole product is judged against.
pub const ONBOARD_WELCOME_DECIDES: &str =
    "You decide what gets contributed. Nothing is sent unless you say so.";
pub const ONBOARD_WELCOME_BODY_2: &str = "This app watches for finished Claude Code and Codex sessions on this machine and shows them \
     to you.";
/// "Good and it is not perfect" is load-bearing: a developer knows automatic
/// redaction is imperfect, and conceding it first is what makes the rest
/// credible. Do not soften it into "thorough" or drop the second clause.
pub const ONBOARD_WELCOME_SCRUB: &str = "Before anything leaves this machine it is scrubbed locally for secrets, keys, and tokens. \
     That scrubbing is good and it is not perfect — which is why you get to look first.";
pub const ONBOARD_GET_STARTED: &str = "Get started";

pub const ONBOARD_CONNECT_TITLE: &str = "Connect";
pub const ONBOARD_CONNECT_PROMPT: &str =
    "Paste the invite link someone sent you, or click it from your email.";
pub const ONBOARD_CONNECT_PLACEHOLDER: &str = "https://…/onboard#…";
pub const ONBOARD_CONNECT_BUTTON: &str = "Connect";
/// One sentence for the entire invite path -- an invite this app cannot
/// parse and one the daemon refused both land here.
///
/// `enroll` answers `enroll-failed` and never echoes the underlying HTTP
/// condition (see "### `enroll`" in `docs/contributor-daemon-ipc-v1_1.md`),
/// so showing anything more specific would either invent detail the daemon
/// withheld or leak the detail it deliberately withheld.
pub const ONBOARD_CONNECT_FAILED: &str =
    "This invite link is no longer valid. Ask whoever sent it for a new one.";

pub const ONBOARD_CONSENT_TITLE: &str = "How may your traces be used?";
pub const ONBOARD_CONSENT_SUBTITLE: &str =
    "You can change this later. It applies to traces you send from now on.";
pub const ONBOARD_CONSENT_ALWAYS: &str = "Always included";
pub const ONBOARD_CONSENT_OPTIONAL: &str = "Optional — each one lets your traces do more";
pub const ONBOARD_CONSENT_CREDIT: &str = "Credit";
pub const ONBOARD_ALWAYS_ON_TAG: &str = "always on";

pub const ONBOARD_SCAN_TITLE: &str = "Extra scrub before sending? (optional)";
pub const ONBOARD_SCAN_LOCAL_ALWAYS: &str = "Local scrubbing removes secrets, keys, tokens and credentials by pattern before anything \
     leaves this machine. It runs either way.";
pub const ONBOARD_SCAN_OFFER: &str = "You can additionally send the message text of each trace — not tool output, not file \
     contents — through a second scanner run by NEAR AI, a third party, to catch personal \
     information the patterns miss: names, addresses, that kind of thing.";
/// Both halves of the disclosure. The cost (text really does leave the
/// machine to a third party) and the reassurance (an unreachable scanner
/// holds traces rather than sending them unscanned). Cutting either half
/// makes the screen dishonest in one direction, so they live in one string.
pub const ONBOARD_SCAN_DISCLOSURE: &str = "This means your message text is transmitted to NEAR AI before it reaches Trace Commons. If \
     that scanner is unreachable, nothing is sent at all — traces wait rather than going out \
     unscanned.";
pub const ONBOARD_SCAN_LOCAL_ONLY: &str = "Local scrubbing only";
pub const ONBOARD_SCAN_WITH_NEAR: &str = "Local scrubbing + NEAR AI scan";

pub const ONBOARD_WATCH_TITLE: &str = "What to watch";

/// The per-project control on screen 5. `Ignore` is offered here and
/// `auto_upload` is not, per the shared spec: excluding a repository is a
/// live thought at this moment and never returns, whereas arming automation
/// before a single preview has been seen asks for trust not yet earned.
pub const ONBOARD_IGNORE: &str = "Ignore";

/// Shown when `set_project_mode` refuses. The same sentence the settings
/// screen uses for the same refusal, so the two places that change a
/// project's mode cannot describe the same failure differently.
pub const PROJECT_MODE_FAILED: &str =
    "That couldn't be changed just now. Nothing else changed either.";
pub const ONBOARD_CONTINUE: &str = "Continue";

pub const ONBOARD_DONE_TITLE: &str = "You're set up. Nothing has been sent.";
/// The macOS wording says "menu bar"; this is the Linux shell, so it says
/// where the app actually lives here. Everything after that first clause is
/// the spec's, unchanged -- the 30-minute quiet period and the at-most-one
/// -every-4-hours promise are commitments the daemon actually keeps.
pub const ONBOARD_DONE_BODY: &str = "Trace Commons lives in your system tray. When a session finishes and goes quiet for 30 \
     minutes, it'll show up there. You'll get at most one notification every 4 hours, and none \
     at all if there's nothing waiting.";
pub const ONBOARD_DONE_BUTTON: &str = "Finish";

/// The short bold label for a consent scope.
///
/// `consent_options` carries the wire name and the description but no
/// human title, so every shell maps them. The fallback matters as much as
/// the table: an operator who adds a scope this build has never heard of
/// still gets a readable row rather than a blank one, and the description
/// beside it comes from the daemon regardless.
pub fn scope_title(wire_name: &str) -> String {
    match wire_name {
        "debugging_evaluation" => "Finding bugs and measuring agents".to_string(),
        "benchmark_only" | "benchmark_creation" => "Turn my traces into test cases".to_string(),
        "ranking_training" | "reward_model_training" => {
            "Train models that judge agent output".to_string()
        }
        "model_training" => "Train coding models directly".to_string(),
        "public_attribution" => "List my handle publicly as a contributor".to_string(),
        other => other.replace('_', " "),
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
    fn the_row_caveat_varies_and_still_concedes() {
        let none = residual_risk_line(0);
        let some = residual_risk_line(4);
        // The whole point: two different situations do not get the same
        // sentence. If these ever converge, the line is wallpaper again.
        assert_ne!(none, some);
        assert!(none.contains("matched nothing"));
        assert!(some.contains("4 things"));
        // And whatever it says, it concedes the limit. A row that reported
        // a count without the concession would be reassurance.
        for line in [&none, &some, &residual_risk_line(1)] {
            assert!(
                line.contains("seen before") || line.contains("patterns it has seen"),
                "the caveat must survive every count: {line}"
            );
        }
        // Singular and plural are both written out; "1 things" reads as a
        // bug, and it is one.
        assert!(residual_risk_line(1).contains("1 thing it"));
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

    #[test]
    fn portal_status_matrix_says_only_what_is_true_in_each_cell() {
        use crate::portal::BackendState::{Absent, Present};

        let present_installed = portal_status_line(Present, true);
        let present_bare = portal_status_line(Present, false);
        let absent_installed = portal_status_line(Absent, true);
        let absent_bare = portal_status_line(Absent, false);

        // A backend that exists is always described as such, whether or
        // not systemd is doing the persisting.
        for line in [present_installed, present_bare] {
            assert!(line.contains("can list Trace Commons as a background app"));
        }
        // A desktop with no backend never says it can -- that would be a
        // false claim, not just an optimistic one.
        for line in [absent_installed, absent_bare] {
            assert!(!line.contains("can list Trace Commons as a background app"));
            assert!(line.contains("no background-app list"));
        }

        // The absent+installed cell is the one the spec calls out by name:
        // it must read as "nothing is wrong", not as a degraded product.
        assert!(absent_installed.to_lowercase().contains("nothing is wrong"));

        // Every cell where systemd *is* what's running the unit says so by
        // name, because that -- not the portal -- is what actually keeps
        // the process running.
        for line in [present_installed, absent_installed] {
            assert!(line.to_lowercase().contains("systemd"));
        }

        // The four conclusive cells are pairwise distinct: each one says
        // something only true in that cell.
        let conclusive = [
            present_installed,
            present_bare,
            absent_installed,
            absent_bare,
        ];
        for (i, a) in conclusive.iter().enumerate() {
            for b in &conclusive[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }

    #[test]
    fn update_copy_never_names_the_portal_or_dbus() {
        // Contributor-facing copy talks about the app, the queue and the
        // machine -- never the mechanism doing the work underneath.
        let all = [
            UPDATE_AVAILABLE_BODY,
            UPDATE_CONFIRM_HEADING,
            UPDATE_CONFIRM_BODY,
            UPDATE_READY_BODY,
            UPDATE_FAILED_BODY,
            UPDATE_UNMANAGED_BODY,
            UPDATE_UNAVAILABLE_BODY,
            &update_offer_line("beefbeefbeef"),
            &update_installing_line(50),
        ]
        .join(" ")
        .to_lowercase();
        for forbidden in ["d-bus", "dbus", "portal", "monitor", "commit", "ostree"] {
            assert!(
                !all.contains(forbidden),
                "update copy names {forbidden}: {all}"
            );
        }
    }

    #[test]
    fn an_unknown_probe_never_asserts_either_answer() {
        use crate::portal::BackendState::Unknown;

        for systemd_unit_installed in [true, false] {
            let line = portal_status_line(Unknown, systemd_unit_installed);
            assert!(!line.contains("can list Trace Commons as a background app"));
            assert!(!line.contains("no background-app list"));
            assert!(line.to_lowercase().contains("couldn't tell"));
        }
    }

    // --- Withdrawal ------------------------------------------------------
    //
    // These are assertions on the copy, not on the plumbing. This block is
    // a second copy of wording whose canonical form lives in a document,
    // and an edit that shortens the cannot-be-recalled clause, or hands an
    // `accepted` trace only the gentler body, is exactly the change nobody
    // would notice in review.

    #[test]
    fn the_canonical_bodies_are_still_the_documents_own_words() {
        // Transcribed from the "Canonical confirmation copy" table in
        // `docs/contributor-daemon-ipc-v1_1.md`. Compared whole rather than
        // by keyword: a paraphrase that kept every keyword would still be a
        // paraphrase.
        assert_eq!(
            WITHDRAW_BODY_NOT_DISTRIBUTED,
            "This trace never entered the commons. Withdrawing deletes it. Nothing was \
             distributed and nothing needs recalling."
        );
        assert_eq!(
            WITHDRAW_BODY_COMMONS_NOT_DISTRIBUTED,
            "This trace is in the commons but has not been included in any published export or \
             benchmark yet. Withdrawing deletes it and excludes it from everything published \
             from here on."
        );
        assert_eq!(
            WITHDRAW_BODY_COMMONS_DISTRIBUTED,
            "This trace has already been included in a published export or benchmark. \
             Withdrawing deletes our copy and excludes it from everything published from here \
             on, but copies that have already been distributed cannot be recalled. Withdrawing \
             does not undo that."
        );
    }

    #[test]
    fn a_trace_already_in_the_commons_is_never_shown_only_the_gentler_tier() {
        // Rule 2. `accepted` may resolve to either commons tier and this
        // window cannot tell which, so showing only the gentler body would
        // be claiming more erasure than may have been achieved.
        let commons = withdraw_confirmation(WithdrawStage::InTheCommons);
        assert!(
            commons.bodies.contains(&WITHDRAW_BODY_COMMONS_DISTRIBUTED),
            "an accepted trace is not warned about distributed copies"
        );
        assert!(
            commons.ambiguity.is_some(),
            "an accepted trace is shown a tier this window cannot know"
        );
        assert_eq!(commons.gravest, Some(1));
        assert_eq!(commons.confirm_label, WITHDRAW_ANYWAY);
    }

    #[test]
    fn a_trace_that_never_entered_the_commons_is_not_told_it_was_excluded() {
        // The other half of rule 2: `submitted`/`quarantined` maps to
        // `not_distributed` exactly, so the gentlest body is shown alone
        // and no export it was never in is mentioned.
        let outside = withdraw_confirmation(WithdrawStage::NotInTheCommons);
        assert_eq!(outside.bodies, &[WITHDRAW_BODY_NOT_DISTRIBUTED]);
        assert_eq!(outside.gravest, None);
        assert_eq!(outside.confirm_label, WITHDRAW);
        assert_eq!(
            WithdrawStage::of_status("submitted"),
            WithdrawStage::NotInTheCommons
        );
        assert_eq!(
            WithdrawStage::of_status("quarantined"),
            WithdrawStage::NotInTheCommons
        );
        assert_eq!(
            WithdrawStage::of_status("accepted"),
            WithdrawStage::InTheCommons
        );
        assert_eq!(
            WithdrawStage::of_status("something-new"),
            WithdrawStage::Unknown
        );
    }

    #[test]
    fn an_unrecognised_stage_cannot_rule_out_the_furthest_reach() {
        let unknown = withdraw_confirmation(WithdrawStage::Unknown);
        assert_eq!(unknown.bodies, &[WITHDRAW_BODY_COMMONS_DISTRIBUTED]);
        assert_eq!(unknown.gravest, Some(0));
    }

    #[test]
    fn every_tier_states_the_same_verified_thing_about_credit() {
        // Rule 3. Credit already awarded stays awarded, and no tier says
        // anything else about it.
        for stage in [
            WithdrawStage::NotInTheCommons,
            WithdrawStage::InTheCommons,
            WithdrawStage::Unknown,
        ] {
            assert_eq!(withdraw_confirmation(stage).credit, WITHDRAW_CREDIT_NOTE);
        }
    }

    #[test]
    fn no_outcome_is_ever_reported_as_a_bare_withdrawn() {
        // Rule 1. Each tier's report carries that tier's canonical body,
        // and an unknown tier is not smoothed into the mild answer.
        for reach in [
            REACH_NOT_DISTRIBUTED,
            REACH_COMMONS_NOT_DISTRIBUTED,
            REACH_COMMONS_DISTRIBUTED,
        ] {
            let sentence = withdraw_result_sentence(Some(reach));
            assert!(
                sentence.contains(withdraw_canonical_body(reach).unwrap()),
                "{reach} does not carry its tier's canonical wording"
            );
        }
        assert!(withdraw_result_sentence(None).contains("cannot be recalled"));
        assert!(
            withdraw_result_sentence(Some("a-tier-from-the-future")).contains("cannot be recalled")
        );
    }

    #[test]
    fn a_failed_withdrawal_opens_by_saying_nothing_happened() {
        // A contributor must not walk away from a failure believing their
        // trace was taken back, whichever failure it was.
        for sentence in [
            WITHDRAW_ACCOUNT_SESSION_REQUIRED.to_string(),
            WITHDRAW_NOT_FOUND.to_string(),
            withdraw_failure_sentence("withdraw-failed"),
            withdraw_failure_sentence("account-session-required"),
            withdraw_failure_sentence("not-found"),
        ] {
            assert!(
                sentence.starts_with("Nothing was withdrawn"),
                "a failure sentence does not open by saying nothing happened: {sentence}"
            );
        }
    }

    #[test]
    fn the_not_found_sentence_discloses_neither_existence_nor_ownership() {
        // Rule 4: the server answers identically whether a submission
        // belongs to somebody else or does not exist, so that accounts
        // cannot be enumerated. This window must not undo that.
        let lower = WITHDRAW_NOT_FOUND.to_lowercase();
        assert!(!lower.contains("belongs to"));
        assert!(!lower.contains("does not exist"));
    }

    #[test]
    fn asking_for_an_update_never_claims_one_arrived() {
        // `refresh_history` answers `requested: true` and nothing else --
        // the poller owns the network call. Copy that said "Updated" would
        // be a claim about a round trip that has not happened yet.
        let lower = CHECK_FOR_UPDATES_ASKED.to_lowercase();
        assert!(lower.starts_with("asked"));
        assert!(!lower.contains("updated"));
        assert!(!lower.contains("refreshed"));
    }

    #[test]
    fn a_profile_that_was_published_never_reads_as_one_that_was_not() {
        // `handle_persisted: false` is a failed *local cache write*, not a
        // failed claim: the server has already taken the handle. Both
        // sentences must therefore open by saying the contributor is on the
        // roster, and neither may contain the vocabulary of a refusal.
        for sentence in [PROFILE_PUBLISHED, PROFILE_PUBLISHED_NOT_CACHED] {
            assert!(
                sentence.starts_with("You're on the roster"),
                "a published profile must be reported as published: {sentence}"
            );
            let lower = sentence.to_lowercase();
            for forbidden in [
                "couldn't publish",
                "failed",
                "wasn't published",
                "nothing changed",
            ] {
                assert!(
                    !lower.contains(forbidden),
                    "a published profile must not read as a failure ({forbidden}): {sentence}"
                );
            }
        }
        // And the uncached one still says the weaker true thing, rather
        // than being the same sentence twice.
        assert_ne!(PROFILE_PUBLISHED, PROFILE_PUBLISHED_NOT_CACHED);
        assert!(PROFILE_PUBLISHED_NOT_CACHED.contains("until you save it once more"));
    }

    #[test]
    fn a_withdrawal_that_happened_never_reads_as_one_that_did_not() {
        // The mirror rule. The row is gone from the server whether or not
        // the local clear stuck, so neither sentence may leave a
        // contributor thinking they are still listed.
        for sentence in [PROFILE_LEFT_ROSTER, PROFILE_LEFT_ROSTER_NOT_CACHED] {
            assert!(
                sentence.starts_with("You've left the roster"),
                "a completed withdrawal must be reported as completed: {sentence}"
            );
            assert!(sentence.to_lowercase().contains("isn't published any more"));
        }
    }

    #[test]
    fn every_refusal_says_nothing_was_published() {
        // A refusal happens before or instead of the PUT, so in every one
        // of these cases the handle did not go up -- and the contributor
        // has to be able to tell this apart from the published-but-uncached
        // case above.
        for label in [
            "handle-required",
            "handle-too-short",
            "handle-too-long",
            "handle-invalid-character",
            "handle-invalid-boundary",
            "handle-consecutive-separators",
            "handle-reserved",
            "bio-too-long",
            "bio-invalid-character",
            "bio-required-or-null",
            "not-logged-in",
            "profile-update-failed",
            "a-label-nobody-has-written-yet",
        ] {
            let sentence = profile_failure_sentence(label);
            assert!(
                sentence.contains("Nothing was published"),
                "{label} does not say the handle stayed private: {sentence}"
            );
        }
    }

    #[test]
    fn a_failed_withdrawal_never_borrows_the_claim_sentence() {
        // "Nothing was published" is false comfort after a failed
        // withdrawal: the handle is published, which is precisely the
        // problem. This one has to say the contributor is still listed.
        for label in ["not-logged-in", "profile-withdraw-failed"] {
            let sentence = roster_leave_failure_sentence(label);
            assert!(!sentence.contains("Nothing was published"));
            assert!(
                sentence.contains("still on the roster"),
                "{label} does not say the listing survived: {sentence}"
            );
        }
    }

    #[test]
    fn no_profile_sentence_echoes_a_server_error() {
        // The daemon never forwards the underlying error -- it can carry a
        // response body or a URL -- and this mapping must not invent a
        // place to put one either. Every branch is a fixed sentence, so
        // an unknown label reads as the generic failure and nothing else.
        let unknown = profile_failure_sentence("https://ingest.example/v1/community/profile");
        assert!(!unknown.contains("https://"));
        assert_eq!(unknown, profile_failure_sentence("something-else"));
    }
}
