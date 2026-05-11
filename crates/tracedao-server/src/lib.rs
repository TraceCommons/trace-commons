//! TraceDAO hosted server crate.

pub mod audit_chain;
pub mod config;
pub mod db;
pub mod error;
pub mod near_credit;
pub mod secrets;
pub mod trace_artifact_gcs;
pub mod trace_artifact_kek;
pub mod trace_artifact_store;
pub mod trace_corpus_storage;
pub mod trace_upload_claim_issuer;

pub const TRACEDAO_SERVER_EXTRACTION_STAGE: &str = "server-storage-owned";
