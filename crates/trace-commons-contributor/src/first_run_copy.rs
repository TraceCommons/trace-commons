//! The first-run wording of Ron's #1030 design (`origin/ftux`,
//! `tauri-desktop/frontend/src/features/ftux/`), as one table every native
//! shell reads instead of writing its own.
//!
//! The strings are Ron's, verbatim, with these departures:
//!
//! - No "Share automatically". The Sharing and per-folder rule choices are
//!   `project_copy::contribution_mode_copy` (Ask me / Automatic / Never),
//!   and a shell renders those.
//! - No data-use scope title or description. Those stay in the core's consent
//!   tables. That includes the handle line and its caption: they are the
//!   `public_attribution` scope's title and description.
//! - The "add your tool" caption names what recognition actually reads; the
//!   tools it used to name are not read (owner decision, 2026-10-04), and a
//!   folder that matches nothing is refused with `tools.add_tool_refused`.
//! - The invite placeholder carries no code: Ron's preview showed a mock one.
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
//! - Rules adds `past_sessions_watch_only`: watching only queues the picked
//!   past sessions on this Mac, and the card says they wait there.
//! - Join adds `invite_or_passkey`: a new passkey creates an account of its
//!   own, so it is not combined with an invite. near.ai needs no invite
//!   (owner, Ron's review of #1235), so there is no line asking for one.
//! - Uses adds Start's failures (`sharing_refused`, `scopes_failed`,
//!   `rules_failed`, `private_ai_failed`, `complete_failed`), and the passkey
//!   sheets add `refused`, so no daemon label is ever shown, and
//!   `bound_elsewhere` for an existing passkey whose account this Mac cannot
//!   join yet.
//! - One string has one key: the session count and Undo, shared by more than
//!   one screen, live in `frame`.
//! - Preview-only strings (the mock-data tag, the simulated system sheets
//!   P-3, P-4 and P-6, the preview's automatic-sharing refusal) are absent.
//!
//! Placeholders are `{tool}`, `{host}`, `{pay_range}`, `{count}`, `{folder}`,
//! `{name}`, `{min}`, `{max}`, `{selected}`, `{total}`, `{tools}`, and the
//! session date and duration's `{weekday}`, `{day}`, `{month}`, `{hours}` and
//! `{minutes}`; the shell fills them and adds nothing else.

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
    pub custom_setup_instead: &'static str,
    pub continue_button: &'static str,
    /// Why Continue is off while a found tool is unanswered.
    pub answer_every_tool: &'static str,
    /// `{count}`: sessions found for a tool (Tools) or a repo (Rules).
    pub session_count: &'static str,
    /// Takes back a choice not yet acted on (a passkey or near.ai on Join).
    /// Not Ron's words.
    pub undo: &'static str,
    /// What an unanswered picker reads: Ron's design-system `Picker`
    /// placeholder. The picker's question stays its accessible label.
    pub choose: &'static str,
}

/// Join: the invite, the account cards and skipping (`join-screen.tsx`).
#[derive(Debug, Clone, serde::Serialize)]
pub struct JoinCopy {
    pub title_light: &'static str,
    pub title_bold: &'static str,
    pub body: &'static str,
    pub body_emphasis: &'static str,
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
    /// accepts, `points_per_accepted_trace`. A shell shows a dash for any
    /// other unit, never the wire label. Not Ron's words; owner-approved.
    pub pay_range_points: &'static str,
    /// `{min}`: the same, when the range is a single figure.
    pub pay_range_points_one: &'static str,
    pub passkey_eyebrow: &'static str,
    pub passkey_text: &'static str,
    /// `{name}`: the passkey's name.
    pub passkey_ready: &'static str,
    pub passkey_create: &'static str,
    pub passkey_done: &'static str,
    /// Create passkey chosen but not yet created: the passkey sheets open
    /// once the daemon starts, after Folders or Tools. Not Ron's words.
    pub passkey_chosen: &'static str,
    pub near_ai_eyebrow: &'static str,
    pub near_ai_text: &'static str,
    pub near_ai_sign_in: &'static str,
    /// near.ai chosen but not yet signed in: the sign-in runs once the
    /// daemon starts, after Folders or Tools. Not Ron's words.
    pub near_ai_chosen: &'static str,
    /// A new passkey creates an account of its own, so it is not combined
    /// with an invite. Not Ron's words.
    pub invite_or_passkey: &'static str,
    pub signed_in: &'static str,
    pub no_sharing: &'static str,
    pub skip_note: &'static str,
    pub skip: &'static str,
    /// After the passkey verification is cancelled (`ftux-page.tsx`).
    pub signed_out: &'static str,
}

/// Each asked-about tool's own install page, keyed by the tool, for "Get {tool}".
#[derive(Debug, Clone, serde::Serialize)]
pub struct InstallUrls {
    pub claude_code: &'static str,
    pub codex: &'static str,
    pub gemini_cli: &'static str,
    pub cline: &'static str,
    pub opencode: &'static str,
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
    pub get_tool: &'static str,
    pub download_tool: &'static str,
    /// Where "Get {tool}" sends a person for each tool the Mac is asked about:
    /// the tool's own install page, over https.
    pub install_urls: InstallUrls,
    pub not_installed: &'static str,
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
    /// already had, so this is not `watcher_start_failed`.
    /// **DRAFT, NEEDS APPROVAL**
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
    /// It closes the question and adds the folder as nothing.
    /// **DRAFT, NEEDS APPROVAL**
    pub neither: &'static str,
    /// `{tool}`: a tool's own row and a folder added for it both read
    /// Watch. The daemon watches one folder per tool, so Continue waits for
    /// one of them to say "I don't use it"; this says so beside the added
    /// folder.
    /// **DRAFT, NEEDS APPROVAL**
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
    pub show_all: &'static str,
    pub show_fewer: &'static str,
    pub never_count: &'static str,
    pub never_label: &'static str,
    /// The folders could not be read; Continue stays disabled.
    pub unavailable: &'static str,
    /// One folder's past sessions could not be read.
    pub sessions_unavailable: &'static str,
    /// Watching only: the past-session card's note. Start queues the picked
    /// sessions on this Mac as pending offers and sends none of them, since
    /// there is no enrolment to send them under.
    /// **DRAFT, NEEDS APPROVAL**
    pub past_sessions_watch_only: &'static str,
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
    /// `{count}` optional uses, every one on.
    pub optional_all_on: &'static str,
    pub optional_all_off: &'static str,
    /// `{count}` optional uses, `{selected}` of them on.
    pub optional_some_on: &'static str,
    pub sharing: &'static str,
    pub sharing_loading: &'static str,
    pub sharing_unavailable: &'static str,
    pub base_use_note: &'static str,
    pub start: &'static str,
    /// Start finished on Ask me because the Automatic grant was refused.
    pub sharing_refused: &'static str,
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
}

/// The passkey popups P-1, P-2, P-5 and P-7 (`passkey-flow.tsx`). P-3, P-4
/// and P-6 are system sheets macOS draws, so they have no row.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PasskeyCopy {
    pub back: &'static str,
    pub close: &'static str,
    pub cancel: &'static str,
    pub choose_title: &'static str,
    pub use_existing: &'static str,
    pub create_new: &'static str,
    pub choose_note: &'static str,
    pub name_title: &'static str,
    pub name_field: &'static str,
    pub clear_name: &'static str,
    pub default_name: &'static str,
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
    pub welcome_sign_in: &'static str,
    pub other_options: &'static str,
    /// A passkey ceremony the daemon or the system refused; its label is
    /// never shown.
    pub refused: &'static str,
    /// "Use existing passkey" signed in to an account already bound (on
    /// another Mac, or a legacy account). Enrolling a further Mac into it is
    /// not built, so the sheet signed out and stays on Choose.
    /// **DRAFT, NEEDS APPROVAL**
    pub bound_elsewhere: &'static str,
}

/// The Private AI card's fallbacks only; its words are
/// `private_inference_copy`'s (`private-ai-card.tsx`).
#[derive(Debug, Clone, serde::Serialize)]
pub struct PrivateAiCopy {
    pub loading: &'static str,
    pub unavailable: &'static str,
    pub toggle_loading: &'static str,
}

/// Every first-run string, grouped by screen.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FirstRunCopy {
    pub frame: FrameCopy,
    pub join: JoinCopy,
    pub folders: FoldersCopy,
    pub tools: ToolsCopy,
    pub rules: RulesCopy,
    pub uses: UsesCopy,
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
            custom_setup_instead: "Custom setup instead",
            continue_button: "Continue",
            answer_every_tool: "Answer every tool above to continue",
            session_count: "{count} sessions",
            undo: "Undo",
            choose: "Choose…",
        },
        join: JoinCopy {
            title_light: "Get started on ",
            title_bold: "your terms",
            body: "Start with an invite link, sign-up or sign-in with an existing account, or just click \"Skip\".",
            body_emphasis: "You'll be able to setup or connect your account later to receive credits and manage access to the near.ai ecosystem.",
            invite_eyebrow: "Invite link",
            invite_placeholder: "https://issuer.tracecommons.ai/onboard#…",
            look_up: "Look up",
            invite_joined: "Joined {host} · {pay_range}",
            invite_error: "That is not an invite link. It ends in #code.",
            invite_dead: "This invite link is no longer valid. Ask whoever sent it for a new one.",
            pay_range_points: "an estimated {min}–{max} points per accepted trace, not yet settled",
            pay_range_points_one: "an estimated {min} points per accepted trace, not yet settled",
            passkey_eyebrow: "Sign in with a passkey",
            passkey_text: "Create a passkey that can be connected later.",
            passkey_ready: "“{name}” is ready. Connect it to near.ai any time.",
            passkey_create: "Create passkey",
            passkey_done: "Done",
            passkey_chosen: "You'll create your passkey once watching starts.",
            near_ai_eyebrow: "Sign in with near.ai",
            near_ai_text: "Use the login you already have. Credits land in that account.",
            near_ai_sign_in: "Sign in",
            near_ai_chosen: "You'll sign in once watching starts.",
            invite_or_passkey: "An invite and a new passkey can't be combined. Use one or the other.",
            signed_in: "Signed in",
            no_sharing: "Connecting or creating an account doesn't authorize any data sharing.",
            skip_note: "Skipping sets up watching only. Contributing needs a near.ai account; sign in any time.",
            skip: "Skip: watch only",
            signed_out: "Passkey not verified, so you were signed out. Join again whenever you like.",
        },
        folders: FoldersCopy {
            title_light: "Which folders may this ",
            title_bold: "app watch?",
            body: "We've found the following tools on your device. Traces work by reading coding-session transcripts from locations you specify. Select an option from each of the tools below to continue.",
            loading: "Looking for coding tools on this Mac…",
            watch: "Watch this folder",
            dont_use: "I don’t use it",
            watch_question: "{tool}: watch this folder?",
            choose_folder: "Choose a different folder for {tool}",
            get_tool: "Get {tool}",
            download_tool: "Download {tool}",
            install_urls: InstallUrls {
                claude_code: "https://claude.com/product/claude-code",
                codex: "https://github.com/openai/codex",
                gemini_cli: "https://github.com/google-gemini/gemini-cli",
                cline: "https://cline.bot/",
                opencode: "https://opencode.ai/",
            },
            not_installed: "Install it, then this row asks again.",
            discovery_failed: "Could not look for coding tools on this Mac.",
            retry: "Look again",
            enroll_refused: "Your invite was found, but joining with it did not go through. Press Continue to try again.",
            lookup_unavailable: "Your invite couldn't be checked just now. Press Continue to try again.",
            sign_in_failed: crate::consent_copy::INFERENCE_SIGN_IN_FAILED,
            settings_failed: "Your folder changes couldn't be saved. Watching goes on with the folders you chose before. Press Continue to try again.",
        },
        tools: ToolsCopy {
            title_light: "Connect your ",
            title_bold: "tools and folders.",
            add_tool: "Not seeing your tool above? Click to add or drag & drop.",
            add_tool_caption: "OpenCode, a moved Claude Code or Codex folder, or a folder of exported traces",
            add_tool_refused: "Trace Commons can't read this folder yet.",
            added_by_you: "Added by you",
            which_kind: "What does {folder} hold?",
            trajectory_label: "Exported traces",
            neither: "Neither",
            one_folder_per_tool: "{tool} can watch only one folder. Answer “I don’t use it” on one of its rows.",
        },
        rules: RulesCopy {
            title_light: "Set your ",
            title_bold: "rules and permissions.",
            loading: "Reading repos from your sessions…",
            empty: "No repos to set rules for yet. Rules appear for repos found in the sessions of a tool you watch.",
            repos_found: "Repos found in {tools} sessions",
            rule_for: "Rule for {folder}",
            past_sessions: "Past sessions, by folder",
            selected_summary: "{selected} of {total} selected",
            folder_selected: "{selected} of {total}",
            include_every: "Include every past session in {folder}",
            show_all: "Show all {count}",
            show_fewer: "Show fewer",
            never_count: "{count} · rule is Never",
            never_label: "{folder}: rule is Never",
            unavailable: "Couldn't read repos from your sessions. Go back, then continue to try again.",
            sessions_unavailable: "Past sessions unavailable",
            past_sessions_watch_only: "You're watching only, so the sessions you pick wait on this Mac, unsent, until you join.",
            weekdays: ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"],
            months: [
                "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
            ],
            session_date: "{weekday} {day} {month}",
            duration_minutes: "{minutes} min",
            duration_hours: "{hours} h {minutes} min",
        },
        uses: UsesCopy {
            title_light: "How your data is ",
            title_bold: "used & permissioned.",
            eyebrow: "How your traces may be used",
            required: "required",
            all_optional: "All optional uses",
            optional_all_on: "{count} optional uses, all on",
            optional_all_off: "{count} optional uses, all off",
            optional_some_on: "{count} optional uses · {selected} on",
            sharing: "Sharing",
            sharing_loading: "Loading sharing copy…",
            sharing_unavailable: "Sharing copy unavailable. Starting is disabled.",
            base_use_note: "Tick the first use to contribute. Without it nothing is shared.",
            start: "Start sharing",
            sharing_refused: "Setup finished, but Automatic wasn't turned on. Sharing is on Ask me.",
            scopes_failed: "How your traces may be used couldn't be saved. Setup hasn't finished; try Start sharing again.",
            rules_failed: "A folder rule or past-session choice couldn't be saved. Setup hasn't finished; try Start sharing again.",
            private_ai_failed: "The Private AI setting couldn't be saved. Setup hasn't finished; try Start sharing again.",
            complete_failed: "Your choices were saved, but setup couldn't be marked done. Setup hasn't finished; try Start sharing again.",
        },
        passkey: PasskeyCopy {
            back: "Back",
            close: "Close",
            cancel: "Cancel",
            choose_title: "Continue with passkey",
            use_existing: "Use existing passkey",
            create_new: "Create new passkey",
            choose_note: "A passkey is your sign-in for Trace Commons and near.ai. Nothing about your sessions is sent by signing in.",
            name_title: "Create new passkey",
            name_field: "Passkey name",
            clear_name: "Clear name",
            default_name: "My trace passkey",
            name_warning: "Store your passkey securely. Losing it means losing access to your account and any credit in it.",
            name_empty: "Give the passkey a name you will recognise.",
            name_too_long: "Keep the name under {max} characters.",
            verify_title: "Verify your passkey",
            verify_body: "Sign a message to prove the passkey is yours and unlock contributing and credit.",
            verify: "Verify passkey",
            verify_note: "Cancelling signs you out.",
            welcome_title: "Welcome back",
            welcome_body: "Sign in with your passkey.",
            welcome_sign_in: "Sign in with passkey",
            other_options: "Other sign-in options",
            refused: "The passkey step didn't go through. Try again, or close this and choose another way to join.",
            bound_elsewhere: "This passkey's account is already set up on another Mac, and adding this Mac to it isn't possible yet, so you were signed out here. Close this to choose another way to join, or to watch only.",
        },
        private_ai: PrivateAiCopy {
            loading: "Loading disclosure…",
            unavailable: "Disclosure unavailable. Enabling is disabled.",
            toggle_loading: "Loading disclosure",
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_install_url_is_https() {
        let urls = first_run_copy().folders.install_urls;
        for url in [
            urls.claude_code,
            urls.codex,
            urls.gemini_cli,
            urls.cline,
            urls.opencode,
        ] {
            assert!(url.starts_with("https://"), "{url}");
        }
    }

    #[test]
    fn a_dead_invite_and_a_pay_range_have_their_own_words() {
        let join = first_run_copy().join;
        assert_ne!(join.invite_dead, join.invite_error);
        for line in [join.pay_range_points, join.pay_range_points_one] {
            assert!(!line.contains('_'), "{line}");
            assert!(line.contains("{min}"), "{line}");
            // `InviteLookupResponse`: clients MUST present the range as
            // estimated credit per accepted trace, not yet settled.
            assert!(line.contains("estimated"), "{line}");
            assert!(line.contains("per accepted trace"), "{line}");
            assert!(line.contains("not yet settled"), "{line}");
        }
        assert!(join.pay_range_points.contains("{max}"));
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
        assert!(tools.one_folder_per_tool.contains("I don’t use it"));
    }

    /// Start's failures each have a sentence that is true when it is shown.
    /// A refused grant comes after setup finished, so it never says nothing
    /// changed and never points at a control the first run does not have.
    #[test]
    fn start_failures_say_what_happened() {
        let uses = first_run_copy().uses;
        assert!(uses.sharing_refused.contains("Ask me"));
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

    /// An existing passkey whose account is already bound elsewhere cannot
    /// be enrolled from this Mac yet; the sheet signs out and says so, in
    /// words of its own rather than the generic refusal.
    #[test]
    fn a_passkey_bound_elsewhere_has_its_own_line() {
        let passkey = first_run_copy().passkey;
        assert_ne!(passkey.bound_elsewhere, passkey.refused);
        assert!(passkey.bound_elsewhere.contains("another Mac"));
        assert!(passkey.bound_elsewhere.contains("signed out"));
    }

    /// A refused passkey ceremony is worded here: the daemon's label never
    /// reaches the sheet.
    #[test]
    fn a_refused_passkey_has_a_sentence() {
        let passkey = first_run_copy().passkey;
        assert!(!passkey.refused.contains('-'), "{}", passkey.refused);
    }

    /// One string, one key. A pair that must be able to diverge is listed
    /// with its reason.
    #[test]
    fn no_two_keys_carry_the_same_string() {
        // P-1's button and P-2's title are two of Ron's strings that read
        // the same today; one is an action, the other names a sheet.
        const MAY_DIVERGE: &[&[&str]] = &[&[".passkey.create_new", ".passkey.name_title"]];
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
}
