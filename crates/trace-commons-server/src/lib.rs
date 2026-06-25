//! TraceCommons hosted server crate.

pub mod account_near;
pub mod account_passkey;
pub mod account_session;
pub mod audit_chain;
pub mod config;
pub mod db;
pub mod error;
pub mod instance_enroll_guard;
pub mod near_credit;
pub mod secrets;
pub mod trace_artifact_gcs;
pub mod trace_artifact_kek;
pub mod trace_artifact_store;
pub mod trace_corpus_storage;
pub mod trace_gate_service;
pub mod trace_upload_claim_allowlist;
pub mod trace_upload_claim_issuer;
pub mod trace_upload_claim_issuer_admin;

pub const TRACE_COMMONS_SERVER_EXTRACTION_STAGE: &str = "server-storage-owned";
