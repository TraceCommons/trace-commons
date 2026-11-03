// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Every integration test of `trace-commons-server`, built as one test binary.
//!
//! Each `tests/<name>.rs` is a module here, so its tests are named
//! `<name>::<test>`; run one file's tests with
//! `cargo test -p trace-commons-server --test it -- <name>::`.
//! Before, each file was its own binary, and every one of them linked the
//! whole dependency graph again. A file that cannot share a process with
//! the others is its own `[[test]]` in Cargo.toml instead, with the reason
//! beside it. The package sets `autotests = false`, so a file that is in
//! neither place would never be built; `wiring` refuses that.

// Several former binaries each included the same helper file (support/,
// mission_rewards_pg/fixture.rs, and the src/bin modules they test) with
// `#[path]`, and some name the module after their own file. As modules of one
// binary they keep doing exactly that, so every file compiles the same code
// and every test keeps its name; loading each shared file once is a separate
// change.
#![allow(clippy::duplicate_mod, clippy::module_inception)]

mod account_merge_pg;
mod account_onboarding_contract;
mod account_onboarding_pg;
mod account_trust_external_pg;
mod account_trust_growth_pg;
mod account_trust_live_pg;
mod account_trust_pg;
mod activity_missions_pg;
mod admission_ledger_pg;
mod audit_hash_chain_writeback;
mod bakeoff_corpus;
mod bakeoff_corpus_script;
mod bakeoff_manifest;
mod bakeoff_metrics;
mod bakeoff_report;
mod bakeoff_subcommand;
mod cloud_kms_kek_wrapper;
mod corpus_trivial_measures;
mod credit_settlement_enumerator_pg;
mod device_invite_subject_pg;
mod inference_connection_pg;
mod inference_funding;
mod legacy_invite_counts;
mod legacy_invite_link_pg;
mod license_boundary;
mod migration_atomicity_pg;
mod mission_publication_pg;
mod mission_rewards_cli_errors;
mod mission_rewards_pg;
mod model_training_consent_pg;
mod near_ai_live_receipt;
mod near_credit_contract;
mod onboarding_retention_pg;
mod payout_pass_pg;
mod payout_rails_pg;
mod pilot_bootstrap_submitter;
mod pilot_bootstrap_translators;
mod r1_pipeline_measurement;
mod referral_bonus_pg;
mod referral_codes_pg;
mod referral_rewards_pg;
mod referral_stream_pg;
mod register_stats_rls;
mod reward_participant_pg;
mod routing_is_attribution_only;
mod run_candidate_eval;
mod staging_pins_tool;
mod starter_plausibility_pg;
mod tail_floor_subcommand;
mod token_bundle_pg;
mod trace_artifact_gcs;
mod trace_artifact_gcs_integration;
mod trace_artifact_kek;
mod trace_artifact_store_envelope;
mod trace_corpus_pg_rls;
mod trace_corpus_pg_store;
mod trace_corpus_storage_contract;
mod trace_gate_service;
mod trace_invite_registry_pg;
mod trace_session_identity;
mod trace_session_withdrawal_pg;
mod upload_principal_ownership_pg;
mod upload_trust_frontier_plan_pg;
mod vector_replay_store_methods;
mod versioned_pipeline_remote_restore;
mod versioned_pipeline_runtime_pg;
mod witness_admission_chain;
mod witness_certificate_cross_implementation;
mod witness_evidence_pg;
mod witness_manifest_matches_compose;

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
