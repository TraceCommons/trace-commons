//! Integration tests of `trace-commons-contributor`, built as one test binary.
//!
//! Each `tests/<name>.rs` below is a module, so its tests are named
//! `<name>::<test>`; run one file's tests with
//! `cargo test -p trace-commons-contributor --test it -- '<name>::'`.
//! Before, each file was its own binary, and every one of them linked the
//! whole dependency graph again. A file that cannot share a process with
//! the others is its own `[[test]]` in Cargo.toml, with the reason beside
//! it. The package sets `autotests = false`, so a file in neither place
//! would never be built; `it_harness` refuses that.

#[macro_use]
#[path = "../../trace-commons-protocol/tests/support/it_harness.rs"]
mod it_harness;

mod antigravity_fixture_hygiene;
mod contribute_script;
mod daemon_audit_ordering;
mod daemon_cli_controls_running_daemon;
mod daemon_end_to_end_upload;
mod daemon_ipc_contract;
mod daemon_ipc_flow2_states;
mod daemon_logout_revocation;
mod daemon_nondeterministic_filter;
mod daemon_preview_body_over_socket;
mod daemon_preview_cli;
mod daemon_reads_staged_imports;
mod daemon_upload_pass_guards;
mod daemon_wallet_signup_without_env;
mod evidence_import_cli;
mod local_insights_cli;
mod local_redaction_audit;
mod macos_shell_settings_contract;
mod managed_accounts;
mod managed_cli_options;
mod managed_folder_settings;
mod managed_launch;
mod managed_native_collection;
mod managed_opencode;
mod managed_profiles;
mod managed_secrets;
mod managed_session_binding;
mod managed_sessions;
mod managed_statusline;
mod managed_terminal;
mod mission_draft_cli;
mod near_ai_cli;
mod no_commons_name_in_copy;
mod queue_fixture_default_guard;
mod release_pipeline;
mod settings_copy_flags;
mod update_conformance;
mod update_manifest_roundtrip;
