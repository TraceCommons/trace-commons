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
//! - Frame adds `undo`, which takes back a near.ai sign-in chosen on Join
//!   and not yet acted on, and `back`, the first-run footer's Back on every
//!   step after Join (owner, 2026-10-08, reversing Ron's review of #1235,
//!   item 9).
//! - Folders has no line for a tool missing from this Mac: its row asks
//!   nothing and offers "Get {tool}" (owner, 2026-10-08).
//! - Uses has no note asking for the first use to be ticked: the required
//!   use is always included, its box ticked and locked (owner, 2026-10-08).
//! - Join adds `invite_dead` for a real invite the daemon refused,
//!   `pay_range_points` and `pay_range_points_one` to word the invite's
//!   credit range in the one unit the daemon accepts, `near_ai_chosen` for
//!   an account chosen before the daemon runs, and
//!   `unknown` for an invite host or pay range the shell does not know.
//! - Tools adds three lines for a folder that matches more than one kind:
//!   `which_kind` asks which, `trajectory_label` names the exported-traces
//!   option and its row, and `neither` dismisses the question; and
//!   `one_folder_per_tool` for a tool watched in two rows.
//! - Folders adds three lines Ron's preview had no need for, since its data
//!   was mocked: `discovery_failed` and `retry` for a discovery that returns
//!   nothing readable, and `enroll_refused` for an enroll refused after the
//!   invite was accepted. It also carries `lookup_unavailable` and
//!   `sign_in_failed` (the core's existing Private AI sign-in line), so leaving Folders
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
//!   watched tools' traces could earn; `frame.step_traces` and
//!   `frame.step_private_ai` for the two new steps; the Uses graphic's three
//!   labels and `automatic_after_sign_in`; and the Private AI screen's title
//!   and tool count. Sign-in moves last, so Join's words are unchanged.
//! - Traces says the traces found can earn credit and names no amount
//!   (owner, 2026-11-04, for the MVP): `traces.found_credit`, or
//!   `traces.found_could_credit` while the use that earns credit is off.
//!   [`traces_estimate_line`] picks the line, so a shell only shows it.
//! - near.ai needs no invite (owner, Ron's review of #1235), so there is no
//!   line asking for one.
//! - Uses adds Start's failures (`sharing_refused`, `scopes_failed`,
//!   `rules_failed`, `private_ai_failed`, `complete_failed`).
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
//! `{min}`, `{max}`, `{selected}`, `{total}`, `{tools}`, `{title}`, and the
//! session date and duration's `{weekday}`, `{day}`, `{month}`, `{hours}` and
//! `{minutes}`; the shell fills them and adds nothing else.

/// Every placeholder name the table uses, each written `{name}` in a string.
pub const PLACEHOLDERS: &[&str] = &[
    "tool",
    "host",
    "pay_range",
    "count",
    "folder",
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
    /// Approved 2026-10-08 (button rule). On Uses it opens the per-repo
    /// rules in place (Ron, 2026-11-05).
    pub custom_setup_instead: &'static str,
    pub continue_button: &'static str,
    /// Why Continue is off while a found tool is unanswered.
    pub answer_every_tool: &'static str,
    /// `{count}`: sessions found for a tool (Tools) or a repo (Rules).
    pub session_count: &'static str,
    /// The same for exactly one session. Same wording as the Settings
    /// source row's `count_one`. DRAFT: pending the owner's approval.
    pub session_count_one: &'static str,
    /// Takes back a choice not yet acted on (near.ai on Join).
    /// Approved 2026-10-06.
    pub undo: &'static str,
    /// What an unanswered picker reads: Ron's design-system `Picker`
    /// placeholder. The picker's question stays its accessible label.
    pub choose: &'static str,
    /// The footer's Back on every step after Join. Approved 2026-10-08.
    pub back: &'static str,
    /// Cancel on the NEAR AI sign-in wait, the Folders footer and the sharing
    /// confirmations.
    pub cancel: &'static str,
    /// Close on the preview sheet's fallback. Unchanged words, as above.
    pub close: &'static str,
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
    pub near_ai_eyebrow: &'static str,
    pub near_ai_text: &'static str,
    pub near_ai_sign_in: &'static str,
    /// near.ai chosen but not yet signed in: the sign-in runs once the
    /// daemon starts, after Folders or Tools. Approved 2026-10-06.
    pub near_ai_chosen: &'static str,
    /// An invite host or pay range the shell does not know, in place of a
    /// dash (owner ruling, 2026-10-06).
    pub unknown: &'static str,
    pub signed_in: &'static str,
    pub no_sharing: &'static str,
    /// Unread by the macOS shell since sign-in became required (owner,
    /// 2026-10-12): Join has no Skip. Kept so the wire table is unchanged.
    pub skip_note: &'static str,
    /// Approved 2026-10-08 (button rule). Join has not read it since
    /// 2026-10-12; Uses' primary action reads it until the person chooses
    /// something (Ron, 2026-11-05).
    pub skip: &'static str,
}

/// Welcome, the animated first run's splash (Ron's 2026-10-21 canvas).
#[derive(Debug, Clone, serde::Serialize)]
pub struct WelcomeCopy {
    /// Ron's words (2026-10-21). The shell raises it a word at a time.
    pub title: &'static str,
    /// Ron's words (2026-10-21).
    pub body: &'static str,
    /// DRAFT, NEEDS APPROVAL. Also Join's primary button on the Private AI
    /// only path, where nothing is shared so "Start sharing" would be
    /// false: DRAFT, NEEDS APPROVAL (2026-11-04) for that reuse.
    pub start: &'static str,
}

/// Traces: what the watched tools' traces could earn. The titles and
/// `on_this_mac` are DRAFT, NEEDS APPROVAL.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TracesCopy {
    pub title_light: &'static str,
    pub title_bold: &'static str,
    /// Under the count of the watched tools' traces.
    pub on_this_mac: &'static str,
    /// The one line under the count once traces are found. No figure: the
    /// MVP names no amount, in points or dollars (owner, 2026-11-04: "we have
    /// detected some traces that can earn you credit").
    /// DRAFT, NEEDS APPROVAL. Shown by [`traces_estimate_line`].
    pub found_credit: &'static str,
    /// The same while the use that earns credit (`uses.credit_scope`) is
    /// not ticked: conditional, so the headline never promises what the
    /// note under it takes away (Zaki's review of #1690, 2026-10-28).
    /// DRAFT, NEEDS APPROVAL.
    pub found_could_credit: &'static str,
    /// Under `found_credit`, smaller: the credit is not fixed until traces
    /// are accepted (owner, 2026-11-04). Not shown under
    /// `found_could_credit`, where `no_credit` sits instead.
    pub credit_depends: &'static str,
    /// Under the conditional estimate while the use that earns credit
    /// (`uses.credit_scope`) is not ticked, with `back_to_uses`, a link to
    /// the Uses step. A neutral note, not a warning, and it names no going
    /// back: only the link does, since it opens a step the footer's Back
    /// would not name. Owner, 2026-10-27; approved in these words in
    /// Zaki's review of #1690 (2026-10-28).
    /// `no_credit` and `back_to_uses` are unread by the macOS shell since
    /// Traces moved before Uses (Ron, 2026-11-05). Kept so the wire table is
    /// unchanged.
    pub no_credit: &'static str,
    pub back_to_uses: &'static str,
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
    /// The near.ai sign-in did not finish: the core's existing Private AI
    /// sign-in line (`private_inference_copy::CREDENTIAL_FAILED`), not a
    /// second wording. It names only Sign in, which Join shows; the
    /// inference step's line it read before offered a Skip that Join does
    /// not have (tester report, 2026-11-04). DRAFT, NEEDS APPROVAL
    /// (2026-11-04): the reassignment.
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
    /// those folders.
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
    /// Settings > Projects' own lines for the same two failures: setup's
    /// name setup and its Start sharing button, which Settings does not
    /// show (copy-guide rule 7).
    pub settings_sharing_refused: &'static str,
    pub settings_sharing_save_failed: &'static str,
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
    /// The collapsed section under the optional uses that holds the
    /// required use; `{title}` is that use's title. Owner, 2026-10-28.
    pub always_on: &'static str,
    /// The optional use that earns credit, shown first among them, with
    /// `credit_required` under its description (owner, 2026-10-27).
    pub credit_scope: &'static str,
    pub credit_required: &'static str,
}

/// The Private AI card's fallbacks only; its words are
/// `private_inference_copy`'s (`private-ai-card.tsx`).
#[derive(Debug, Clone, serde::Serialize)]
pub struct PrivateAiCopy {
    pub loading: &'static str,
    /// While the offer's words cannot be read. The step has had no switch
    /// since #1685, so it says nothing about enabling.
    pub unavailable: &'static str,
    /// The first two sentences of `private_inference_copy::OFFER_WHAT_FIRST_RUN`,
    /// the only words the Uses card shows before "Learn more" (owner,
    /// 2026-10-08). `offer_lead`, a space and `offer_more` are `OFFER_WHAT_FIRST_RUN`
    /// exactly, so nothing of the disclosure is reworded or lost.
    pub offer_lead: &'static str,
    /// The rest of `OFFER_WHAT_FIRST_RUN`, shown with the exposure and no-repoint
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
            session_count_one: "1 trace",
            undo: "Undo",
            choose: "Choose…",
            back: "Back",
            cancel: "Cancel",
            close: "Close",
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
            near_ai_eyebrow: "Sign in with near.ai",
            near_ai_text: "Use the login you already have. Credits land in that account.",
            near_ai_sign_in: "Sign in",
            near_ai_chosen: "You'll sign in once watching starts.",
            unknown: "Unknown",
            signed_in: "Signed in",
            no_sharing: "Connecting or creating an account doesn't authorize any data sharing.",
            skip_note: "Skipping sets up watching only. Contributing needs a near.ai account; sign in any time.",
            skip: "Skip",
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
            sign_in_failed: crate::private_inference_copy::CREDENTIAL_FAILED,
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
            // Approved by the owner 2026-11-04 (Settings copy pass 2, P120).
            automatic_scope: "Sends past and new traces once they've been unchanged for 24 hours.",
            ask_each_time: "Ask each time",
            // Approved by the owner 2026-11-04 (Settings copy pass 2, P122).
            settings_sharing_note: "Folders set to Ask or Never keep their own setting.",
            keep_asks_title: "Keep folders set to Ask?",
            // Approved by the owner 2026-11-04 (Settings copy pass 2, P123).
            keep_asks_body: "Folders set to Ask: {count}.",
            // Approved by the owner 2026-11-04 (Settings copy pass 2, P130).
            keep_asks_keep: "Keep Ask",
            // Approved by the owner 2026-11-04 (Settings copy pass 2, P132).
            keep_asks_all: "All Automatic",
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
            // Approved by the owner 2026-11-04 (Settings copy pass 2, P126).
            settings_sharing_refused: "Automatic wasn't turned on.",
            // Approved by the owner 2026-11-04 (Settings copy pass 2, P131).
            settings_sharing_save_failed: "Sharing couldn't be saved.",
            scopes_failed: "How your traces may be used couldn't be saved. Setup hasn't finished; try Start sharing again.",
            rules_failed: "A folder rule or past-trace choice couldn't be saved. Setup hasn't finished; try Start sharing again.",
            private_ai_failed: "The Private AI setting couldn't be saved. Setup hasn't finished; try Start sharing again.",
            complete_failed: "Your choices were saved, but setup couldn't be marked done. Setup hasn't finished; try Start sharing again.",
            this_mac: "This Mac",
            you_review: "You review",
            the_commons: "The commons",
            automatic_after_sign_in: "Starts when setup finishes after sign-in.",
            always_on: "Always on: {title} for shared traces",
            credit_scope: crate::consent_copy::DATA_USE_CREDIT_SCOPE,
            credit_required: crate::consent_copy::DATA_USE_CREDIT_REQUIRED,
        },
        traces: TracesCopy {
            title_light: "What your traces ",
            title_bold: "could earn",
            on_this_mac: "traces on this Mac",
            found_credit: "We have detected some traces that can earn you credit.",
            found_could_credit: "We have detected some traces that could earn you credit.",
            credit_depends: "Final credit depends on how many traces are accepted.",
            no_credit: "Allow Train coding models to earn it.",
            back_to_uses: "Go back to Uses",
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

/// Traces' one line under the count: that the traces found can earn
/// credit, with no figure (owner, 2026-11-04: the MVP names no amount).
/// `None` draws no line: nothing found yet (`traces` unknown or 0).
#[must_use]
pub fn traces_estimate_line(copy: &TracesCopy, traces: Option<u64>) -> Option<String> {
    found_line(copy, traces, true)
}

/// [`traces_estimate_line`] while the use that earns credit is not ticked:
/// worded as what the traces could earn (`found_could_credit`).
#[must_use]
pub fn traces_could_earn_line(copy: &TracesCopy, traces: Option<u64>) -> Option<String> {
    found_line(copy, traces, false)
}

fn found_line(copy: &TracesCopy, traces: Option<u64>, earns_credit: bool) -> Option<String> {
    traces.filter(|n| *n > 0)?;
    Some(
        if earns_credit {
            copy.found_credit
        } else {
            copy.found_could_credit
        }
        .to_owned(),
    )
}

/// What a shell sends to word Traces' line: the traces it found and whether
/// the use that earns credit is ticked. An older shell also sends the
/// daemon's dollar range and an invite's points range; they are ignored,
/// since the line names no amount.
#[derive(Debug, Default, serde::Deserialize)]
struct EstimateRequest {
    #[serde(default)]
    traces: Option<u64>,
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
    found_line(
        &first_run_copy().traces,
        request.traces,
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

    /// One session reads "1 trace", not "1 traces": the singular has no
    /// hole, and the plural keeps its `{count}`.
    #[test]
    fn a_single_session_reads_in_the_singular() {
        let frame = &first_run_copy().frame;
        assert_eq!(frame.session_count_one, "1 trace");
        assert!(!frame.session_count_one.contains('{'));
        assert!(frame.session_count.contains("{count}"));
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

    /// While the use that earns credit is off, the line says what the
    /// traces could earn; with it on, that they can (Zaki's review of
    /// #1690). Neither names an amount (owner, 2026-11-04).
    #[test]
    fn the_found_line_is_conditional_while_credit_is_off() {
        let copy = first_run_copy().traces;
        assert_eq!(
            traces_estimate_line(&copy, Some(10)).as_deref(),
            Some("We have detected some traces that can earn you credit.")
        );
        assert_eq!(
            traces_could_earn_line(&copy, Some(10)).as_deref(),
            Some("We have detected some traces that could earn you credit.")
        );
        assert_eq!(traces_could_earn_line(&copy, Some(0)), None);
        let json = |v: serde_json::Value| traces_estimate_line_json(&v.to_string());
        assert_eq!(
            json(serde_json::json!({"traces": 10, "earns_credit": false})).as_deref(),
            Some(copy.found_could_credit)
        );
        assert_eq!(
            json(serde_json::json!({"traces": 10, "earns_credit": true})).as_deref(),
            Some(copy.found_credit)
        );
        assert_eq!(
            json(serde_json::json!({"traces": 10})).as_deref(),
            Some(copy.found_credit)
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
    /// own sentences, split once: together they are first run's offer
    /// exactly, in the core's NEAR AI spelling (owner, 2026-10-08). Settings
    /// reads the shorter `OFFER_WHAT` since the Settings copy pass (P178).
    #[test]
    fn the_private_ai_lead_and_rest_are_the_offer_split_once() {
        let copy = first_run_copy().private_ai;
        assert_eq!(
            format!("{} {}", copy.offer_lead, copy.offer_more),
            crate::private_inference_copy::OFFER_WHAT_FIRST_RUN
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

    /// Back is the frame's one word, read by the first-run footer (one
    /// string, one key). A tool missing from this
    /// Mac has no line of its own (owner, 2026-10-08).
    #[test]
    fn back_is_the_frames_and_a_missing_tool_has_no_line() {
        let copy = first_run_copy();
        assert_eq!(copy.frame.back, "Back");
        let json = serde_json::to_value(&copy).unwrap();
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

    /// Join's sign-in failure names only controls Join shows (copy guide
    /// rule 7): Join has Sign in and no Skip (owner, 2026-10-12), so the
    /// line must not offer to skip (tester report, 2026-11-04).
    #[test]
    fn the_sign_in_failure_offers_no_skip() {
        let copy = first_run_copy();
        let line = copy.folders.sign_in_failed.to_lowercase();
        assert!(!line.contains("skip"), "{line}");
        assert!(line.contains("try again"), "{line}");
    }

    /// Every way leaving Folders or Tools can stop has a sentence, so
    /// Continue never does nothing in silence. A sign-in that did not finish
    /// reads the core's existing Private AI sign-in line rather than a
    /// second wording.
    #[test]
    fn leaving_the_roots_says_why_it_stopped() {
        let copy = first_run_copy();
        assert_eq!(
            copy.folders.sign_in_failed,
            crate::private_inference_copy::CREDENTIAL_FAILED
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

    /// near.ai needs no invite, so nothing on Join asks for one.
    #[test]
    fn join_never_asks_for_an_invite() {
        let json = serde_json::to_string(&first_run_copy()).unwrap();
        assert!(!json.contains("near_ai_needs_invite"));
        assert!(!json.contains("Paste an invite above"));
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
        // The MVP names no amount, in points or dollars (owner, 2026-11-04).
        for line in [
            copy.traces.found_credit,
            copy.traces.found_could_credit,
            copy.traces.credit_depends,
        ] {
            assert!(!line.contains('$'), "{line}");
            assert!(!line.contains("point"), "{line}");
            assert!(!line.contains('{'), "{line}");
            assert!(line.ends_with('.'), "{line}");
        }
        for line in [copy.private_ai.can_connect, copy.private_ai.can_connect_one] {
            assert!(line.contains("{total}"), "{line}");
        }
        assert_eq!(copy.frame.step_private_ai, copy.private_ai.title_bold);
    }

    /// One string, one key. A pair that must be able to diverge is listed
    /// with its reason.
    #[test]
    fn no_two_keys_carry_the_same_string() {
        // The Private AI step's name and its title's bold half read the
        // same; one names a step, the other ends a sentence.
        const MAY_DIVERGE: &[&[&str]] = &[&[".frame.step_private_ai", ".private_ai.title_bold"]];
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

    /// Nothing found, or not counted yet: no line.
    #[test]
    fn no_traces_is_no_line() {
        let copy = first_run_copy().traces;
        for found in [None, Some(0)] {
            assert_eq!(traces_estimate_line(&copy, found), None);
            assert_eq!(traces_could_earn_line(&copy, found), None);
        }
    }

    /// The shells' wire: a JSON object in, the line or nothing out. An
    /// older shell's dollar or points range is read and ignored.
    #[test]
    fn the_estimate_request_reads_its_json() {
        let copy = first_run_copy().traces;
        let line = |v: serde_json::Value| traces_estimate_line_json(&v.to_string());
        assert_eq!(
            line(serde_json::json!({
                "traces": 10,
                "usd_micros_per_accepted_trace": {"min": 120_000, "max": 340_000},
                "credit_range": {"min": 4, "max": 4, "unit": "points_per_accepted_trace"},
            }))
            .as_deref(),
            Some(copy.found_credit)
        );
        assert_eq!(line(serde_json::json!({"traces": null})), None);
        assert_eq!(line(serde_json::json!({})), None);
        assert_eq!(traces_estimate_line_json("not json"), None);
    }
}
