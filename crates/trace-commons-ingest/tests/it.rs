// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The ingest service's tests that build outside the library.
//!
//! Each `tests/<name>.rs` listed here is a module of this one test binary,
//! as in the server crate's `tests/it.rs`. They reach the service through
//! the library's `test-support` fixtures (`trace_commons_ingest::test_support`),
//! which this crate's self dev-dependency enables for its own tests. A
//! change to one of these files rebuilds this binary and not the service.
//!
//! Run one file's tests with `--test it -- '<name>::'`. The PostgreSQL-backed
//! ones share the `tenant-a` fixture tenant with the library's unit tests:
//! run them with `--test-threads=1`, and never at the same time as the
//! `--lib` tests against the same database.

mod activity_missions_tests;
mod bootstrap_atomicity_tests;
mod export_expiry_tests;
mod large_envelope_tests;
mod legacy_data_authorization_tests;
mod legacy_invite_link_tests;
mod mission_catalog_tests;
mod model_training_consent_tests;
mod multitenant_settlement_tests;
mod native_session_sliding_expiry_tests;
mod near_ai_provisioning_tests;
mod payout_link_tests;
mod payout_rails_tests;
mod public_run_lifecycle_tests;
mod public_run_tests;
mod referral_admin_tests;
mod referral_tests;
mod removed_account_routes_tests;
mod reward_participant_tests;
mod scope_escalation_tests;
mod terms_acceptance_tests;
