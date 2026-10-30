//! The first-run wording of Ron's #1030 design (`origin/ftux`,
//! `tauri-desktop/frontend/src/features/ftux/`), as one table every native
//! shell reads instead of writing its own.
//!
//! The strings are Ron's, verbatim, with these departures:
//!
//! - No "Share automatically". The Sharing and per-folder rule choices are
//!   `project_copy::contribution_mode_copy` (Ask / Automatic / Never),
//!   and a shell renders those.
//! - No data-use scope title or description. Those are the core's consent
//!   table, `daemon::enroll::consent_options`, which carries each scope's
//!   `title` and `description`; every shell reads both from it. That
//!   includes the handle line and its caption: they are the
//!   `public_attribution` scope's title and description.
//! - The "add your tool" caption names what recognition actually reads; the
//!   tools it used to name are not read (owner decision, 2026-10-04), and a
//!   folder that matches nothing is refused with `tools.add_tool_refused`.
//! - The invite placeholder carries no code: Ron's preview showed a mock one.
//! - The passkey name warning is reworded: an account can hold more than one
//!   passkey, so Ron's "Losing it means losing access" claimed more than is
//!   true (Kristi's review of #1252).
//! - Frame adds `undo`, which takes back a passkey or near.ai chosen on Join
//!   and not yet acted on, and `back`, the first-run footer's Back on every
//!   step after Join (owner, 2026-10-08, reversing Ron's review of #1235,
//!   item 9) and the passkey popups' Back alike.
//! - Folders has no line for a tool missing from this Mac: its row asks
//!   nothing and offers "Get {tool}" (owner, 2026-10-08).
//! - Uses has no note asking for the first use to be ticked: the required
//!   use is always included, its box ticked and locked (owner, 2026-10-08).
//! - Join adds `invite_dead` for a real invite the daemon refused,
//!   `pay_range_points` and `pay_range_points_one` to word the invite's
//!   credit range in the one unit the daemon accepts, `passkey_chosen` and
//!   `near_ai_chosen` for an account chosen before the daemon runs, and
//!   `unknown` for an invite host or pay range the shell does not know.
//! - Tools adds three lines for a folder that matches more than one kind:
//!   `which_kind` asks which, `trajectory_label` names the exported-traces
//!   option and its row, and `neither` dismisses the question; and
//!   `one_folder_per_tool` for a tool watched in two rows.
//! - Folders adds three lines Ron's preview had no need for, since its data
//!   was mocked: `discovery_failed` and `retry` for a discovery that returns
//!   nothing readable, and `enroll_refused` for an enroll refused after the
//!   invite was accepted. It also carries `lookup_unavailable` and
//!   `sign_in_failed` (the core's existing sign-in line), so leaving Folders
//!   or Tools never stops in silence, and `settings_failed` for a changed
//!   folder declaration the running daemon refused.
//! - Rules adds `unavailable` and `retry` for folders the daemon could not
//!   read, `sessions_unavailable` for one folder's past sessions, and
//!   `past_sessions_watch_only`: watching only queues the picked past
//!   sessions on this Mac, and the card says they wait there; and
//!   `not_listed`, how many older sessions a folder has beyond the newest
//!   500 the picker lists.
//! - The animated first run (Ron's 2026-10-21 canvas, owner request
//!   2026-10-22) adds `welcome`, the splash before Tools; `traces`, what the
//!   watched tools' traces could earn, with `traces.estimate`, the owner's
//!   default range before an invite gives one; `frame.step_traces` and
//!   `frame.step_private_ai` for the two new steps; the Uses graphic's three
//!   labels and `automatic_after_sign_in`; and the Private AI screen's title
//!   and tool count. Sign-in moves last, so Join's words are unchanged.
//! - Traces words one estimate for the traces found, never a per-trace rate
//!   (owner, 2026-10-23): `traces.estimate_points` from points, or, when the
//!   commons publishes a dollar range per accepted trace
//!   (`GET /v1/public/first-run-estimate`), `traces.estimate_usd` in
//!   dollars of NEAR AI credit instead. [`traces_estimate_line`] does the
//!   arithmetic and the formatting, so a shell only shows the string.
//! - Join adds `passkey_unavailable` for a Create passkey whose daemon start
//!   failed, so the sheets could not open over Join.
//! - Join adds `invite_or_passkey`: a new passkey creates an account of its
//!   own, so it is not combined with an invite. near.ai needs no invite
//!   (owner, Ron's review of #1235), so there is no line asking for one.
//! - Uses adds Start's failures (`sharing_refused`, `scopes_failed`,
//!   `rules_failed`, `private_ai_failed`, `complete_failed`), and the passkey
//!   sheets add `refused`, so no daemon label is ever shown,
//!   `bound_elsewhere` for an existing passkey whose (legacy) account this
//!   Mac cannot join with a passkey, and `near_ai_mismatch` for a Mac whose
//!   near.ai sign-in is not the one its passkey's account uses.
//! - One string has one key: the session count and Undo, shared by more than
//!   one screen, live in `frame`.
//! - Preview-only strings (the mock-data tag, the simulated system sheets
//!   P-3, P-4 and P-6, the preview's automatic-sharing refusal) are absent.
//!
//! - Uses adds `more_about`, the accessible name of the info button that
//!   holds a scope's description or a Sharing answer's detail, and the
//!   Private AI card adds `offer_lead`, `offer_more` and `learn_more`: the
//!   card shows the offer's first two sentences and discloses the rest in
//!   place (owner, 2026-10-08).
//!
//! Placeholders are `{tool}`, `{host}`, `{pay_range}`, `{count}`, `{folder}`,
//! `{name}`, `{min}`, `{max}`, `{selected}`, `{total}`, `{tools}`, `{title}`, and the
//! session date and duration's `{weekday}`, `{day}`, `{month}`, `{hours}` and
//! `{minutes}`; the shell fills them and adds nothing else.

use trace_commons_protocol::first_run_estimate::{USD_MICROS_PER_CENT, UsdMicrosRange};
use trace_commons_protocol::invite_lookup::{
    CREDIT_RANGE_UNIT_POINTS, CreditRange, valid_credit_range,
};

/// Every placeholder name the table uses, each written `{name}` in a string.
pub const PLACEHOLDERS: &[&str] = &[
    "tool",
    "host",
    "pay_range",
    "count",
    "folder",
    "name",
    "min",
    "max",
    "selected",
    "total",
    "tools",
    "title",
    "weekday",
    "day",
    "month",
    "hours",
    "minutes",
];

/// The first-run window: tiers, step labels and the shared footer controls.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FrameCopy {
    pub quick_setup: &'static str,
    pub custom_setup: &'static str,
    pub step_join: &'static str,
    pub step_folders: &'static str,
    pub step_tools: &'static str,
    pub step_rules: &'static str,
    pub step_uses: &'static str,
    /// The animated first run's Traces step. DRAFT, NEEDS APPROVAL.
    pub step_traces: &'static str,
    /// The animated first run's Private AI step. The same words as
    /// `private_ai.title_bold`, and free to diverge: one names a step, the
    /// other ends a title. DRAFT, NEEDS APPROVAL.
    pub step_private_ai: &'static str,
    /// Approved 2026-10-08 (button rule). On Tools it opens the per-repo
    /// rules in place (Ron, 2026-10-21).
    pub custom_setup_instead: &'static str,
    pub continue_button: &'static str,
    /// Why Continue is off while a found tool is unanswered.
    pub answer_every_tool: &'static str,
    /// `{count}`: sessions found for a tool (Tools) or a repo (Rules).
    pub session_count: &'static str,
    /// Takes back a choice not yet acted on (a passkey or near.ai on Join).
    /// Approved 2026-10-06.
    pub undo: &'static str,
    /// What an unanswered picker reads: Ron's design-system `Picker`
    /// placeholder. The picker's question stays its accessible label.
    pub choose: &'static str,
    /// The footer's Back on every step after Join, and the passkey popups'
    /// Back. Approved 2026-10-08.
    pub back: &'static str,
}

/// Join: the invite, the account cards and skipping (`join-screen.tsx`).
#[derive(Debug, Clone, serde::Serialize)]
pub struct JoinCopy {
    pub title_light: &'static str,
    pub title_bold: &'static str,
    /// One paragraph (owner ruling, 2026-10-08). Sign-in is required
    /// (owner, 2026-10-12): Join has no Skip, so the body names none.
    pub body: &'static str,
    pub invite_eyebrow: &'static str,
    pub invite_placeholder: &'static str,
    pub look_up: &'static str,
    /// `{host}`: the invite's issuer host. `{pay_range}`: the invite's credit
    /// range as the shell presents it (`issuer_client::lookup_invite` says it
    /// is an estimate, never a promise).
    pub invite_joined: &'static str,
    /// The local refusal: what was pasted is not an invite link at all.
    pub invite_error: &'static str,
    /// The daemon refused a real invite (expired, used up, revoked or not
    /// found). Not Ron's words; owner-approved 2026-10-05.
    pub invite_dead: &'static str,
    /// `{min}`, `{max}`: the invite's range in the one unit the daemon
    /// accepts, `points_per_accepted_trace`. A shell shows `unknown` for any
    /// other unit, never the wire label. Not Ron's words; owner-approved.
    /// It words the invite's own terms, per accepted trace, not the total
    /// for the traces found, so it stays per trace; "not yet settled"
    /// dropped, Approved 2026-10-23.
    pub pay_range_points: &'static str,
    /// `{min}`: the same, when the range is a single figure. Approved
    /// 2026-10-23.
    pub pay_range_points_one: &'static str,
    pub passkey_eyebrow: &'static str,
    pub passkey_text: &'static str,
    /// `{name}`: the passkey's name.
    pub passkey_ready: &'static str,
    pub passkey_create: &'static str,
    pub passkey_done: &'static str,
    /// Create passkey chosen but not yet created: the passkey sheets open
    /// once the daemon starts, after Folders or Tools. Approved 2026-10-06.
    pub passkey_chosen: &'static str,
    /// Create passkey on Join starts the daemon so the sheets open there,
    /// over Join (#1030), and it did not start: no account was made and
    /// nothing is watched. Approved 2026-10-07 (owner rewrite).
    pub passkey_unavailable: &'static str,
    pub near_ai_eyebrow: &'static str,
    pub near_ai_text: &'static str,
    pub near_ai_sign_in: &'static str,
    /// near.ai chosen but not yet signed in: the sign-in runs once the
    /// daemon starts, after Folders or Tools. Approved 2026-10-06.
    pub near_ai_chosen: &'static str,
    /// A new passkey creates an account of its own, so it is not combined
    /// with an invite. Approved 2026-10-06.
    pub invite_or_passkey: &'static str,
    /// An invite host or pay range the shell does not know, in place of a
    /// dash (owner ruling, 2026-10-06).
    pub unknown: &'static str,
    pub signed_in: &'static str,
    pub no_sharing: &'static str,
    /// Unread by the macOS shell since sign-in became required (owner,
    /// 2026-10-12): Join has no Skip. Kept so the wire table is unchanged.
    pub skip_note: &'static str,
    /// Approved 2026-10-08 (button rule). Unread since 2026-10-12, as
    /// `skip_note`.
    pub skip: &'static str,
    /// After the passkey verification is cancelled (`ftux-page.tsx`).
    pub signed_out: &'static str,
}

/// Welcome, the animated first run's splash (Ron's 2026-10-21 canvas).
#[derive(Debug, Clone, serde::Serialize)]
pub struct WelcomeCopy {
    /// Ron's words (2026-10-21). The shell raises it a word at a time.
    pub title: &'static str,
    /// Ron's words (2026-10-21).
    pub body: &'static str,
    /// DRAFT, NEEDS APPROVAL.
    pub start: &'static str,
}

/// Traces: what the watched tools' traces could earn. The titles and
/// `on_this_mac` are DRAFT, NEEDS APPROVAL; the estimate lines are approved
/// (2026-10-23).
#[derive(Debug, Clone, serde::Serialize)]
pub struct TracesCopy {
    pub title_light: &'static str,
    pub title_bold: &'static str,
    /// Under the count of the watched tools' traces.
    pub on_this_mac: &'static str,
    /// `{min}`, `{max}`: the traces found times the per-trace range in
    /// points (the invite's, or `estimate` before one), as one total.
    /// Approved 2026-10-23 (owner: a total, not a per-trace rate, and no
    /// "not yet settled"). Filled by [`traces_estimate_line`].
    pub estimate_points: &'static str,
    /// `{min}`: the same, when the total is a single figure. Approved
    /// 2026-10-23.
    pub estimate_points_one: &'static str,
    /// `{min}`, `{max}`: the traces found times the server's dollar range
    /// per accepted trace (`trace_commons_protocol::first_run_estimate`),
    /// as one total; each figure carries its `$`. Approved 2026-10-23 as
    /// "an estimated $X–$Y in NEAR AI credit"; capitalized here because the
    /// line opens the card. Shown in place of `estimate_points`, never
    /// beside it. Filled by [`traces_estimate_line`].
    pub estimate_usd: &'static str,
    /// `{min}`: the same, when the total is a single figure ("an estimated
    /// $X in NEAR AI credit"). Approved 2026-10-23.
    pub estimate_usd_one: &'static str,
    /// The same four lines while the use that earns credit
    /// (`uses.credit_scope`) is not ticked: what the traces could earn if
    /// the person allowed it, worded as conditional so the headline never
    /// promises what the note under it takes away (Zaki's review of #1690,
    /// 2026-10-28, approved in these words).
    pub could_estimate_points: &'static str,
    pub could_estimate_points_one: &'static str,
    pub could_estimate_usd: &'static str,
    pub could_estimate_usd_one: &'static str,
    /// The per-trace range Traces multiplies before an invite's lookup
    /// gives one, in points per accepted trace (owner, 2026-10-22: 3 to 8,
    /// marked as an estimate). An invite's own range replaces it; a server
    /// dollar range replaces both. The owner's figure.
    pub estimate: EstimateRange,
    /// Under the conditional estimate while the use that earns credit
    /// (`uses.credit_scope`) is not ticked, with `back_to_uses`, a link to
    /// the Uses step. A neutral note, not a warning, and it names no going
    /// back: only the link does, since it opens a step the footer's Back
    /// would not name. Owner, 2026-10-27; approved in these words in
    /// Zaki's review of #1690 (2026-10-28).
    pub no_credit: &'static str,
    pub back_to_uses: &'static str,
}

/// A range of points per accepted trace, the one unit the daemon accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct EstimateRange {
    pub min: u32,
    pub max: u32,
}

/// Folders, Quick setup's tool list (`tool-screens.tsx`, `tool-row.tsx`).
#[derive(Debug, Clone, serde::Serialize)]
pub struct FoldersCopy {
    pub title_light: &'static str,
    pub title_bold: &'static str,
    pub body: &'static str,
    pub loading: &'static str,
    pub watch: &'static str,
    pub dont_use: &'static str,
    /// `{tool}`: the row's accessible picker label.
    pub watch_question: &'static str,
    pub choose_folder: &'static str,
    /// An installed tool whose folder was not found: the button in place of
    /// the picker, and (`{tool}`) its accessible name.
    pub choose_tool_folder: &'static str,
    pub choose_tool_folder_accessibility: &'static str,
    /// A tool that is not installed. Where it leads is
    /// `crate::install_guides`, the table every surface reads.
    pub get_tool: &'static str,
    pub download_tool: &'static str,
    /// Discovery returned no row the shell could read.
    pub discovery_failed: &'static str,
    /// Runs discovery again after `discovery_failed`.
    pub retry: &'static str,
    /// Enroll was refused after the invite was looked up and accepted. The
    /// daemon does not say why, so this names no cause.
    pub enroll_refused: &'static str,
    /// The invite could not be looked up just now (no answer from the
    /// issuer); nothing is said about the invite itself.
    pub lookup_unavailable: &'static str,
    /// The near.ai sign-in did not finish: the core's existing sign-in line
    /// (`consent_copy::INFERENCE_SIGN_IN_FAILED`), not a second wording.
    pub sign_in_failed: &'static str,
    /// The daemon is running and refused a changed folder declaration on a
    /// later Continue (`set_settings`). Watching goes on with the folders it
    /// already had, so this is not `watcher_start_failed`. Approved
    /// 2026-10-06.
    pub settings_failed: &'static str,
}

/// Tools, Custom setup's tool list and the add tile (`tool-screens.tsx`).
#[derive(Debug, Clone, serde::Serialize)]
pub struct ToolsCopy {
    pub title_light: &'static str,
    pub title_bold: &'static str,
    pub add_tool: &'static str,
    pub add_tool_caption: &'static str,
    /// A picked folder whose layout matches no kind the core reads.
    pub add_tool_refused: &'static str,
    pub added_by_you: &'static str,
    /// `{folder}`: a picked folder that matches more than one kind, asked
    /// which one it is.
    pub which_kind: &'static str,
    /// A folder of exported traces, as an option and as its row's name.
    pub trajectory_label: &'static str,
    /// The last option of `which_kind`'s picker: the folder is neither kind.
    /// It closes the question and adds the folder as nothing. Approved
    /// 2026-10-06.
    pub neither: &'static str,
    /// `{tool}`: a tool's own row and a folder added for it both read
    /// Watch. The daemon watches one folder per tool, so Continue waits for
    /// one of them to say "I don't use it"; this says so beside the added
    /// folder. Approved 2026-10-06.
    pub one_folder_per_tool: &'static str,
}

/// Rules and the past-session picker (`rules-screen.tsx`).
#[derive(Debug, Clone, serde::Serialize)]
pub struct RulesCopy {
    pub title_light: &'static str,
    pub title_bold: &'static str,
    pub loading: &'static str,
    pub empty: &'static str,
    /// `{tools}`: the watched tools' names, joined by the shell's list rule.
    pub repos_found: &'static str,
    pub rule_for: &'static str,
    pub past_sessions: &'static str,
    pub selected_summary: &'static str,
    pub folder_selected: &'static str,
    pub include_every: &'static str,
    /// Approved 2026-10-08 (button rule).
    pub show_all: &'static str,
    pub show_fewer: &'static str,
    pub never_count: &'static str,
    pub never_label: &'static str,
    /// The folders could not be read; Continue stays disabled, and the
    /// card offers `retry` beside it, so the line points at no other way
    /// on. Approved 2026-10-06.
    pub unavailable: &'static str,
    /// Reads the folders again after `unavailable`: Rules' own word, not
    /// Folders' `retry`. Approved 2026-10-06.
    pub retry: &'static str,
    /// One folder's past sessions could not be read.
    pub sessions_unavailable: &'static str,
    /// Watching only: the past-session card's note. Start queues the picked
    /// sessions on this Mac as pending offers and sends none of them, since
    /// there is no enrollment to send them under. Approved 2026-10-06.
    pub past_sessions_watch_only: &'static str,
    /// `{count}`: a folder's older sessions beyond the newest 500 the
    /// picker lists (`list_past_sessions`' `not_listed`; owner decision
    /// 2026-10-05). Shown only when `{count}` is above zero.
    /// Approved 2026-10-07.
    pub not_listed: &'static str,
    /// A session's weekday names, Sunday first, as Ron's `WEEKDAYS`.
    pub weekdays: [&'static str; 7],
    /// A session's month names, January first, as Ron's `MONTHS`.
    pub months: [&'static str; 12],
    /// `{weekday}`, `{day}`, `{month}`: a session's start, read in UTC so it
    /// never moves a day with the time zone (Ron's `formatSessionDate`).
    pub session_date: &'static str,
    /// `{minutes}`: a session under an hour long (Ron's `formatDuration`).
    pub duration_minutes: &'static str,
    /// `{hours}`, `{minutes}`: an hour or longer. The shell pads the minutes
    /// to two digits from two hours up, as Ron's `formatDuration` does.
    pub duration_hours: &'static str,
}

/// Uses: data use, Sharing and starting (`uses-screen.tsx`).
#[derive(Debug, Clone, serde::Serialize)]
pub struct UsesCopy {
    pub title_light: &'static str,
    pub title_bold: &'static str,
    pub eyebrow: &'static str,
    pub required: &'static str,
    pub all_optional: &'static str,
    pub reward_opportunities: &'static str,
    pub automatic_scope: &'static str,
    pub ask_each_time: &'static str,
    /// Settings > Projects' sharing choice, the setup choice made
    /// changeable (owner, 2026-10-30; #1753 A2): the line under it, and the
    /// question Automatic asks when folders are set to Ask, with `{count}`
    /// those folders. DRAFT, NEEDS APPROVAL.
    pub settings_sharing_note: &'static str,
    pub keep_asks_title: &'static str,
    pub keep_asks_body: &'static str,
    pub keep_asks_keep: &'static str,
    pub keep_asks_all: &'static str,
    /// `{count}` optional uses, every one on.
    pub optional_all_on: &'static str,
    pub optional_all_off: &'static str,
    /// `{count}` optional uses, `{selected}` of them on.
    pub optional_some_on: &'static str,
    pub sharing: &'static str,
    pub sharing_loading: &'static str,
    pub sharing_unavailable: &'static str,
    /// `{title}`: a scope's or the Sharing card's title. The accessible
    /// name of the info button holding its longer text. Approved
    /// 2026-10-08: the owner's example wording.
    pub more_about: &'static str,
    pub start: &'static str,
    /// Start finished on Ask because the Automatic grant was refused.
    pub sharing_refused: &'static str,
    pub sharing_save_failed: &'static str,
    /// Start stopped before anything it sends was saved: the data uses.
    pub scopes_failed: &'static str,
    /// Start stopped on a folder rule or a past-session include; the data
    /// uses were saved.
    pub rules_failed: &'static str,
    /// Start stopped on the Private AI setting while the Private AI copy,
    /// whose `write_unconfirmed` a shell shows first, is unavailable.
    pub private_ai_failed: &'static str,
    /// Every call succeeded but setup could not be marked finished.
    pub complete_failed: &'static str,
    /// The Uses graphic's three labels: where traces start, the review gate
    /// (on Ask; Automatic reads the mode table's word), and where they go.
    /// DRAFT, NEEDS APPROVAL.
    pub this_mac: &'static str,
    pub you_review: &'static str,
    pub the_commons: &'static str,
    /// Under Automatic before sign-in: sign-in is the last step, and the
    /// grant needs the enrollment it makes. DRAFT, NEEDS APPROVAL.
    pub automatic_after_sign_in: &'static str,
    /// Two buttons of equal weight above the optional data uses, which
    /// start unticked (R7): `allow_all` ticks every optional use, the
    /// public handle excepted; `only_required` unticks them, leaving the
    /// required use. Owner, 2026-10-27; approved as written in Zaki's
    /// review of #1690 (2026-10-28).
    pub allow_all: &'static str,
    pub only_required: &'static str,
    /// The collapsed section under the optional uses that holds the
    /// required use; `{title}` is that use's title. Owner, 2026-10-28.
    pub always_on: &'static str,
    /// The optional use that earns credit, shown first among them, with
    /// `credit_required` under its description (owner, 2026-10-27).
    pub credit_scope: &'static str,
    pub credit_required: &'static str,
}

/// The passkey popups P-1, P-2, P-5 and P-7 (`passkey-flow.tsx`). P-3, P-4
/// and P-6 are system sheets macOS draws, so they have no row.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PasskeyCopy {
    pub close: &'static str,
    pub cancel: &'static str,
    pub choose_title: &'static str,
    /// Approved 2026-10-08 (button rule).
    pub use_existing: &'static str,
    /// Approved 2026-10-08 (button rule).
    pub create_new: &'static str,
    pub choose_note: &'static str,
    /// The Name popup's title.
    pub name_title: &'static str,
    /// The Name popup's submit button. Approved 2026-10-08 (button rule).
    pub name_submit: &'static str,
    pub name_field: &'static str,
    pub clear_name: &'static str,
    pub default_name: &'static str,
    /// Ron's warning, reworded (Kristi b#11): an account can hold more than
    /// one passkey (`passkey_add_*`), so losing one is not losing the
    /// account. Approved 2026-10-07.
    pub name_warning: &'static str,
    pub name_empty: &'static str,
    /// `{max}`: the longest name allowed, in characters.
    pub name_too_long: &'static str,
    pub verify_title: &'static str,
    pub verify_body: &'static str,
    pub verify: &'static str,
    pub verify_note: &'static str,
    pub welcome_title: &'static str,
    pub welcome_body: &'static str,
    /// Approved 2026-10-08 (button rule).
    pub welcome_sign_in: &'static str,
    /// Approved 2026-10-08 (button rule).
    pub other_options: &'static str,
    /// A passkey ceremony the daemon or the system refused; its label is
    /// never shown.
    pub refused: &'static str,
    /// "Use existing passkey" signed in to a legacy account (created by a
    /// path other than passkey creation, so it has no binding an enrollment
    /// could check).
    /// Adding a Mac to one with a passkey is not built, so the sheet signed
    /// out and stays on Choose. (An account bound on another Mac goes through
    /// Verify and joins.) Approved 2026-10-06.
    pub bound_elsewhere: &'static str,
    /// Verify, joining an account bound on another Mac, was refused: this
    /// Mac's near.ai sign-in is not the one that account uses. Nothing was
    /// added; the sheet signed out and stays on Choose. Approved 2026-10-06.
    pub near_ai_mismatch: &'static str,
}

/// The Private AI card's fallbacks only; its words are
/// `private_inference_copy`'s (`private-ai-card.tsx`).
#[derive(Debug, Clone, serde::Serialize)]
pub struct PrivateAiCopy {
    pub loading: &'static str,
    /// While the offer's words cannot be read. The step has had no switch
    /// since #1685, so it says nothing about enabling.
    pub unavailable: &'static str,
    /// The first two sentences of `private_inference_copy::OFFER_WHAT`,
    /// the only words the Uses card shows before "Learn more" (owner,
    /// 2026-10-08). `offer_lead`, a space and `offer_more` are `OFFER_WHAT`
    /// exactly, so nothing of the disclosure is reworded or lost.
    pub offer_lead: &'static str,
    /// The rest of `OFFER_WHAT`, shown with the exposure and no-repoint
    /// sentences once "Learn more" is pressed.
    pub offer_more: &'static str,
    /// Discloses the rest of the Private AI offer in place. Approved
    /// 2026-10-08 (owner).
    pub learn_more: &'static str,
    /// The Private AI step's title. Ron's words (2026-10-21 canvas).
    pub title_light: &'static str,
    pub title_bold: &'static str,
    /// `{total}`: the tools found on this Mac. After the figure of those
    /// Private AI can route. DRAFT, NEEDS APPROVAL.
    pub can_connect: &'static str,
    /// The same, when one tool was found. DRAFT, NEEDS APPROVAL.
    pub can_connect_one: &'static str,
}

/// Every first-run string, grouped by screen.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FirstRunCopy {
    pub frame: FrameCopy,
    pub welcome: WelcomeCopy,
    pub join: JoinCopy,
    pub folders: FoldersCopy,
    pub tools: ToolsCopy,
    pub rules: RulesCopy,
    pub uses: UsesCopy,
    pub traces: TracesCopy,
    pub passkey: PasskeyCopy,
    pub private_ai: PrivateAiCopy,
}

#[must_use]
pub fn first_run_copy() -> FirstRunCopy {
    FirstRunCopy {
        frame: FrameCopy {
            quick_setup: "Quick setup",
            custom_setup: "Custom setup",
            step_join: "Join",
            step_folders: "Folders",
            step_tools: "Tools",
            step_rules: "Rules",
            step_uses: "Uses",
            step_traces: "Traces",
            step_private_ai: "Private AI",
            custom_setup_instead: "Customize",
            continue_button: "Continue",
            answer_every_tool: "Answer every tool above to continue",
            session_count: "{count} traces",
            undo: "Undo",
            choose: "Choose…",
            back: "Back",
        },
        welcome: WelcomeCopy {
            title: "Welcome to Near AI",
            body: "Private AI solutions that put you in control of your data.",
            start: "Get started",
        },
        join: JoinCopy {
            title_light: "Sign in and choose an org for ",
            title_bold: "payment and inference",
            body: "Sign in with your NEAR AI account to get started.",
            invite_eyebrow: "Invite link",
            invite_placeholder: crate::channel::INVITE_PLACEHOLDER,
            look_up: "Look up",
            invite_joined: "Joined {host} · {pay_range}",
            invite_error: "That is not an invite link. It ends in #code.",
            invite_dead: "This invite link is no longer valid. Ask whoever sent it for a new one.",
            pay_range_points: "an estimated {min}–{max} points per accepted trace",
            pay_range_points_one: "an estimated {min} points per accepted trace",
            passkey_eyebrow: "Sign in with a passkey",
            passkey_text: "Create a passkey that can be connected later.",
            passkey_ready: "“{name}” is ready. Connect it to near.ai any time.",
            passkey_create: "Create passkey",
            passkey_done: "Done",
            passkey_chosen: "You'll create your passkey once watching starts.",
            passkey_unavailable: "Couldn't start passkey setup, so no account was made. Try again, or sign in with NEAR AI.",
            near_ai_eyebrow: "Sign in with near.ai",
            near_ai_text: "Use the login you already have. Credits land in that account.",
            near_ai_sign_in: "Sign in",
            near_ai_chosen: "You'll sign in once watching starts.",
            invite_or_passkey: "An invite and a new passkey can't be combined. Use one or the other.",
            unknown: "Unknown",
            signed_in: "Signed in",
            no_sharing: "Connecting or creating an account doesn't authorize any data sharing.",
            skip_note: "Skipping sets up watching only. Contributing needs a near.ai account; sign in any time.",
            skip: "Skip",
            signed_out: "Passkey not verified, so you were signed out. Join again whenever you like.",
        },
        folders: FoldersCopy {
            title_light: "Which folders may this ",
            title_bold: "app watch?",
            body: "We've found the following tools on your device. Traces work by reading coding-agent transcripts from locations you specify. Select an option from each of the tools below to continue.",
            loading: "Looking for coding tools on this Mac…",
            // Approved 2026-10-08 (button rule): the picker's answers.
            watch: "Watch",
            dont_use: "Not used",
            watch_question: "{tool}: watch this folder?",
            choose_folder: "Choose a different folder for {tool}",
            // Approved 2026-10-15 (#1419 review, decision 2): an installed
            // tool with no folder found keeps its row; this button stands
            // where Watch / Not used would be. No caption under it.
            choose_tool_folder: "Choose folder",
            choose_tool_folder_accessibility: "Choose {tool} folder",
            // Approved 2026-10-15 (#1419 review, decision 3): "Install"
            // everywhere. The row names the tool, and `download_tool` is the
            // button's accessible name.
            get_tool: "Install",
            download_tool: "Install {tool}",
            discovery_failed: "Could not look for coding tools on this Mac.",
            retry: "Look again",
            enroll_refused: "Your invite was found, but joining with it did not go through. Press Continue to try again.",
            lookup_unavailable: "Your invite couldn't be checked just now. Press Continue to try again.",
            sign_in_failed: crate::consent_copy::INFERENCE_SIGN_IN_FAILED,
            settings_failed: "Your folder changes couldn't be saved. Watching goes on with the folders you chose before. Press Continue to try again.",
        },
        tools: ToolsCopy {
            title_light: "Connect your ",
            // Ron, 2026-10-21: "Connect your tools", under the graphic.
            title_bold: "tools",
            add_tool: "Not seeing your tool above? Click to add or drag & drop.",
            add_tool_caption: "OpenCode, a moved Claude Code or Codex folder, or a folder of exported traces",
            add_tool_refused: concat!(crate::app_name!(), " can't read this folder yet."),
            added_by_you: "Added by you",
            which_kind: "What does {folder} hold?",
            trajectory_label: "Exported traces",
            neither: "Neither",
            one_folder_per_tool: "{tool} can watch only one folder. Answer “Not used” on one of its rows.",
        },
        rules: RulesCopy {
            title_light: "Set your ",
            title_bold: "rules and permissions.",
            loading: "Reading repos from your traces…",
            empty: "No repos to set rules for yet. Rules appear for repos found in the traces of a tool you watch.",
            repos_found: "Repos found in {tools} traces",
            rule_for: "Rule for {folder}",
            past_sessions: "Past traces, by folder",
            selected_summary: "{selected} of {total} selected",
            folder_selected: "{selected} of {total}",
            include_every: "Include every past trace in {folder}",
            show_all: "Show all",
            show_fewer: "Show fewer",
            never_count: "{count} · rule is Never",
            never_label: "{folder}: rule is Never",
            unavailable: "Couldn't read repos from your traces.",
            retry: "Try again",
            sessions_unavailable: "Past traces unavailable",
            past_sessions_watch_only: "You're watching only, so the traces you pick wait on this Mac, unsent, until you join.",
            not_listed: "Older traces not listed here: {count}",
            weekdays: ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"],
            months: [
                "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
            ],
            session_date: "{weekday} {day} {month}",
            duration_minutes: "{minutes} min",
            duration_hours: "{hours} h {minutes} min",
        },
        uses: UsesCopy {
            title_light: "Data ",
            // Titles under the animated graphics carry no period.
            title_bold: "uses",
            eyebrow: "Data uses",
            required: crate::consent_copy::SCOPE_TAG_REQUIRED,
            all_optional: "All optional uses",
            reward_opportunities: crate::consent_copy::DATA_USE_REWARD_OPPORTUNITIES,
            automatic_scope: "Upload older traces first, then new traces, from all watched folders. Traces must be unchanged for 24 hours. Ignore a folder to exclude it.",
            ask_each_time: "Ask each time",
            // DRAFT, NEEDS APPROVAL (2026-10-30).
            settings_sharing_note: "A folder set to Ask keeps asking, even when Automatic is on. A folder set to Never is never shared.",
            keep_asks_title: "Keep folders set to Ask?",
            keep_asks_body: "Folders set to Ask: {count}. Keep them asking, or make every folder Automatic.",
            keep_asks_keep: "Keep them on Ask",
            keep_asks_all: "Make all Automatic",
            optional_all_on: "{count} optional uses, all on",
            optional_all_off: "{count} optional uses, all off",
            optional_some_on: "{count} optional uses · {selected} on",
            sharing: "Sharing",
            sharing_loading: "Loading sharing copy…",
            sharing_unavailable: "Sharing copy unavailable. Starting is disabled.",
            more_about: "More about {title}",
            start: "Start sharing",
            sharing_refused: "Setup finished, but Automatic wasn't turned on. Sharing is on Ask.",
            sharing_save_failed: "Sharing couldn't be saved. Setup hasn't finished; try Start sharing again.",
            scopes_failed: "How your traces may be used couldn't be saved. Setup hasn't finished; try Start sharing again.",
            rules_failed: "A folder rule or past-trace choice couldn't be saved. Setup hasn't finished; try Start sharing again.",
            private_ai_failed: "The Private AI setting couldn't be saved. Setup hasn't finished; try Start sharing again.",
            complete_failed: "Your choices were saved, but setup couldn't be marked done. Setup hasn't finished; try Start sharing again.",
            this_mac: "This Mac",
            you_review: "You review",
            the_commons: "The commons",
            automatic_after_sign_in: "Starts when setup finishes after sign-in.",
            allow_all: "Allow all",
            only_required: "Only required",
            always_on: "Always on: {title} for shared traces",
            credit_scope: crate::consent_copy::DATA_USE_CREDIT_SCOPE,
            credit_required: crate::consent_copy::DATA_USE_CREDIT_REQUIRED,
        },
        traces: TracesCopy {
            title_light: "What your traces ",
            title_bold: "could earn",
            on_this_mac: "traces on this Mac",
            estimate_points: "An estimated {min}–{max} points",
            estimate_points_one: "An estimated {min} points",
            estimate_usd: "An estimated {min}–{max} in NEAR AI credit",
            estimate_usd_one: "An estimated {min} in NEAR AI credit",
            could_estimate_points: "You could earn an estimated {min}–{max} points",
            could_estimate_points_one: "You could earn an estimated {min} points",
            could_estimate_usd: "You could earn an estimated {min}–{max} in NEAR AI credit",
            could_estimate_usd_one: "You could earn an estimated {min} in NEAR AI credit",
            estimate: EstimateRange { min: 3, max: 8 },
            no_credit: "Allow Train coding models to earn it.",
            back_to_uses: "Go back to Uses",
        },
        passkey: PasskeyCopy {
            close: "Close",
            cancel: "Cancel",
            choose_title: "Continue with passkey",
            use_existing: "Use existing",
            create_new: "Create new",
            choose_note: "A passkey is your sign-in for Trace Commons and near.ai. Nothing about your traces is sent by signing in.",
            name_title: "Create new passkey",
            name_submit: "Create",
            name_field: "Passkey name",
            clear_name: "Clear name",
            default_name: "My trace passkey",
            name_warning: "Store your passkey securely. If you lose every passkey on your account, you may lose access to it and any credit in it.",
            name_empty: "Give the passkey a name you will recognise.",
            name_too_long: "Keep the name under {max} characters.",
            verify_title: "Verify your passkey",
            verify_body: "Sign a message to prove the passkey is yours and unlock contributing and credit.",
            verify: "Verify passkey",
            verify_note: "Cancelling signs you out.",
            welcome_title: "Welcome back",
            welcome_body: "Sign in with your passkey.",
            welcome_sign_in: "Sign in",
            other_options: "Other options",
            refused: "The passkey step didn't go through. Try again, or close this and choose another way to join.",
            bound_elsewhere: "This passkey's account wasn't created with a passkey, and adding a Mac to it with a passkey isn't possible yet, so you were signed out here. Close this to choose another way to join.",
            near_ai_mismatch: "This Mac is signed in to a different near.ai account from the one this passkey's account uses, so this Mac wasn't added to it, and you were signed out here. Sign in to near.ai with the account you use on your other Mac and try again, or close this to choose another way to join.",
        },
        private_ai: PrivateAiCopy {
            loading: "Loading disclosure…",
            unavailable: "Disclosure unavailable.",
            offer_lead: "Lets your tools send AI requests through this app, answered by NEAR AI once you sign in. No tool switches over until you connect it.",
            offer_more: "Requests are logged on this computer.",
            learn_more: "Learn more",
            title_light: "Connect to ",
            title_bold: "Private AI",
            can_connect: "of {total} tools can connect",
            can_connect_one: "of {total} tool can connect",
        },
    }
}

/// One dollar figure from whole micro-dollars: "$0.40", "$1", "$1.50",
/// "$1,234.56". Whole dollars drop the cents; anything else shows two places.
/// A sub-cent remainder is cut, never rounded up, though none reaches here:
/// the server's range is whole cents (`UsdMicrosRange::is_valid`), and so is
/// any whole multiple of it.
#[must_use]
pub fn usd_figure(usd_micros: u64) -> String {
    let dollars = group(usd_micros / 1_000_000);
    let cents = (usd_micros % 1_000_000) / USD_MICROS_PER_CENT;
    if cents == 0 {
        format!("${dollars}")
    } else {
        format!("${dollars}.{cents:02}")
    }
}

/// An integer with a comma between each group of three digits.
#[must_use]
pub fn group(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn fill_range(range: &str, one: &str, min: String, max: String) -> String {
    if min == max {
        one.replace("{min}", &min)
    } else {
        range.replace("{min}", &min).replace("{max}", &max)
    }
}

/// Traces' one estimate line: the traces found times a per-trace range, as
/// a total. `None` draws no line: nothing found yet (`traces` unknown or 0),
/// or a total past `u64`.
///
/// A server dollar range (`usd`, from `GET /v1/public/first-run-estimate`)
/// that this build would show words the total in dollars of NEAR AI credit,
/// whatever an invite says. Without one, points as before: the invite's
/// range when it is in `points_per_accepted_trace` and one the daemon
/// accepts, else the owner's default ([`TracesCopy::estimate`]). Never both.
///
/// The dollar line is the one narrow exception to "the app never shows a
/// dollar figure it cannot verify" (`crate::commons_credit`): it is
/// labelled an estimate, shown only on the first run, from the operator's
/// published configuration, and is never a balance.
#[must_use]
pub fn traces_estimate_line(
    copy: &TracesCopy,
    traces: Option<u64>,
    usd: Option<UsdMicrosRange>,
    points: Option<CreditRange>,
) -> Option<String> {
    estimate_line(copy, traces, usd, points, true)
}

/// [`traces_estimate_line`] while the use that earns credit is not ticked:
/// the same total, worded as what the traces could earn
/// (`could_estimate_*`).
#[must_use]
pub fn traces_could_earn_line(
    copy: &TracesCopy,
    traces: Option<u64>,
    usd: Option<UsdMicrosRange>,
    points: Option<CreditRange>,
) -> Option<String> {
    estimate_line(copy, traces, usd, points, false)
}

fn estimate_line(
    copy: &TracesCopy,
    traces: Option<u64>,
    usd: Option<UsdMicrosRange>,
    points: Option<CreditRange>,
    earns_credit: bool,
) -> Option<String> {
    let (usd_range, usd_one, points_range, points_one) = if earns_credit {
        (
            copy.estimate_usd,
            copy.estimate_usd_one,
            copy.estimate_points,
            copy.estimate_points_one,
        )
    } else {
        (
            copy.could_estimate_usd,
            copy.could_estimate_usd_one,
            copy.could_estimate_points,
            copy.could_estimate_points_one,
        )
    };
    let traces = traces.filter(|n| *n > 0)?;
    if let Some(usd) = usd.filter(UsdMicrosRange::is_valid) {
        let min = traces.checked_mul(usd.min)?;
        let max = traces.checked_mul(usd.max)?;
        return Some(fill_range(
            usd_range,
            usd_one,
            usd_figure(min),
            usd_figure(max),
        ));
    }
    let (per_min, per_max) = points
        .filter(|r| r.unit == CREDIT_RANGE_UNIT_POINTS && valid_credit_range(r.min, r.max))
        .and_then(|r| Some((u64::try_from(r.min).ok()?, u64::try_from(r.max).ok()?)))
        .unwrap_or((u64::from(copy.estimate.min), u64::from(copy.estimate.max)));
    let min = traces.checked_mul(per_min)?;
    let max = traces.checked_mul(per_max)?;
    Some(fill_range(points_range, points_one, group(min), group(max)))
}

/// What a shell sends to word Traces' estimate: the traces it found, the
/// daemon's `first_run_estimate` answer's range, and an invite lookup's
/// `credit_range`. Every field optional; unknown ones ignored.
#[derive(Debug, Default, serde::Deserialize)]
struct EstimateRequest {
    #[serde(default)]
    traces: Option<u64>,
    #[serde(default)]
    usd_micros_per_accepted_trace: Option<UsdMicrosRange>,
    #[serde(default)]
    credit_range: Option<CreditRange>,
    /// Whether the use that earns credit is ticked; absent means it is, so
    /// an older shell keeps its words.
    #[serde(default)]
    earns_credit: Option<bool>,
}

/// [`traces_estimate_line`] over a shell's JSON request (`tc_first_run_estimate_line`).
/// Unreadable JSON is no line.
#[must_use]
pub fn traces_estimate_line_json(request: &str) -> Option<String> {
    let request: EstimateRequest = serde_json::from_str(request).ok()?;
    estimate_line(
        &first_run_copy().traces,
        request.traces,
        request.usd_micros_per_accepted_trace,
        request.credit_range,
        request.earns_credit.unwrap_or(true),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dead_invite_and_a_pay_range_have_their_own_words() {
        let join = first_run_copy().join;
        assert_ne!(join.invite_dead, join.invite_error);
        for line in [join.pay_range_points, join.pay_range_points_one] {
            assert!(!line.contains('_'), "{line}");
            assert!(line.contains("{min}"), "{line}");
            // `InviteLookupResponse`: clients MUST present the range as
            // estimated credit per accepted trace. "not yet settled" was
            // dropped by the owner (2026-10-23).
            assert!(line.contains("estimated"), "{line}");
            assert!(line.contains("per accepted trace"), "{line}");
            assert!(!line.contains("not yet settled"), "{line}");
        }
        assert_eq!(
            join.pay_range_points,
            "an estimated {min}–{max} points per accepted trace"
        );
        assert_eq!(
            join.pay_range_points_one,
            "an estimated {min} points per accepted trace"
        );
    }

    #[test]
    fn first_run_copy_names_both_tiers_and_never_says_share_automatically() {
        let json = serde_json::to_string(&first_run_copy()).unwrap();
        assert!(json.contains("\"Quick setup\"") && json.contains("\"Custom setup\""));
        assert!(!json.contains("Share automatically"));
        assert!(!json.contains("Theia") && !json.contains("SSH"));
    }

    /// Ron's `join-screen.tsx`: `Joined {invite.host} · {invite.payRange}`.
    /// The range is the shell's to fill, never to append.
    #[test]
    fn the_joined_line_holds_the_host_and_the_pay_range() {
        assert_eq!(
            first_run_copy().join.invite_joined,
            "Joined {host} · {pay_range}"
        );
    }

    /// Every `{...}` in the table is one the module doc lists, so a shell
    /// knows every hole it has to fill.
    #[test]
    fn every_placeholder_is_a_documented_one() {
        let json = serde_json::to_string(&first_run_copy()).unwrap();
        let mut unknown = Vec::new();
        let mut rest = json.as_str();
        while let Some(open) = rest.find('{') {
            rest = &rest[open + 1..];
            let Some(close) = rest.find('}') else { break };
            let name = &rest[..close];
            if !name.is_empty()
                && name.chars().all(|c| c.is_ascii_lowercase() || c == '_')
                && !PLACEHOLDERS.contains(&name)
            {
                unknown.push(name.to_owned());
            }
        }
        assert!(unknown.is_empty(), "undocumented placeholders: {unknown:?}");
        for name in PLACEHOLDERS {
            assert!(
                json.contains(&format!("{{{name}}}")),
                "{{{name}}} is documented but no string carries it"
            );
        }
    }

    /// Ron's #1030 picker placeholder (design-system `Picker`): an
    /// unanswered picker reads it, and the question stays its accessible
    /// label.
    #[test]
    fn an_unanswered_picker_reads_rons_choose() {
        let copy = first_run_copy();
        assert_eq!(copy.frame.choose, "Choose…");
        assert_ne!(copy.frame.choose, copy.folders.watch_question);
    }

    /// Ron's `formatSessionDate` and `formatDuration` (`ftux-model.ts`):
    /// "Sat 12 Sep", "52 min", "1 h 18 min", "2 h 04 min". The shell reads
    /// the date in UTC and fills these; it writes no unit of its own.
    #[test]
    fn session_dates_and_durations_are_rons_formats() {
        let rules = first_run_copy().rules;
        assert_eq!(
            rules.weekdays,
            ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"]
        );
        assert_eq!(
            rules.months,
            [
                "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"
            ]
        );
        let date = rules
            .session_date
            .replace("{weekday}", "Sat")
            .replace("{day}", "12")
            .replace("{month}", "Sep");
        assert_eq!(date, "Sat 12 Sep");
        assert_eq!(rules.duration_minutes.replace("{minutes}", "52"), "52 min");
        assert_eq!(
            rules
                .duration_hours
                .replace("{hours}", "2")
                .replace("{minutes}", "04"),
            "2 h 04 min"
        );
    }

    /// Back is a button in the footer, never an instruction: no line tells
    /// the person to go back. The one exception is the link under Traces'
    /// note when the use that earns credit is off (owner, 2026-10-27; the
    /// note itself no longer says it, Zaki's review of #1690).
    #[test]
    fn no_line_says_to_go_back() {
        let mut copy = first_run_copy();
        assert_eq!(copy.traces.back_to_uses, "Go back to Uses");
        copy.traces.back_to_uses = "";
        let json = serde_json::to_string(&copy).unwrap();
        assert!(
            !json.contains("Go back") && !json.contains("go back"),
            "{json}"
        );
    }

    /// The required use sits in a collapsed section named for it (owner,
    /// 2026-10-28).
    #[test]
    fn the_always_on_section_names_the_required_use() {
        let copy = first_run_copy();
        assert_eq!(copy.uses.always_on, "Always on: {title} for shared traces");
        assert_eq!(
            copy.uses
                .always_on
                .replace("{title}", "Debugging and evaluation"),
            "Always on: Debugging and evaluation for shared traces"
        );
    }

    /// The note names the use that earns credit by its title, and says
    /// what allowing it does, not what the person did not do.
    #[test]
    fn the_no_credit_note_names_the_use_that_earns_it() {
        let copy = first_run_copy();
        assert_eq!(copy.uses.credit_scope, "model_training");
        assert_eq!(
            copy.traces.no_credit,
            "Allow Train coding models to earn it."
        );
        assert!(!copy.traces.no_credit.contains("didn't"));
    }

    /// While the use that earns credit is off, the headline says what the
    /// traces could earn, in each of its four forms; with it on, the
    /// headline is unchanged (Zaki's review of #1690).
    #[test]
    fn the_estimate_is_conditional_while_credit_is_off() {
        let copy = first_run_copy().traces;
        let usd = |min, max| Some(UsdMicrosRange { min, max });
        let points = |min, max| {
            Some(CreditRange {
                min,
                max,
                unit: CREDIT_RANGE_UNIT_POINTS.to_owned(),
            })
        };
        for (earning, could) in [
            (
                traces_estimate_line(&copy, Some(10), usd(120_000, 340_000), None),
                traces_could_earn_line(&copy, Some(10), usd(120_000, 340_000), None),
            ),
            (
                traces_estimate_line(&copy, Some(3), usd(500_000, 500_000), None),
                traces_could_earn_line(&copy, Some(3), usd(500_000, 500_000), None),
            ),
            (
                traces_estimate_line(&copy, Some(10), None, None),
                traces_could_earn_line(&copy, Some(10), None, None),
            ),
            (
                traces_estimate_line(&copy, Some(10), None, points(4, 4)),
                traces_could_earn_line(&copy, Some(10), None, points(4, 4)),
            ),
        ] {
            let earning = earning.expect("a line");
            let could = could.expect("a line");
            assert!(earning.starts_with("An estimated "), "{earning}");
            assert_eq!(could, format!("You could earn an e{}", &earning[4..]));
        }
        assert_eq!(
            traces_could_earn_line(&copy, Some(10), usd(120_000, 340_000), None).as_deref(),
            Some("You could earn an estimated $1.20–$3.40 in NEAR AI credit")
        );
        assert_eq!(traces_could_earn_line(&copy, Some(0), None, None), None);
        let json = |v: serde_json::Value| traces_estimate_line_json(&v.to_string());
        assert_eq!(
            json(serde_json::json!({"traces": 10, "earns_credit": false})).as_deref(),
            Some("You could earn an estimated 30–80 points")
        );
        assert_eq!(
            json(serde_json::json!({"traces": 10, "earns_credit": true})).as_deref(),
            Some("An estimated 30–80 points")
        );
        assert_eq!(
            json(serde_json::json!({"traces": 10})).as_deref(),
            Some("An estimated 30–80 points")
        );
    }

    /// Watching only queues picked past sessions on this Mac and sends
    /// nothing, so the card says they wait there, and never that they are
    /// shared.
    #[test]
    fn watching_only_says_past_sessions_wait_on_this_mac() {
        let rules = first_run_copy().rules;
        assert!(rules.past_sessions_watch_only.contains("this Mac"));
        assert!(rules.past_sessions_watch_only.contains("join"));
        assert_ne!(rules.past_sessions_watch_only, rules.past_sessions);
    }

    /// A folder with more past sessions than the picker lists says how many
    /// more, by count alone: it never claims they were included.
    #[test]
    fn a_folder_with_more_sessions_than_listed_says_how_many() {
        let rules = first_run_copy().rules;
        assert!(rules.not_listed.contains("{count}"));
        assert!(!rules.not_listed.contains("include"));
        assert_ne!(rules.not_listed, rules.show_all);
    }

    /// An unknown invite host or pay range reads as a word, never a dash
    /// (owner ruling, 2026-10-06), and it is not a figure.
    #[test]
    fn an_unknown_host_or_range_is_a_word() {
        let join = first_run_copy().join;
        assert_eq!(join.unknown, "Unknown");
        assert!(!join.unknown.contains('—'));
        assert!(!join.unknown.chars().any(|c| c.is_ascii_digit()));
    }

    /// Rules' unavailable line points at no way back: the screen offers a
    /// retry beside it.
    #[test]
    fn unreadable_rules_point_at_no_back_link() {
        let copy = first_run_copy();
        assert!(!copy.rules.unavailable.contains("Go back"));
        assert!(!copy.rules.unavailable.contains("Back"));
        assert_ne!(copy.rules.unavailable, copy.folders.discovery_failed);
        // Rules has its own retry word, not Folders' "Look again".
        assert_eq!(copy.rules.retry, "Try again");
        assert_ne!(copy.rules.retry, copy.folders.retry);
    }

    /// The required use is always included, so nothing asks for it to be
    /// ticked (owner, 2026-10-08).
    #[test]
    fn nothing_asks_for_the_required_use_to_be_ticked() {
        let json = serde_json::to_value(first_run_copy()).unwrap();
        assert!(json["uses"].get("base_use_note").is_none());
        assert!(!json.to_string().contains("Tick the first use"));
    }

    /// The Private AI card's lead and the rest it discloses are the offer's
    /// own sentences, split once: together they are `OFFER_WHAT` exactly,
    /// in the core's NEAR AI spelling (owner, 2026-10-08).
    #[test]
    fn the_private_ai_lead_and_rest_are_the_offer_split_once() {
        let copy = first_run_copy().private_ai;
        assert_eq!(
            format!("{} {}", copy.offer_lead, copy.offer_more),
            crate::private_inference_copy::OFFER_WHAT
        );
        assert!(
            copy.offer_lead
                .starts_with("Lets your tools send AI requests through this app")
        );
        assert!(!copy.offer_lead.contains("Near.AI"));
        assert_eq!(copy.offer_lead.matches(". ").count(), 1, "two sentences");
        assert_eq!(copy.learn_more, "Learn more");
    }

    /// The info button's name carries the row's title.
    #[test]
    fn the_info_buttons_name_carries_the_title() {
        assert!(first_run_copy().uses.more_about.contains("{title}"));
    }

    /// Back is the frame's one word, read by the first-run footer and the
    /// passkey popups alike (one string, one key). A tool missing from this
    /// Mac has no line of its own (owner, 2026-10-08).
    #[test]
    fn back_is_the_frames_and_a_missing_tool_has_no_line() {
        let copy = first_run_copy();
        assert_eq!(copy.frame.back, "Back");
        let json = serde_json::to_value(&copy).unwrap();
        assert!(json["passkey"].get("back").is_none());
        assert!(json["folders"].get("not_installed").is_none());
        let text = json.to_string();
        assert!(!text.contains("this row asks again"), "{text}");
    }

    /// Enroll runs only after the invite was looked up and accepted, so its
    /// refusal must not read as the Join screen's "not an invite link".
    #[test]
    fn a_refused_enroll_does_not_call_a_valid_invite_malformed() {
        let copy = first_run_copy();
        assert_ne!(copy.folders.enroll_refused, copy.join.invite_error);
        assert!(!copy.folders.enroll_refused.contains("not an invite link"));
    }

    /// Discovery that returns nothing usable says so and offers a retry, so
    /// Folders is never an empty list with a closed Continue.
    #[test]
    fn a_failed_discovery_has_a_line_and_a_retry() {
        let copy = first_run_copy();
        assert_ne!(copy.folders.discovery_failed, copy.folders.loading);
        assert!(!copy.folders.retry.is_empty());
    }

    /// A folder that is both an OpenCode export and a trajectory export is a
    /// question the shell asks with the core's words: a prompt naming the
    /// folder and a name for the trajectory option.
    #[test]
    fn an_ambiguous_folder_has_a_question_and_a_trajectory_name() {
        let tools = first_run_copy().tools;
        assert!(
            tools.which_kind.contains("{folder}"),
            "{}",
            tools.which_kind
        );
        assert!(!tools.trajectory_label.trim().is_empty());
        assert_ne!(tools.trajectory_label, tools.add_tool_caption);
        // The question can be dismissed: the folder is neither kind.
        assert!(!tools.neither.trim().is_empty());
        assert_ne!(tools.neither, tools.trajectory_label);
    }

    /// A tool watched in two rows holds Continue, and the added row says
    /// why with the tool's name.
    #[test]
    fn a_tool_watched_twice_has_its_own_line() {
        let tools = first_run_copy().tools;
        assert!(tools.one_folder_per_tool.contains("{tool}"));
        assert!(tools.one_folder_per_tool.contains("“Not used”"));
    }

    /// Start's failures each have a sentence that is true when it is shown.
    /// A refused grant comes after setup finished, so it never says nothing
    /// changed and never points at a control the first run does not have.
    #[test]
    fn start_failures_say_what_happened() {
        let uses = first_run_copy().uses;
        assert!(uses.sharing_refused.contains("on Ask."));
        assert!(uses.sharing_refused.contains("Automatic"));
        for line in [
            uses.sharing_refused,
            uses.scopes_failed,
            uses.rules_failed,
            uses.private_ai_failed,
        ] {
            assert!(!line.contains("Nothing changed"), "{line}");
            assert!(!line.contains("pill"), "{line}");
        }
        for line in [
            uses.scopes_failed,
            uses.rules_failed,
            uses.private_ai_failed,
            uses.complete_failed,
        ] {
            assert!(line.contains("Setup hasn't finished"), "{line}");
        }
        // The marker was not written, so setup did not finish: never the
        // refused grant's "Setup finished".
        assert!(!uses.complete_failed.contains("Setup finished"));
    }

    /// Every way leaving Folders or Tools can stop has a sentence, so
    /// Continue never does nothing in silence. A sign-in that did not finish
    /// reads the core's existing sign-in line rather than a second wording.
    #[test]
    fn leaving_the_roots_says_why_it_stopped() {
        let copy = first_run_copy();
        assert_eq!(
            copy.folders.sign_in_failed,
            crate::consent_copy::INFERENCE_SIGN_IN_FAILED
        );
        // Lookup never reached an answer: nothing is said about the invite.
        assert_ne!(copy.folders.lookup_unavailable, copy.join.invite_error);
        assert_ne!(copy.folders.lookup_unavailable, copy.folders.enroll_refused);
        assert!(!copy.folders.lookup_unavailable.contains("not an invite"));
        // A refused change of folders on a second Continue: the daemon is
        // running, so it never reads as the watcher failing to start.
        assert_ne!(
            copy.folders.settings_failed,
            crate::onboarding_copy::WATCHER_START_FAILED
        );
        assert!(copy.folders.settings_failed.contains("Continue"));
    }

    /// Create passkey that could not start the daemon says so on Join, and
    /// never reads as the watcher's line: no folder answer exists yet.
    #[test]
    fn a_passkey_that_could_not_start_has_its_own_line() {
        let join = first_run_copy().join;
        assert_ne!(
            join.passkey_unavailable,
            crate::onboarding_copy::WATCHER_START_FAILED
        );
        assert!(!join.passkey_unavailable.contains("folder"));
        // Owner, 2026-10-12: sign-in required, so nothing offers watching only.
        assert!(!join.passkey_unavailable.contains("watch"));
        assert!(join.passkey_unavailable.contains("NEAR AI"));
    }

    /// A new passkey creates an account of its own, so Join says why it is
    /// held back beside an invite. near.ai needs no invite, so nothing asks
    /// for one.
    #[test]
    fn join_says_why_a_passkey_is_held_back_and_never_asks_for_an_invite() {
        let join = first_run_copy().join;
        assert_ne!(join.invite_or_passkey, join.passkey_text);
        let json = serde_json::to_string(&first_run_copy()).unwrap();
        assert!(!json.contains("near_ai_needs_invite"));
        assert!(!json.contains("Paste an invite above"));
    }

    /// An existing passkey whose account this Mac cannot join (a legacy
    /// account, which has no binding an enrollment could check) signs out and says so,
    /// in words of its own rather than the generic refusal. An account bound
    /// on another Mac is joinable now, so the line no longer claims a Mac
    /// can't be added to one.
    #[test]
    fn a_passkey_bound_elsewhere_has_its_own_line() {
        let passkey = first_run_copy().passkey;
        assert_ne!(passkey.bound_elsewhere, passkey.refused);
        assert!(!passkey.bound_elsewhere.contains("another Mac"));
        assert!(passkey.bound_elsewhere.contains("signed out"));
    }

    /// A second Mac signed in to a different near.ai account than the one
    /// its passkey's account uses is refused and signed out. The line says
    /// which sign-in to fix, names no account, and is not the generic
    /// refusal or the legacy line.
    #[test]
    fn a_near_ai_mismatch_has_its_own_line() {
        let passkey = first_run_copy().passkey;
        assert_ne!(passkey.near_ai_mismatch, passkey.refused);
        assert_ne!(passkey.near_ai_mismatch, passkey.bound_elsewhere);
        assert!(passkey.near_ai_mismatch.contains("near.ai"));
        assert!(passkey.near_ai_mismatch.contains("signed out"));
        assert!(!passkey.near_ai_mismatch.contains('{'));
    }

    /// Kristi b#11: an account can hold more than one passkey
    /// (`passkey_add_*`), so losing one passkey is not losing the account.
    /// The warning says what is at stake without claiming more.
    #[test]
    fn the_passkey_warning_claims_no_more_than_is_true() {
        let warning = first_run_copy().passkey.name_warning;
        assert!(warning.contains("every passkey"), "{warning}");
        assert!(!warning.contains("Losing it means"), "{warning}");
    }

    /// A refused passkey ceremony is worded here: the daemon's label never
    /// reaches the sheet.
    #[test]
    fn a_refused_passkey_has_a_sentence() {
        let passkey = first_run_copy().passkey;
        assert!(!passkey.refused.contains('-'), "{}", passkey.refused);
    }

    /// The animated first run (Ron's 2026-10-21 canvas): Ron's own Welcome
    /// and Tools words, titles without a period now that they sit under a
    /// graphic, and a points estimate worded like Join's pay range, so a
    /// figure never reads as a promise.
    #[test]
    fn the_animated_first_run_has_its_words() {
        let copy = first_run_copy();
        assert_eq!(copy.welcome.title, "Welcome to Near AI");
        assert_eq!(
            copy.welcome.body,
            "Private AI solutions that put you in control of your data."
        );
        assert_eq!(copy.tools.title_bold, "tools");
        for bold in [
            copy.tools.title_bold,
            copy.uses.title_bold,
            copy.traces.title_bold,
            copy.private_ai.title_bold,
            copy.join.title_bold,
        ] {
            assert!(!bold.ends_with('.'), "{bold}");
        }
        // One total for the traces found, never a per-trace rate, and no
        // "not yet settled" (owner, 2026-10-23).
        for line in [
            copy.traces.estimate_points,
            copy.traces.estimate_points_one,
            copy.traces.estimate_usd,
            copy.traces.estimate_usd_one,
        ] {
            assert!(line.starts_with("An estimated {min}"), "{line}");
            assert!(!line.contains("per accepted trace"), "{line}");
            assert!(!line.contains("not yet settled"), "{line}");
            assert!(!line.contains('$'), "the figure carries the sign: {line}");
        }
        assert_eq!(
            copy.traces.estimate_points,
            "An estimated {min}–{max} points"
        );
        assert_eq!(copy.traces.estimate_points_one, "An estimated {min} points");
        assert_eq!(
            copy.traces.estimate_usd,
            "An estimated {min}–{max} in NEAR AI credit"
        );
        assert_eq!(
            copy.traces.estimate_usd_one,
            "An estimated {min} in NEAR AI credit"
        );
        // The owner's per-trace default before an invite (2026-10-22).
        assert_eq!(copy.traces.estimate, EstimateRange { min: 3, max: 8 });
        assert!(copy.traces.estimate.min <= copy.traces.estimate.max);
        for line in [copy.private_ai.can_connect, copy.private_ai.can_connect_one] {
            assert!(line.contains("{total}"), "{line}");
        }
        assert_eq!(copy.frame.step_private_ai, copy.private_ai.title_bold);
    }

    /// One string, one key. A pair that must be able to diverge is listed
    /// with its reason.
    #[test]
    fn no_two_keys_carry_the_same_string() {
        // Join's near.ai sign-in and Welcome back's passkey sign-in both
        // read "Sign in" under the button rule (2026-10-08); they start
        // different ceremonies and may diverge.
        // The Private AI step's name and its title's bold half read the
        // same; one names a step, the other ends a sentence.
        const MAY_DIVERGE: &[&[&str]] = &[
            &[".join.near_ai_sign_in", ".passkey.welcome_sign_in"],
            &[".frame.step_private_ai", ".private_ai.title_bold"],
        ];
        fn leaves(value: &serde_json::Value, path: &str, out: &mut Vec<(String, String)>) {
            match value {
                serde_json::Value::String(text) => out.push((text.clone(), path.to_owned())),
                serde_json::Value::Object(map) => {
                    for (key, child) in map {
                        leaves(child, &format!("{path}.{key}"), out);
                    }
                }
                _ => {}
            }
        }
        let mut all = Vec::new();
        leaves(
            &serde_json::to_value(first_run_copy()).unwrap(),
            "",
            &mut all,
        );
        let mut by_text: std::collections::BTreeMap<String, Vec<String>> = Default::default();
        for (text, path) in all {
            by_text.entry(text).or_default().push(path);
        }
        let shared: Vec<Vec<String>> = by_text
            .into_values()
            .filter(|paths| paths.len() > 1)
            .map(|mut paths| {
                paths.sort();
                paths
            })
            .filter(|paths| !MAY_DIVERGE.iter().any(|allowed| paths == allowed))
            .collect();
        assert!(shared.is_empty(), "keys sharing one string: {shared:?}");
    }

    fn empty_leaves(value: &serde_json::Value, path: &str, out: &mut Vec<String>) {
        match value {
            serde_json::Value::String(text) if text.trim().is_empty() => out.push(path.to_owned()),
            serde_json::Value::Object(map) => {
                for (key, child) in map {
                    empty_leaves(child, &format!("{path}.{key}"), out);
                }
            }
            serde_json::Value::Array(items) => {
                for (at, child) in items.iter().enumerate() {
                    empty_leaves(child, &format!("{path}[{at}]"), out);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn every_first_run_string_is_non_empty() {
        let value = serde_json::to_value(first_run_copy()).unwrap();
        let mut empty = Vec::new();
        empty_leaves(&value, "", &mut empty);
        assert!(empty.is_empty(), "empty first-run strings: {empty:?}");
    }

    fn usd(min: u64, max: u64) -> Option<UsdMicrosRange> {
        Some(UsdMicrosRange { min, max })
    }

    fn points(min: i64, max: i64) -> Option<CreditRange> {
        Some(CreditRange::points(min, max))
    }

    #[test]
    fn dollars_are_whole_dollars_or_two_places_with_grouping() {
        assert_eq!(usd_figure(400_000), "$0.40");
        assert_eq!(usd_figure(1_000_000), "$1");
        assert_eq!(usd_figure(1_500_000), "$1.50");
        assert_eq!(usd_figure(10_000), "$0.01");
        assert_eq!(usd_figure(1_234_560_000), "$1,234.56");
        assert_eq!(usd_figure(1_000_000_000_000), "$1,000,000");
        // Sub-cent remainders never reach here (the range is whole cents);
        // were one to, it is cut, never rounded up.
        assert_eq!(usd_figure(19_999), "$0.01");
        assert_eq!(group(0), "0");
        assert_eq!(group(999), "999");
        assert_eq!(group(1_000), "1,000");
        assert_eq!(group(10_920), "10,920");
    }

    /// The server's range present: one dollar total for the traces found.
    #[test]
    fn a_server_range_is_one_dollar_total() {
        let copy = first_run_copy().traces;
        assert_eq!(
            traces_estimate_line(&copy, Some(10), usd(120_000, 340_000), None).as_deref(),
            Some("An estimated $1.20–$3.40 in NEAR AI credit")
        );
        assert_eq!(
            traces_estimate_line(&copy, Some(1), usd(400_000, 1_000_000), None).as_deref(),
            Some("An estimated $0.40–$1 in NEAR AI credit")
        );
        assert_eq!(
            traces_estimate_line(&copy, Some(3), usd(500_000, 500_000), None).as_deref(),
            Some("An estimated $1.50 in NEAR AI credit")
        );
        // An invite's points never override the server's dollars.
        assert_eq!(
            traces_estimate_line(&copy, Some(1365), usd(10_000, 20_000), points(2, 6)).as_deref(),
            Some("An estimated $13.65–$27.30 in NEAR AI credit")
        );
    }

    /// No server range: today's points, as one total, from the invite's
    /// range or the owner's default.
    #[test]
    fn without_a_server_range_the_total_is_points() {
        let copy = first_run_copy().traces;
        assert_eq!(
            traces_estimate_line(&copy, Some(1365), None, None).as_deref(),
            Some("An estimated 4,095–10,920 points")
        );
        assert_eq!(
            traces_estimate_line(&copy, Some(1365), None, points(2, 6)).as_deref(),
            Some("An estimated 2,730–8,190 points")
        );
        assert_eq!(
            traces_estimate_line(&copy, Some(10), None, points(4, 4)).as_deref(),
            Some("An estimated 40 points")
        );
        // A range in another unit is not points: the default stands.
        let foreign = CreditRange {
            min: 1,
            max: 2,
            unit: "dollars".into(),
        };
        assert_eq!(
            traces_estimate_line(&copy, Some(1), None, Some(foreign)).as_deref(),
            Some("An estimated 3–8 points")
        );
        // A range the daemon would refuse is not used either.
        assert_eq!(
            traces_estimate_line(&copy, Some(1), None, points(-1, 5)).as_deref(),
            Some("An estimated 3–8 points")
        );
        // A server range this build would not show is no range.
        assert_eq!(
            traces_estimate_line(&copy, Some(1), usd(10_001, 20_000), None).as_deref(),
            Some("An estimated 3–8 points")
        );
    }

    /// Nothing found, or not counted yet: no line, never "$0" or "0 points".
    #[test]
    fn no_traces_is_no_line() {
        let copy = first_run_copy().traces;
        for found in [None, Some(0)] {
            assert_eq!(
                traces_estimate_line(&copy, found, usd(10_000, 20_000), None),
                None
            );
            assert_eq!(traces_estimate_line(&copy, found, None, None), None);
        }
    }

    /// A total past u64 is no line, not a wrapped figure.
    #[test]
    fn an_overflowing_total_is_no_line() {
        let copy = first_run_copy().traces;
        assert_eq!(
            traces_estimate_line(&copy, Some(u64::MAX), usd(10_000, 20_000), None),
            None
        );
        assert_eq!(
            traces_estimate_line(&copy, Some(u64::MAX), None, points(2, 6)),
            None
        );
    }

    /// The shells' wire: a JSON object in, the line or nothing out.
    #[test]
    fn the_estimate_request_reads_its_json() {
        let line = |v: serde_json::Value| traces_estimate_line_json(&v.to_string());
        assert_eq!(
            line(serde_json::json!({
                "traces": 10,
                "usd_micros_per_accepted_trace": {"min": 120_000, "max": 340_000},
            }))
            .as_deref(),
            Some("An estimated $1.20–$3.40 in NEAR AI credit")
        );
        assert_eq!(
            line(serde_json::json!({
                "traces": 10,
                "credit_range": {"min": 4, "max": 4, "unit": "points_per_accepted_trace"},
            }))
            .as_deref(),
            Some("An estimated 40 points")
        );
        assert_eq!(line(serde_json::json!({"traces": null})), None);
        assert_eq!(line(serde_json::json!({})), None);
        assert_eq!(traces_estimate_line_json("not json"), None);
    }
}
