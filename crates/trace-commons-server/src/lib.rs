// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! TraceCommons hosted server crate.

pub mod account_native_auth;
pub mod account_near;
pub mod account_passkey;
pub mod account_session;
pub mod audit_chain;
pub mod celestine_sloth_claim;
pub mod config;
pub mod contributor_cap;
pub mod correction_value;
pub mod credit_quality;
pub mod db;
pub mod dedup_assign;
pub mod dedup_simhash;
pub mod driver_liveness;
pub mod error;
pub mod instance_enroll_guard;
pub mod near_credit;
pub mod near_legion_claim;
pub mod secrets;
pub mod trace_artifact_gcs;
pub mod trace_artifact_kek;
pub mod trace_artifact_store;
pub mod trace_corpus_storage;
pub mod trace_gate_service;
pub mod trace_invite_admin;
pub mod trace_invite_registry;
pub mod trace_score_attestation;
pub mod trace_upload_claim_allowlist;
pub mod trace_upload_claim_issuer;
pub mod trace_upload_claim_issuer_admin;

pub const TRACE_COMMONS_SERVER_EXTRACTION_STAGE: &str = "server-storage-owned";
