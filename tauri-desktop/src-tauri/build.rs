#[cfg(target_os = "macos")]
use std::{env, path::PathBuf, process::Command};

const TAURI_COMMANDS: &[&str] = &[
    "core_status",
    "queue_outcome_line",
    "residual_secret_line",
    "redaction_summary_copy",
    "project_ignore_copy",
    "arming_offer_copy",
    "attestation_copy",
    "certificate_copy",
    "eligibility_copy",
    "eligibility_group_copy",
    "daemon_call",
    "retry_daemon_startup",
    "preview_entry",
    "dismiss_entry",
    "approve_entry",
    "approve_project",
    "publish_profile",
    "withdraw_profile",
    "pause_daemon",
    "resume_daemon",
    "arming_suggestion",
    "accept_arming",
    "decline_arming",
    "cancel_entry",
    "cancel_project",
    "preview_body",
    "preview_turns",
    "search_original",
    "set_quiescence_minutes",
    "set_approval_hold_seconds",
    "set_digest_hours",
    "set_max_uploads_per_day",
    "set_max_bytes_per_day_mb",
    "witness_status",
    "configure_witness",
    "clear_witness",
    "discover_routing",
    "configure_routing",
    "probe_routing",
    "probe_routed_tools",
    "consent_options",
    "scrubber_pattern_names",
    "enroll_with_invite",
    "set_consent_scopes",
    "acknowledge_near_ai_notice",
    "set_private_inference",
    "set_inference_evidence",
    "set_token_contribution",
    "set_token_capture",
    "token_storage_status",
    "clean_token_storage",
    "set_source_declaration",
    "set_project_mode",
    "list_projects",
    "list_audit",
    "start_private_ai_credential",
    "private_ai_credential_status",
    "private_ai_balance",
    "private_ai_funding",
    "private_ai_harnesses",
    "plan_harness",
    "commit_harness",
    "open_external_url",
    "platform_capabilities",
    "notification_permission",
    "request_notification_permission",
    "set_start_at_login",
    "open_system_settings",
    "quit_app",
    "cancel_private_ai_credential",
    "forget_private_ai_credential",
    "native_wallet_flow",
    "contributor_disclosure_copy",
    "witness_review_copy",
    "near_ai_account_enroll",
    "prepare_admission_session",
    "witness_preview_support",
    "witness_preview_request",
    "history_detail",
    "request_history_refresh",
    "account_session_status",
    "account_sign_in",
    "withdrawal_confirmation_prompt",
    "withdraw_history",
    "validate_public_run_editor",
    "publish_public_run",
    "unpublish_public_run",
    "skill_learning_copy",
    "skill_candidate",
    "skill_review",
    "skill_evaluate",
    "skill_install_plan",
    "skill_install_commit",
    "skill_install_status",
    "skill_install_rollback",
    "insights_list",
    "insights_summary",
    "analyze_insight",
    "explain_insight",
    "delete_insight",
    "annotate_insight",
    "clear_insight_annotation",
    "link_test_report",
    "link_git_insight",
    "unlink_insight_evidence",
    "pick_directory",
    "consume_deep_link",
    "open_native_wallet_url",
    "open_account_sign_in_url",
    "insights_question_cards",
    "insights_episode_list",
    "insights_episode_create",
    "insights_episode_explain",
    "insights_episode_annotate",
    "insights_episode_clear_assessment",
    "insights_episode_delete",
    "comparison_task_create",
    "comparison_task_list",
    "comparison_task_explain",
    "comparison_task_replace_episodes",
    "comparison_task_set_context",
    "comparison_task_set_outcome",
    "comparison_task_clear_outcome",
    "comparison_task_reconfirm",
    "comparison_specification_list",
    "comparison_specification_preview",
    "comparison_specification_save",
    "comparison_specification_evaluate",
    "mission_draft_list",
    "mission_draft_show",
    "mission_draft_import",
    "mission_draft_delete",
    "compute_status",
    "enable_compute",
    "resume_compute",
    "pause_compute",
    "disable_compute",
];

fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(TAURI_COMMANDS)),
    )
    .expect("failed to generate Tauri command permissions");

    #[cfg(target_os = "macos")]
    build_macos_native_bridge();
}

#[cfg(target_os = "macos")]
fn build_macos_native_bridge() {
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set by Cargo"));
    let object = out_dir.join("trace_commons_macos.o");
    let archive = out_dir.join("libtrace_commons_macos.a");
    let target = env::var("TARGET").expect("TARGET is set by Cargo");
    let architecture = match target.as_str() {
        "aarch64-apple-darwin" => "arm64",
        "x86_64-apple-darwin" => "x86_64",
        _ => panic!("unsupported macOS target for native bridge: {target}"),
    };

    println!("cargo:rerun-if-changed=native_macos.m");

    let status = Command::new("clang")
        .args([
            "-fobjc-arc",
            "-fblocks",
            "-arch",
            architecture,
            "-mmacosx-version-min=13.0",
            "-c",
            "native_macos.m",
            "-o",
        ])
        .arg(&object)
        .status()
        .expect("failed to start clang for macOS native bridge");
    assert!(
        status.success(),
        "clang failed to build macOS native bridge"
    );

    let status = Command::new("ar")
        .args(["-rcs"])
        .arg(&archive)
        .arg(&object)
        .status()
        .expect("failed to archive macOS native bridge");
    assert!(status.success(), "ar failed to archive macOS native bridge");

    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=static=trace_commons_macos");
    println!("cargo:rustc-link-lib=framework=Foundation");
    println!("cargo:rustc-link-lib=framework=AppKit");
    println!("cargo:rustc-link-lib=framework=ServiceManagement");
    println!("cargo:rustc-link-lib=framework=UserNotifications");
}
