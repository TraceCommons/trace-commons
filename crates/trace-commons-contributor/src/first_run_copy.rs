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
//! - Tools adds two lines for a folder that matches more than one kind:
//!   `which_kind` asks which, and `trajectory_label` names the
//!   exported-traces option and its row.
//! - Folders adds three lines Ron's preview had no need for, since its data
//!   was mocked: `discovery_failed` and `retry` for a discovery that returns
//!   nothing readable, and `enroll_refused` for an enroll refused after the
//!   invite was accepted.
//! - Preview-only strings (the mock-data tag, the simulated system sheets
//!   P-3, P-4 and P-6, the preview's automatic-sharing refusal) are absent.
//!
//! Placeholders are `{tool}`, `{host}`, `{pay_range}`, `{count}`, `{folder}`,
//! `{name}`, `{max}`, `{selected}`, `{total}` and `{tools}`; the shell fills
//! them and adds nothing else.

/// Every placeholder name the table uses, each written `{name}` in a string.
pub const PLACEHOLDERS: &[&str] = &[
    "tool",
    "host",
    "pay_range",
    "count",
    "folder",
    "name",
    "max",
    "selected",
    "total",
    "tools",
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
    pub invite_error: &'static str,
    pub passkey_eyebrow: &'static str,
    pub passkey_text: &'static str,
    /// `{name}`: the passkey's name.
    pub passkey_ready: &'static str,
    pub passkey_create: &'static str,
    pub passkey_done: &'static str,
    pub near_ai_eyebrow: &'static str,
    pub near_ai_text: &'static str,
    pub near_ai_sign_in: &'static str,
    pub signed_in: &'static str,
    pub no_sharing: &'static str,
    pub skip_note: &'static str,
    pub skip: &'static str,
    /// After the passkey verification is cancelled (`ftux-page.tsx`).
    pub signed_out: &'static str,
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
    pub not_installed: &'static str,
    /// Discovery returned no row the shell could read.
    pub discovery_failed: &'static str,
    /// Runs discovery again after `discovery_failed`.
    pub retry: &'static str,
    /// Enroll was refused after the invite was looked up and accepted. The
    /// daemon does not say why, so this names no cause.
    pub enroll_refused: &'static str,
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
    /// `{count}`: sessions found for a tool.
    pub session_count: &'static str,
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
    pub session_count: &'static str,
    pub rule_for: &'static str,
    pub past_sessions: &'static str,
    pub selected_summary: &'static str,
    pub folder_selected: &'static str,
    pub include_every: &'static str,
    pub show_all: &'static str,
    pub show_fewer: &'static str,
    pub never_count: &'static str,
    pub never_label: &'static str,
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
            passkey_eyebrow: "Sign in with a passkey",
            passkey_text: "Create a passkey that can be connected later.",
            passkey_ready: "“{name}” is ready. Connect it to near.ai any time.",
            passkey_create: "Create passkey",
            passkey_done: "Done",
            near_ai_eyebrow: "Sign in with near.ai",
            near_ai_text: "Use the login you already have. Credits land in that account.",
            near_ai_sign_in: "Sign in",
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
            not_installed: "Install it, then this row asks again.",
            discovery_failed: "Could not look for coding tools on this Mac.",
            retry: "Look again",
            enroll_refused: "Your invite was found, but joining with it did not go through. Press Continue to try again.",
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
            session_count: "{count} sessions",
        },
        rules: RulesCopy {
            title_light: "Set your ",
            title_bold: "rules and permissions.",
            loading: "Reading repos from your sessions…",
            empty: "No repos to set rules for yet. Rules appear for repos found in the sessions of a tool you watch.",
            repos_found: "Repos found in {tools} sessions",
            session_count: "{count} sessions",
            rule_for: "Rule for {folder}",
            past_sessions: "Past sessions, by folder",
            selected_summary: "{selected} of {total} selected",
            folder_selected: "{selected} of {total}",
            include_every: "Include every past session in {folder}",
            show_all: "Show all {count}",
            show_fewer: "Show fewer",
            never_count: "{count} · rule is Never",
            never_label: "{folder}: rule is Never",
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
