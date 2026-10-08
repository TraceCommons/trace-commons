//! Shared wording for every managed-session shell.
//!
//! The buttons `launch`, `use_default`, `reconnect` and `save_login`:
//! Approved 2026-10-08 (button rule). `launch_title` keeps the launch dialog's full title.
pub fn copy() -> serde_json::Value {
    serde_json::from_str(r#"{
            "title": "Managed sessions",
            "description": "Choose a saved account for each new session. Standard sessions can keep running alongside it.",
            "launch": "Launch",
            "launch_title": "Launch managed session",
            "add": "Add account",
            "refresh": "Refresh",
            "terminal_scope": "App launches open in {destination}. CLI launches stay in their current terminal.",
            "terminal_unavailable": "Terminal launch is unavailable in this build. Use near-ai in your terminal.",
            "empty": "No managed sessions yet.",
            "sign_in": "Sign in",
            "exit_code": "Exit code: {code}",
            "dismiss": "Dismiss",
            "connecting": "Connecting to managed sessions…",
            "retry": "Retry",
            "rename": "Rename account",
            "label": "Account label",
            "save": "Save",
            "cancel": "Cancel",
            "remove_question": "Remove saved account?",
            "remove": "Remove account",
            "remove_description": "This removes the saved credentials and isolated profile from this computer.",
            "manage": "Manage",
            "default": "Default",
            "use_default": "Make default",
            "rename_short": "Rename",
            "reconnect": "Reconnect",
            "verify": "Verify account",
            "near_ai": "NEAR AI",
            "api_key": "API key",
            "subscription": "Subscription",
            "save_title": "Save an account",
            "tool": "Tool",
            "claude": "Claude Code",
            "codex": "Codex",
            "connection": "Connection",
            "label_placeholder": "Label, such as Personal or Work",
            "login_description": "Sign in through the native tool in Terminal. Repeat Add account to save another subscription.",
            "key_storage": "The key is stored in the operating system credential store.",
            "save_login": "Sign in",
            "save_account": "Save account",
            "saved_account": "Saved account",
            "choose_account": "Choose account",
            "project_placeholder": "Choose a project folder",
            "choose_folder": "Choose folder",
            "launch_scope": "Opens in {destination}. This selection applies to this launch.",
            "launch_short": "Launch",
            "terminal": "your terminal",
            "global_title": "Change global settings",
            "global_scope": "Changes below affect standard sessions after restarting the tool.",
            "launch_unknown": "Refresh sessions before retrying.",
            "action_failed": "Managed action failed.",
            "accounts_title": "Saved accounts",
            "state_starting": "Starting",
            "state_running": "Running",
            "state_exited": "Exited",
            "state_failed": "Failed",
            "state_unknown": "Unknown",
            "auth_sign_in_required": "Sign-in required",
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
