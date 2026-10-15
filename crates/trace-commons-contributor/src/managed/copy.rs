//! Shared wording for every managed-session shell.
//!
//! The buttons `launch`, `use_default`, `reconnect` and `save_login`:
//! Approved 2026-10-08 (button rule). `launch_title` keeps the launch dialog's full title.
//!
//! Sessions on the NEAR AI sign-in. Approved 2026-10-12:
//! - `start_claude` "Start Claude Code"
//! - `start_codex` "Start Codex"
//! - `sign_in_scope` "Uses your NEAR AI sign-in. Runs in {destination}, in the
//!   folder you choose." Shells fill `{destination}` from
//!   `capabilities.terminal_destination`, as for `launch_scope`, so Windows
//!   never reads "Terminal".
//! - `sign_in_needed` "Sign in to NEAR AI to start a session." Also the line
//!   a shell shows for the refusal `managed-near-ai-sign-in-required`.
//! - `sign_in_near_ai` "Sign in" -- the button that starts the NEAR AI
//!   sign-in.
//! - `sign_in_near_ai_accessibility` "Sign in to NEAR AI" -- its accessible
//!   name.
//!
//! Installing a coding tool. Approved 2026-10-15 (#1419 review):
//! - `tools_title` "Coding tools", above the install block shown when no
//!   coding tool is installed.
//! - `tools_empty` "Install a coding tool to get started."
//! - `install` "Install"; its accessible name appends the tool's name.
//! - `tool_unavailable` "{tool} wasn't found on this computer." Shells fill
//!   `{tool}` with the tool's display name (`claude`, `codex`, `opencode`).
//! - `refresh` "Refresh": a bare glyph's tooltip and accessible name.
pub fn copy() -> serde_json::Value {
    serde_json::from_str(r#"{
            "title": "Managed sessions",
            "tools_title": "Coding tools",
            "tools_empty": "Install a coding tool to get started.",
            "install": "Install",
            "tool_unavailable": "{tool} wasn't found on this computer.",
            "description": "Start a session with a saved account. Your other sessions keep running.",
            "launch": "Launch",
            "launch_title": "Launch managed session",
            "add": "Add account",
            "refresh": "Refresh",
            "terminal_scope": "Opens in {destination}. Launches from the CLI stay in their terminal.",
            "terminal_unavailable": "Can't open a terminal from this build. Run near-ai in your terminal instead.",
            "empty": "No sessions yet.",
            "sign_in": "Sign-in",
            "exit_code": "Exit code {code}",
            "dismiss": "Dismiss",
            "connecting": "Connecting…",
            "retry": "Retry",
            "rename": "Rename account",
            "label": "Name",
            "save": "Save",
            "cancel": "Cancel",
            "remove_question": "Remove this account?",
            "remove": "Remove",
            "remove_description": "Its saved sign-in and profile are deleted from this computer.",
            "manage": "More",
            "default": "Default",
            "use_default": "Make default",
            "rename_short": "Rename",
            "reconnect": "Sign in",
            "verify": "Check sign-in",
            "near_ai": "NEAR AI",
            "api_key": "API key",
            "openai_api_key": "OpenAI API key",
            "subscription": "Subscription",
            "save_title": "Add account",
            "tool": "Tool",
            "claude": "Claude Code",
            "codex": "Codex",
            "opencode": "OpenCode",
            "model": "Model ID",
            "collect_traces": "Collect this session",
            "opencode_scope": "Uses only the selected key and model in an isolated profile. Project OpenCode settings and plugins are disabled. API key means an OpenAI key; NEAR AI uses a NEAR AI key.",
            "collection_scope": "Collects this session when OpenCode exits, under your existing review and contribution settings. Other sessions created inside OpenCode are not collected.",
            "collection_pending": "Trace collection pending",
            "collection_collected": "Trace collected",
            "collection_failed": "Trace collection failed; local session preserved",
            "connection": "Sign-in type",
            "label_placeholder": "e.g. Personal or Work",
            "login_description": "You'll sign in through the tool in Terminal.",
            "key_storage": "Stored in this computer's credential store.",
            "save_login": "Sign in",
            "save_account": "Save",
            "saved_account": "Account",
            "choose_account": "Choose account",
            "project_placeholder": "No folder chosen",
            "choose_folder": "Choose folder",
            "launch_scope": "Opens in {destination}.",
            "launch_short": "Launch",
            "terminal": "your terminal",
            "global_title": "Standard settings",
            "global_scope": "These apply to standard sessions once you restart the tool.",
            "launch_unknown": "Refresh, then try again.",
            "action_failed": "That didn't work. Try again.",
            "accounts_title": "Saved accounts",
            "accounts_refresh_accessibility": "Refresh saved accounts",
            "state_starting": "Starting",
            "state_running": "Running",
            "state_exited": "Exited",
            "state_failed": "Failed",
            "state_unknown": "Unknown",
            "auth_sign_in_required": "Needs sign-in",
            "auth_checking": "Checking",
            "auth_ready": "Ready",
            "auth_unavailable": "Unavailable",
            "start_claude": "Start Claude Code",
            "start_codex": "Start Codex",
            "sign_in_scope": "Uses your NEAR AI sign-in. Runs in {destination}, in the folder you choose.",
            "sign_in_needed": "Sign in to NEAR AI to start a session.",
            "sign_in_near_ai": "Sign in",
            "sign_in_near_ai_accessibility": "Sign in to NEAR AI"
    }"#).expect("static managed copy is valid JSON")
}

#[cfg(test)]
mod tests {
    use super::copy;
    use crate::managed::{AuthState, sessions::SessionState};

    fn key(prefix: &str, value: impl serde::Serialize) -> String {
        let value = serde_json::to_value(value).expect("enum serializes");
        format!("{prefix}_{}", value.as_str().expect("unit variant"))
    }

    /// The saved accounts' refresh icon says what it re-reads to assistive
    /// tech, as the core's words rather than a shell's join.
    #[test]
    fn the_accounts_refresh_has_its_own_accessible_name() {
        let copy = copy();
        assert_eq!(
            copy["accounts_refresh_accessibility"],
            "Refresh saved accounts"
        );
    }

    /// The NEAR AI sign-in launch reads these words, exactly as proposed.
    #[test]
    fn the_sign_in_launch_has_its_words() {
        let copy = copy();
        assert_eq!(copy["start_claude"], "Start Claude Code");
        assert_eq!(copy["start_codex"], "Start Codex");
        assert_eq!(
            copy["sign_in_scope"],
            "Uses your NEAR AI sign-in. Runs in {destination}, in the folder you choose."
        );
        assert_eq!(
            copy["sign_in_needed"],
            "Sign in to NEAR AI to start a session."
        );
        assert_eq!(copy["sign_in_near_ai"], "Sign in");
        assert_eq!(copy["sign_in_near_ai_accessibility"], "Sign in to NEAR AI");
    }

    /// Every lifecycle and sign-in state a shell can be sent has a label,
    /// so no shell draws the raw wire value.
    #[test]
    fn every_session_and_auth_state_has_a_label() {
        let copy = copy();
        for state in [
            SessionState::Starting,
            SessionState::Running,
            SessionState::Exited,
            SessionState::Failed,
            SessionState::Unknown,
        ] {
            let key = key("state", state);
            assert!(copy[&key].is_string(), "missing {key}");
        }
        for state in [
            AuthState::SignInRequired,
            AuthState::Checking,
            AuthState::Ready,
            AuthState::Unavailable,
        ] {
            let key = key("auth", state);
            assert!(copy[&key].is_string(), "missing {key}");
        }
    }

    /// Ron's 2026-10-15 review of #1419: the install words, with `{tool}`
    /// filled with the tool's display name by every shell.
    #[test]
    fn the_install_words_are_the_approved_ones() {
        let copy = copy();
        assert_eq!(copy["tools_title"], "Coding tools");
        assert_eq!(copy["tools_empty"], "Install a coding tool to get started.");
        assert_eq!(
            copy["tool_unavailable"],
            "{tool} wasn't found on this computer."
        );
        assert_eq!(copy["install"], "Install");
        assert_eq!(copy["refresh"], "Refresh");
        for removed in ["opencode_export_scope", "not_installed", "check_again"] {
            assert!(copy.get(removed).is_none(), "{removed} should be gone");
        }
    }
}
