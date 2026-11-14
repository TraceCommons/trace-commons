// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Integration tests of `trace-commons-server`, built as one test binary.
//!
//! Each `tests/<name>.rs` below is a module, so its tests are named
//! `<name>::<test>`; run one file's tests with
//! `cargo test -p trace-commons-server --test it -- '<name>::'`.
//! Before, each file was its own binary, and every one of them linked the
//! whole dependency graph again. A file that cannot share a process with
//! the others is its own `[[test]]` in Cargo.toml, with the reason beside
//! it. The package sets `autotests = false`, so a file in neither place
//! would never be built; `it_harness` refuses that.

#[macro_use]
#[path = "../../trace-commons-protocol/tests/support/it_harness.rs"]
mod it_harness;

// The src/bin modules that several suites test. They refer to each other as
// `super::<module>`, so they keep the layout of their src/bin directory, and
// each is compiled once here rather than once per suite that uses it. Each
// suite uses only part of a binary's modules, so the rest reads as dead code.
#[allow(dead_code)]
#[path = "../src/bin/gate_calibrate"]
mod gate_calibrate {
    pub mod bakeoff_corpus;
    pub mod bakeoff_manifest;
    pub mod bakeoff_metrics;
    pub mod bakeoff_report;
    pub mod run_candidate_eval;
    pub mod trivial_measures;
}
#[allow(dead_code)]
#[path = "../src/bin/pilot_bootstrap"]
mod pilot_bootstrap {
    pub mod hf_dataset;
    pub mod sidecar;
    pub mod submitter;
    pub mod translators;
}

mod account_onboarding_contract;
mod audit_hash_chain_writeback;
mod bakeoff_corpus;
mod bakeoff_corpus_script;
mod bakeoff_manifest;
mod bakeoff_metrics;
mod bakeoff_report;
mod bakeoff_subcommand;
mod cloud_kms_kek_wrapper;
mod corpus_trivial_measures;
mod inference_funding;
mod license_boundary;
mod near_ai_live_receipt;
mod near_credit_contract;
mod pilot_bootstrap_submitter;
mod pilot_bootstrap_translators;
mod routing_is_attribution_only;
mod run_candidate_eval;
mod tail_floor_subcommand;
mod trace_artifact_gcs;
mod trace_artifact_gcs_integration;
mod trace_artifact_kek;
mod trace_artifact_store_envelope;
mod trace_corpus_storage_contract;
mod trace_gate_service;
mod trace_session_identity;
mod unsafe_boundary;
mod versioned_pipeline_remote_restore;
mod witness_admission_chain;
mod witness_certificate_cross_implementation;
mod witness_manifest_matches_compose;
