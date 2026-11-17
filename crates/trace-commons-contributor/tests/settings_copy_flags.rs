//! The Settings copy audit's flags (2026-11-04): text that named a control
//! or step the screen does not show, stale words, and keys one string
//! shared between a title and a button, or between two buttons that need
//! different accessible names. Each test pins one fix.

use trace_commons_contributor::{
    account_change_copy, consent_copy, first_run_copy, preview_copy, private_inference_copy,
    project_copy, shell_words_copy, source_copy, witness_copy,
};

#[test]
fn settings_sharing_failures_do_not_name_setup() {
    let uses = first_run_copy::first_run_copy().uses;
    for line in [
        uses.settings_sharing_refused,
        uses.settings_sharing_save_failed,
    ] {
        assert!(!line.is_empty());
        assert!(!line.contains("Setup"), "names setup: {line}");
        assert!(
            !line.contains("Start sharing"),
            "names setup's button: {line}"
        );
    }
    // Setup keeps its own lines.
    assert!(uses.sharing_refused.contains("Setup finished"));
    assert!(uses.sharing_save_failed.contains("Start sharing"));
}

#[test]
fn folder_mode_is_named_never_not_ignore() {
    let uses = first_run_copy::first_run_copy().uses;
    // The scope line no longer names a folder mode at all since the owner's
    // Settings copy pass 2 (P120); it must still never say "Ignore".
    assert!(!uses.automatic_scope.contains("Ignore"));
    let title = project_copy::ignore_project_title("acme");
    assert!(title.contains("Never"), "{title}");
    assert!(title.contains("acme"), "{title}");
    assert!(!title.contains("Ignore"), "{title}");
}

#[test]
fn uses_save_failure_names_refresh() {
    let line = source_copy::source_settings_copy().consent_save_failed;
    // Owner dropped the pointer at Refresh 2026-11-04 (Settings copy pass 2):
    // the control beside it is the fix. It must still not say "retry",
    // which Uses has no control for.
    assert!(!line.to_lowercase().contains("retry"), "{line}");
}

#[test]
fn settings_core_down_line_claims_no_last_report() {
    let screens = preview_copy::monitor_screens_copy();
    assert!(!screens.settings_core_unreachable.contains("last reported"));
    assert!(preview_copy::MONITOR_CORE_UNREACHABLE.starts_with(screens.settings_core_unreachable));
}

#[test]
fn add_login_failure_does_not_say_switch_accounts() {
    let copy = account_change_copy::account_change_copy();
    assert!(!copy.payouts_needs_login_add_failed.contains("switch"));
    assert!(copy.payouts_needs_login_add_failed.contains("login"));
    assert_ne!(copy.payouts_needs_login_add_failed, copy.error_unavailable);
}

#[test]
fn stale_words_are_gone() {
    assert_eq!(private_inference_copy::STATE_UNKNOWN, "Status unavailable.");
    // The product name is out of this copy; the capture prerequisite stays.
    assert!(!witness_copy::WITNESS_TOKEN_CAPTURE_NOTE.contains("IronWire"));
    assert!(witness_copy::WITNESS_TOKEN_CAPTURE_NOTE.contains("request capture"));
    let screens = preview_copy::monitor_screens_copy();
    assert_ne!(screens.shell.watcher_eyebrow, "Daemon");
    assert!(!screens.flow_map.style_caption.contains("GUI"));
}

#[test]
fn signed_out_notice_button_says_sign_in() {
    assert_eq!(preview_copy::monitor_screens_copy().sign_in, "Sign in");
}

#[test]
fn titles_and_buttons_have_their_own_keys() {
    let change = account_change_copy::account_change_copy();
    assert_ne!(change.step_current_button, change.step_current);
    assert_ne!(change.step_new_button, change.step_new);
    // Each step's accessible name still includes its visible button word.
    assert!(
        change
            .step_current_accessibility_name
            .contains(change.step_current_button)
    );
    assert!(
        change
            .step_new_accessibility_name
            .contains(change.step_new_button)
    );

    let profile = shell_words_copy::public_profile_words();
    assert!(!profile.go_public_open.is_empty());
    assert!(!profile.go_public.is_empty());
}

#[test]
fn repeated_buttons_have_distinct_accessible_names() {
    let shell = preview_copy::monitor_screens_copy().shell;
    let refresh = [shell.settings_refresh_desktop, shell.settings_refresh_uses];
    for name in refresh {
        assert!(name.starts_with(shell.settings_refresh), "{name}");
        assert_ne!(name, shell.settings_refresh);
    }
    assert_eq!(
        refresh
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        2
    );

    let witness = witness_copy::witness_copy();
    let names = [
        witness.inference_enable_accessibility_name,
        witness.inference_disable_accessibility_name,
        witness.token_enable_accessibility_name,
        witness.token_disable_accessibility_name,
    ];
    assert_eq!(
        names
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        4
    );
    assert!(names[0].starts_with(witness.inference_enable));
    assert!(names[1].starts_with(witness.inference_disable));
    assert!(names[2].starts_with(witness.token_enable));
    assert!(names[3].starts_with(witness.token_disable));

    let devices = consent_copy::account_devices_copy();
    assert_ne!(
        devices.unlink_confirm_accessibility_name,
        devices.unlink_accessibility_name
    );
    assert!(
        devices
            .unlink_confirm_accessibility_name
            .contains("{device}")
    );

    let profile = shell_words_copy::public_profile_words();
    assert!(
        profile
            .withdraw_accessibility_name
            .starts_with(profile.withdraw)
    );
    assert_ne!(profile.withdraw_accessibility_name, profile.withdraw);

    let source = source_copy::source_settings_copy();
    assert!(source.tool_action_accessibility_name.contains("{tool}"));
    assert!(source.tool_action_accessibility_name.contains("{action}"));
}

#[test]
fn settings_words_formerly_spelled_in_the_shell_are_in_the_core() {
    let screens = preview_copy::monitor_screens_copy();
    assert_eq!(screens.shell.updates_eyebrow, "Updates");
    let evidence = source_copy::source_settings_copy().evidence;
    assert_eq!(evidence.missing, "Not found on this machine");
    assert_eq!(evidence.count, "{count} traces");
    assert_eq!(evidence.most_recent, "{traces}, most recent {age}");
}
