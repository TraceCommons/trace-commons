//! Shared wording for every managed-session shell.
//!
//! The buttons `launch`, `use_default`, `reconnect` and `save_login`:
//! Approved 2026-10-08 (button rule). `launch_title` keeps the launch dialog's full title.
pub fn copy() -> serde_json::Value {
    serde_json::from_str(r#"{
            "title": "Managed sessions",
            "provider_openai": "OpenAI",
            "provider_anthropic": "Anthropic",
            "provider_near_ai": "NEAR AI",
            "collection_off": "Trace collection off",
            "map_scope": "Session routes; model calls are unverified.",
            "collection_disabled": "Trace collection off",
            "model_required": "Enter a model ID to launch OpenCode.",
            "opencode_export_scope": "Watch local sessions, import an export, or collect a managed session.",
            "tools_title": "Coding tools",
            "tools_empty": "Install a coding tool to start a managed session. Saved accounts and session history stay here.",
            "install": "Install",
            "check_again": "Check again",
            "unsupported_version": "This tool version is not supported. Install a supported version, then try again.",
            "update": "Update",
            "not_installed": "Not installed",
            "tool_unavailable": "This tool is not installed or is unavailable. Install it, then choose Check again.",
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
            "state_starting": "Starting",
            "state_running": "Running",
            "state_exited": "Exited",
            "state_failed": "Failed",
            "state_unknown": "Unknown",
            "auth_sign_in_required": "Needs sign-in",
            "auth_checking": "Checking",
            "auth_ready": "Ready",
            "auth_unavailable": "Unavailable"
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
}
