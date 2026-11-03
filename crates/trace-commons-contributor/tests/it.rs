//! Every integration test of `trace-commons-contributor`, built as one test binary.
//!
//! Each `tests/<name>.rs` is a module here, so its tests are named
//! `<name>::<test>`; run one file's tests with
//! `cargo test -p trace-commons-contributor --test it -- <name>::`.
//! Before, each file was its own binary, and every one of them linked the
//! whole dependency graph again. A file that cannot share a process with
//! the others is its own `[[test]]` in Cargo.toml instead, with the reason
//! beside it. The package sets `autotests = false`, so a file that is in
//! neither place would never be built; `wiring` refuses that.

mod antigravity_fixture_hygiene;
mod channel_hosts;
mod contribute_script;
mod credential_store_isolation;
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
mod preflight;
mod queue_fixture_default_guard;
mod release_pipeline;
mod update_conformance;
mod update_manifest_roundtrip;
mod version_channel;

mod wiring {
    /// A `tests/*.rs` file that is neither a module of this binary nor
    /// its own `[[test]]` target compiles nowhere and runs nowhere, and
    /// nothing else would say so.
    #[test]
    fn every_test_file_is_built() {
        let root = include_str!("it.rs");
        let manifest = include_str!("../Cargo.toml");
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
        let mut unbuilt = Vec::new();
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_none_or(|ext| ext != "rs") {
                continue;
            }
            let stem = path.file_stem().unwrap().to_str().unwrap().to_owned();
            if stem == "it" {
                continue;
            }
            let module = format!("\nmod {stem};\n");
            let target = format!("path = \"tests/{stem}.rs\"");
            if !root.contains(&module) && !manifest.contains(&target) {
                unbuilt.push(stem);
            }
        }
        unbuilt.sort();
        assert!(
            unbuilt.is_empty(),
            "no test target builds these tests/*.rs files: {unbuilt:?}. Add `mod <name>;` to \
             tests/it.rs, or give the file its own [[test]] in Cargo.toml with the reason"
        );
    }
}
