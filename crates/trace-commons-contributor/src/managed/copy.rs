//! Shared wording for every managed-session shell.
//!
//! The buttons `launch`, `use_default`, `reconnect` and `save_login`:
//! Approved 2026-10-08 (button rule). `launch_title` keeps the launch dialog's full title.
//!
//! Sessions on the NEAR AI sign-in. Approved 2026-10-12:
//! - `start_claude` "Start Claude Code"
//! - `start_codex` "Start Codex"
//! - `start_opencode` "Start OpenCode" -- Approved 2026-10-16. Shown only
//!   while the snapshot's `tools` row for `opencode` says it is installed.
//! - `sign_in_scope` "Uses your NEAR AI sign-in. Runs in {destination}, in the
//!   folder you choose." Shells fill `{destination}` from
//!   `capabilities.terminal_destination`, as for `launch_scope`, so Windows
//!   never reads "Terminal".
//! - `open_tool` "Open {tool}" -- the launch dialog's button once a folder
//!   is chosen (owner, 2026-10-19: "Open Claude"). Shells fill `{tool}`
//!   with the tool's name.
//! - `sign_in_needed` "Sign in to NEAR AI to start a session." Also the line
//!   a shell shows for the refusal `managed-near-ai-sign-in-required`.
//! - `sign_in_near_ai` "Sign in" -- the button that starts the NEAR AI
//!   sign-in.
//! - `sign_in_near_ai_accessibility` "Sign in to NEAR AI" -- its accessible
//!   name.
//!
//! The terminal choice, in Settings > Private AI on every shell. Approved by
//! the design owner 2026-10-14 (#1406):
//! - `terminal_choice` "Open sessions in" -- the Settings picker's label,
//!   and its accessible name. Its options are the app names the snapshot
//!   sends in `capabilities.terminal_choices`.
//! - `terminal_not_found` "{terminal} wasn't found on this computer.
//!   Choose another in Settings." The line a shell shows for the refusal
//!   `managed-terminal-not-found` ("not found", never "not installed");
//!   it fills `{terminal}` from `capabilities.terminal_destination`.
//!
//! A session row's actions (Ron, #1407, 2026-10-14):
//! - `show` "Show" and `stop` "Stop" -- the row menu's items and the row's
//!   accessibility actions. Clicking a running row also shows it.
//! - `stop_question` "Stop this session?" and `stop_description` "{tool}
//!   stops what it's doing in {folder}." -- the confirmation, with `cancel`
//!   and `stop`. Shells fill `{tool}` with the tool's name and `{folder}`
//!   with the row's folder name.
//! - `start_again` "Start again" -- an ended row's glass button, before
//!   `dismiss` as a link.
//!
//! Installing a coding tool. Approved 2026-10-15 (#1419 review):
//! - `tools_title` "Coding tools", above the install block shown when no
//!   coding tool is installed.
//! - `tools_empty` "Install a coding tool to get started."
//! - `install` "Install"; its accessible name appends the tool's name.
//! - `tool_unavailable` "{tool} wasn't found on this computer." Shells fill
//!   `{tool}` with the tool's display name (`claude`, `codex`, `opencode`).
//! - `refresh` "Refresh": a bare glyph's tooltip and accessible name.
//!
//! The explanation before the folder picker (#1408). Approved 2026-10-14:
//! - `folder_choice` "Choose a folder for {tool} to work in. It opens in
//!   {destination}." Shells fill `{tool}` with the tool's name key and
//!   `{destination}` from `capabilities.terminal_destination`, as for
//!   `sign_in_scope`. Its title is the Start button's own label, and its
//!   buttons are `cancel` and `choose_folder`.
//!
//! Choosing the model of a NEAR AI session. Proposed 2026-10-16, not yet
//! approved:
//! - `model_choice` "Model" -- the Launch sheet's model picker label and
//!   accessible name, for Claude Code and Codex on a saved NEAR AI account.
//! - `model_default` "Default: {model}" -- the picker's first option, which
//!   names no model and lets the session take the tool's default. Shells
//!   fill `{model}` from `managed_model_catalogue`'s `default`; without one
//!   they show `model_default_unknown` "Default".
//! - `model_unavailable` "That model isn't available on NEAR AI. Choose
//!   another." The line a shell shows for the refusal
//!   `managed-model-unavailable`.
pub fn copy() -> serde_json::Value {
    serde_json::from_str(r#"{
            "title": "Managed sessions",
            "collection_account_busy": "Finish the existing session for this account before starting one with trace collection.",
            "collect_managed_trace": "Collect trace for review",
            "managed_collection_scope": "Saves this session’s trace locally after it ends. Your existing review and contribution settings still apply.",
            "folder_settings_scope": "Model, CLI options and trace collection are remembered for this folder and agent.",
            "folder_settings_loading": "Loading folder settings…",
            "folder_settings_failed": "Could not load folder settings. Try again.",
            "folder_settings_reset": "Reset",
            "folder_settings_reset_accessibility": "Reset {tool} folder settings",
            "folder_model_unavailable": "The saved model is unavailable. Choose a model before launching.",
            "cli_options_title": "Agent CLI options",
            "cli_options_scope": "Applies to new managed sessions. Quote values that contain spaces. Account routing, profiles and session identity are managed by Trace Commons.",
            "cli_options_label": "CLI options",
            "cli_options_save": "Save",
            "cli_options_save_accessibility": "Save {tool} CLI options",
            "cli_options_invalid": "Check quotes and escapes. Account routing, profile and session overrides are unavailable here; choose the OpenCode model when launching.",
            "cli_options_unavailable": "Update Trace Commons to edit managed session options.",
            "cli_options_read_failed": "Could not read managed session options. Try Refresh.",
            "provider_openai": "OpenAI",
            "provider_anthropic": "Anthropic",
            "provider_near_ai": "NEAR AI",
            "collection_off": "Trace collection off",
            "map_scope": "Session routes; model calls are unverified.",
            "collection_disabled": "Trace collection off",
            "model_required": "Enter a model ID to launch OpenCode.",
            "tools_title": "Coding tools",
            "tools_empty": "Install a coding tool to get started.",
            "install": "Install",
            "install_in_terminal": "Open {terminal} and run the official installer for {tool}.",
            "update_in_terminal": "Open {terminal} and update {tool}.",
            "tool_unavailable": "{tool} wasn't found on this computer.",
            "unsupported_version": "This tool version is not supported. Install a supported version, then try again.",
            "update": "Update",
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
            "model_choice": "Model",
            "model_default": "Default: {model}",
            "model_default_unknown": "Default",
            "model_unavailable": "That model isn't available on NEAR AI. Choose another.",
            "collect_traces": "Collect this session",
            "opencode_scope": "Uses only the selected key and model in an isolated profile. Project OpenCode settings and plugins are disabled. API key means an OpenAI key; NEAR AI uses a NEAR AI key.",
            "collection_scope": "Collects this session when OpenCode exits, under your existing review and contribution settings. Other sessions created inside OpenCode are not collected.",
            "collection_pending": "Trace collection pending",
            "collection_collected": "Trace saved locally",
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
            "folder_choice": "Choose a folder for {tool} to work in. It opens in {destination}.",
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
            "start_opencode": "Start OpenCode",
            "open_tool": "Open {tool}",
            "sign_in_scope": "Uses your NEAR AI sign-in. Runs in {destination}, in the folder you choose.",
            "sign_in_needed": "Sign in to NEAR AI to start a session.",
            "sign_in_near_ai": "Sign in",
            "sign_in_near_ai_accessibility": "Sign in to NEAR AI",
            "terminal_choice": "Open sessions in",
            "terminal_not_found": "{terminal} wasn't found on this computer. Choose another in Settings.",
            "show": "Show",
            "stop": "Stop",
            "stop_question": "Stop this session?",
            "stop_description": "{tool} stops what it's doing in {folder}.",
            "start_again": "Start again"
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

    /// #1408: the line before the folder picker, as approved 2026-10-14.
    #[test]
    fn the_folder_choice_line_has_its_words() {
        assert_eq!(
            copy()["folder_choice"],
            "Choose a folder for {tool} to work in. It opens in {destination}."
        );
    }

    /// The NEAR AI sign-in launch reads these words, exactly as proposed.
    #[test]
    fn the_sign_in_launch_has_its_words() {
        let copy = copy();
        assert_eq!(copy["start_claude"], "Start Claude Code");
        assert_eq!(copy["start_codex"], "Start Codex");
        assert_eq!(copy["start_opencode"], "Start OpenCode");
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

    /// The terminal choice reads these words, exactly as proposed.
    #[test]
    fn the_terminal_choice_has_its_words() {
        let copy = copy();
        assert_eq!(copy["terminal_choice"], "Open sessions in");
        assert_eq!(
            copy["terminal_not_found"],
            "{terminal} wasn't found on this computer. Choose another in Settings."
        );
    }

    /// Show, Stop and Start again on a session row read these words
    /// (Ron, #1407, 2026-10-14).
    #[test]
    fn the_session_row_actions_have_their_words() {
        let copy = copy();
        assert_eq!(copy["show"], "Show");
        assert_eq!(copy["stop"], "Stop");
        assert_eq!(copy["stop_question"], "Stop this session?");
        assert_eq!(
            copy["stop_description"],
            "{tool} stops what it's doing in {folder}."
        );
        assert_eq!(copy["start_again"], "Start again");
        assert_eq!(copy["cancel"], "Cancel");
        assert_eq!(copy["dismiss"], "Dismiss");
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
