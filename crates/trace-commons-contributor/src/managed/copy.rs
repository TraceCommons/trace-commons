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
//! - `guide_title`, `guide_body`, `guide_step_earn`, `guide_step_start` and
//!   `guide_step_use` -- the Private AI tab's step-by-step card, first on
//!   the tab and closed with its X. Approved by the product owner
//!   2026-10-28, with `sign_in_title` and `title` "My Private AI sessions".
//! - `sign_in_title` "Start a Private AI session" -- the launch card's
//!   title on the Private AI tab (approved 2026-10-28).
//! - `sign_in_scope` "Uses your NEAR AI sign-in." Shortened and approved by
//!   the owner 2026-10-29 (it named the terminal and the folder). Shells
//!   still replace a `{destination}` if the line ever carries one.
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
//! - `start_again` "Start again" -- an ended row's glass button when it
//!   has no `resume`; otherwise an item in the row's menu. `dismiss` is that
//!   row's menu item too, except when it is the row's only action.
//!
//! Resuming a session (design agreed 2026-10-31,
//! `docs/superpowers/specs/2026-10-31-managed-session-resume-design.md`):
//! - `resume` "Resume" -- an ended row's primary button when the row says
//!   `can_resume` and the snapshot says `capabilities.terminal_launch` and
//!   `capabilities.resume`; `start_again` and `dismiss` then move into the
//!   row's menu. It continues the session's conversation in a new
//!   session.
//! - `copy_resume_command` "Copy resume command" -- an item in the row's
//!   menu on an ended row that says `can_resume`, shown once the shell
//!   knows the config directory it passes. Copies the
//!   command line that resumes the session from any terminal.
//! - `resumed` "Resumed" -- a small secondary label on the status line of a
//!   row that continues an earlier session (the row says `resumed_from`).
//!   `resumed_from` "Continues {time}" -- the same label with the root
//!   row's start time; shells fill `{time}` with it, formatted as they
//!   format a row's own time.
//! - `copied` "Copied" -- the row's status text for about two
//!   seconds after the "Copy resume command" menu item copies, then it
//!   reverts. An in-place label change, not a notice.
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
//! A folder is optional in the launch dialog (owner, 2026-10-29): a session
//! with none runs in the person's home folder, which the daemon resolves.
//! - `pick_folder` "Pick a folder as context" -- the dialog's folder button.
//!   Approved by the owner 2026-10-29.
//! - `home_folder` "Home folder" and `clear_folder` "Use home" --
//!   what the launch dialog and the Launch sheet show while no folder is
//!   picked, and the button that clears a picked one. Approved by the owner
//!   2026-10-29.
//!
//! `folder_choice` (#1408) is gone: the launch dialog no longer explains the
//! folder choice (owner, 2026-10-29: "way too verbose"), so no shell reads it.
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
//!
//! Favorite models. Proposed 2026-10-29, not yet approved:
//! - `model_favorite_add` "Add to favorites" and `model_favorite_remove`
//!   "Remove from favorites" -- the accessible name and tooltip of the star
//!   beside the model picker, for the chosen model, while it is not and is a
//!   favorite. Favorites are listed first in the picker.
//! - `model_favorite_unavailable` "{model} is no longer available on NEAR
//!   AI." -- a row beside the picker for each favorite NEAR AI stopped
//!   listing, with a `remove` button whose accessible name is
//!   `model_favorite_remove_named` "Remove {model} from favorites". Shells
//!   fill `{model}` with the model id.
//! - `favorites_title` "Favorite models" -- the card in Settings > Private
//!   AI that lists every favorite, each with `remove`; one NEAR AI stopped
//!   listing reads `model_favorite_unavailable` instead of its bare id.
//! - `folder_model_unavailable` changed from "The saved model is
//!   unavailable. Choose a model before launching." -- a saved model NEAR AI
//!   no longer lists now falls back to the default instead of holding the
//!   launch, and this line says so.
//!
//! The background model picker, under the model picker for Claude Code and
//! OpenCode on NEAR AI. Approved by the owner 2026-10-31:
//! - `background_model_title` "Background model" -- the field's visible
//!   label above the picker.
//! - `background_model_choice` "Model for background tasks" -- the
//!   picker's accessible name, apart from the model picker's `model_choice`.
//!   Its default row reuses `model_default` and `model_default_unknown`.
//!
//! The model picker's group headings, one per NEAR AI privacy tier
//! (`managed::models::ModelTier`). Named by the owner 2026-10-30; the
//! picker lists them most private first (owner, 2026-10-31):
//! - `model_group_near_ai` "NEAR AI private models" -- NEAR AI's own TEEs
//!   (its "Confidential TEE").
//! - `model_group_external` "External private models" -- a third party's
//!   TEE (its "3P Confidential TEE").
//! - `model_group_incognito` "Incognito models" -- a vendor's model on the
//!   vendor's servers, reached through NEAR AI's gateway (NEAR AI's
//!   term).
//!
//! Refusing a launch on the NEAR AI sign-in whose balance is too low for its
//! first calls. Proposed 2026-10-30, not yet approved:
//! - `balance_too_low` "Add funds to your NEAR AI balance to start a
//!   session." The line a shell shows for the refusal
//!   `managed-near-ai-balance-too-low`.
//!
//! A NEAR AI Claude Code launch and its project's model choices. Approved by
//! the owner with this wording, 2026-11-01:
//! - `project_chooses_model` "This project's Claude Code settings, agents,
//!   commands or skills choose a model. NEAR AI sessions use the session's
//!   models." The line a shell shows for the refusal
//!   `managed-project-chooses-model`; the CLI prints the same sentence.
//! - `project_workflows` "This project has Claude Code workflows. A workflow
//!   step can choose its own model, so it may not stay in this session's
//!   privacy tier." A notice the launcher prints in the session's terminal
//!   before Claude Code starts, on NEAR AI only; the session still starts.
//!   The macOS launch sheet also shows it as a caption under the model
//!   pickers, for a folder `managed_folder_settings_get` reports
//!   `project_workflows` for, on a NEAR AI session (owner, 2026-11-01).
pub fn copy() -> serde_json::Value {
    serde_json::from_str(r#"{
            "title": "My Private AI sessions",
            "collection_account_busy": "Finish the existing session for this account before starting one with trace collection.",
            "collect_managed_trace": "Collect trace for review",
            "managed_collection_scope": "Saves this session’s trace locally after it ends. Your existing review and contribution settings still apply.",
            "folder_settings_scope": "Model, CLI options and trace collection are remembered for this folder and agent.",
            "folder_settings_loading": "Loading folder settings…",
            "folder_settings_failed": "Could not load folder settings. Try again.",
            "folder_settings_reset": "Reset",
            "folder_settings_reset_accessibility": "Reset {tool} folder settings",
            "folder_model_unavailable": "The saved model is no longer available, so the default is selected.",
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
            "background_model_title": "Background model",
            "background_model_choice": "Model for background tasks",
            "model_default": "Default: {model}",
            "model_default_unknown": "Default",
            "model_group_incognito": "Incognito models",
            "model_group_near_ai": "NEAR AI private models",
            "model_group_external": "External private models",
            "model_unavailable": "That model isn't available on NEAR AI. Choose another.",
            "model_favorite_add": "Add to favorites",
            "model_favorite_remove": "Remove from favorites",
            "model_favorite_unavailable": "{model} is no longer available on NEAR AI.",
            "model_favorite_remove_named": "Remove {model} from favorites",
            "favorites_title": "Favorite models",
            "balance_too_low": "Add funds to your NEAR AI balance to start a session.",
            "project_chooses_model": "This project's Claude Code settings, agents, commands or skills choose a model. NEAR AI sessions use the session's models.",
            "project_workflows": "This project has Claude Code workflows. A workflow step can choose its own model, so it may not stay in this session's privacy tier.",
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
            "pick_folder": "Pick a folder as context",
            "home_folder": "Home folder",
            "clear_folder": "Use home",
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
            "guide_title": "How Private AI works",
            "guide_body": "Pay for the AI you already use with credit from your traces.",
            "guide_step_earn": "Share traces to earn NEAR AI credit.",
            "guide_step_start": "Start Claude Code, Codex or OpenCode below.",
            "guide_step_use": "Work as usual. Calls go through Private AI and use your credit.",
            "sign_in_title": "Start a Private AI session",
            "sign_in_scope": "Uses your NEAR AI sign-in.",
            "sign_in_needed": "Sign in to NEAR AI to start a session.",
            "sign_in_near_ai": "Sign in",
            "sign_in_near_ai_accessibility": "Sign in to NEAR AI",
            "terminal_choice": "Open sessions in",
            "terminal_not_found": "{terminal} wasn't found on this computer. Choose another in Settings.",
            "show": "Show",
            "stop": "Stop",
            "stop_question": "Stop this session?",
            "stop_description": "{tool} stops what it's doing in {folder}.",
            "start_again": "Start again",
            "resume": "Resume",
            "copy_resume_command": "Copy resume command",
            "resumed": "Resumed",
            "resumed_from": "Continues {time}",
            "copied": "Copied"
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
    fn the_folder_choice_line_is_gone_and_the_folder_words_are_short() {
        let copy = copy();
        assert!(copy.get("folder_choice").is_none());
        assert_eq!(copy["pick_folder"], "Pick a folder as context");
        assert_eq!(copy["home_folder"], "Home folder");
        assert_eq!(copy["clear_folder"], "Use home");
    }

    /// The NEAR AI sign-in launch reads these words, exactly as proposed.
    #[test]
    fn the_sign_in_launch_has_its_words() {
        let copy = copy();
        assert_eq!(copy["start_claude"], "Start Claude Code");
        assert_eq!(copy["start_codex"], "Start Codex");
        assert_eq!(copy["start_opencode"], "Start OpenCode");
        assert_eq!(copy["sign_in_title"], "Start a Private AI session");
        assert_eq!(copy["sign_in_scope"], "Uses your NEAR AI sign-in.");
        assert_eq!(
            copy["sign_in_needed"],
            "Sign in to NEAR AI to start a session."
        );
        assert_eq!(copy["sign_in_near_ai"], "Sign in");
        assert_eq!(copy["sign_in_near_ai_accessibility"], "Sign in to NEAR AI");
    }

    /// The star beside the model picker names its action both ways.
    #[test]
    fn the_favorite_star_has_its_words() {
        let copy = copy();
        assert_eq!(copy["model_favorite_add"], "Add to favorites");
        assert_eq!(copy["model_favorite_remove"], "Remove from favorites");
        assert_eq!(
            copy["model_favorite_unavailable"],
            "{model} is no longer available on NEAR AI."
        );
        assert_eq!(
            copy["model_favorite_remove_named"],
            "Remove {model} from favorites"
        );
        assert_eq!(copy["favorites_title"], "Favorite models");
        // A saved model NEAR AI dropped falls back to the default.
        assert_eq!(
            copy["folder_model_unavailable"],
            "The saved model is no longer available, so the default is selected."
        );
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
        assert_eq!(copy["resume"], "Resume");
        assert_eq!(copy["copy_resume_command"], "Copy resume command");
        assert_eq!(copy["resumed"], "Resumed");
        assert_eq!(copy["resumed_from"], "Continues {time}");
        assert_eq!(copy["copied"], "Copied");
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
