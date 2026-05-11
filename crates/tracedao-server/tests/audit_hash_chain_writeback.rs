//! Unit-level coverage for the on-write DB audit chain verification (Gap 2).
//!
//! The verification helper is pure: given the expected `event_hash` /
//! `previous_event_hash` we computed for the canonical file-format event and
//! the row we just read back from the mirror, it asserts both hashes match
//! and otherwise emits a stable `AuditChainDriftRejected` error. No
//! PostgreSQL connection is needed; we construct the `TraceAuditEventRecord`
//! directly and exercise each branch.

use chrono::Utc;
use tracedao_server::audit_chain::{
    AUDIT_CHAIN_DRIFT_REJECTED_CLASS, AuditChainDriftError, audit_event_matches_writeback,
};
use tracedao_server::trace_corpus_storage::{
    TraceAuditAction, TraceAuditEventRecord, TraceAuditSafeMetadata,
};
use uuid::Uuid;

fn sample_record(
    audit_event_id: Uuid,
    event_hash: Option<&str>,
    previous_event_hash: Option<&str>,
) -> TraceAuditEventRecord {
    TraceAuditEventRecord {
        audit_event_id,
        tenant_id: "tenant-a".to_string(),
        audit_sequence: 1,
        actor_principal_ref: "principal-hash".to_string(),
        actor_role: "reviewer".to_string(),
        action: TraceAuditAction::Read,
        reason: None,
        request_id: None,
        submission_id: Some(Uuid::nil()),
        object_ref_id: None,
        export_manifest_id: None,
        decision_inputs_hash: None,
        previous_event_hash: previous_event_hash.map(str::to_string),
        event_hash: event_hash.map(str::to_string),
        canonical_event_json: None,
        metadata: TraceAuditSafeMetadata::Empty,
        occurred_at: Utc::now(),
    }
}

#[test]
fn writeback_match_returns_ok() {
    let event_id = Uuid::new_v4();
    let expected_event_hash = "sha256:0011aabbcc";
    let expected_prev = "sha256:genesis";
    let record = sample_record(event_id, Some(expected_event_hash), Some(expected_prev));
    audit_event_matches_writeback(
        event_id,
        Some(expected_event_hash),
        Some(expected_prev),
        Some(&record),
    )
    .expect("matching hashes verify cleanly");
}

#[test]
fn writeback_rejects_missing_row() {
    let err =
        audit_event_matches_writeback(Uuid::new_v4(), Some("sha256:abc"), Some("sha256:genesis"), None)
            .expect_err("missing row must reject");
    assert_eq!(err, AuditChainDriftError::RowMissing);
    let display = err.to_string();
    assert!(
        display.starts_with(AUDIT_CHAIN_DRIFT_REJECTED_CLASS),
        "error class prefix expected: {display}"
    );
}

#[test]
fn writeback_rejects_tampered_event_hash() {
    let event_id = Uuid::new_v4();
    let record = sample_record(event_id, Some("sha256:TAMPERED"), Some("sha256:genesis"));
    let err = audit_event_matches_writeback(
        event_id,
        Some("sha256:0011aabbcc"),
        Some("sha256:genesis"),
        Some(&record),
    )
    .expect_err("tampered event_hash must reject");
    assert_eq!(err, AuditChainDriftError::EventHashMismatch);
    assert!(err.to_string().starts_with(AUDIT_CHAIN_DRIFT_REJECTED_CLASS));
}

#[test]
fn writeback_rejects_tampered_previous_event_hash() {
    let event_id = Uuid::new_v4();
    let record = sample_record(event_id, Some("sha256:0011aabbcc"), Some("sha256:TAMPERED"));
    let err = audit_event_matches_writeback(
        event_id,
        Some("sha256:0011aabbcc"),
        Some("sha256:genesis"),
        Some(&record),
    )
    .expect_err("tampered previous_event_hash must reject");
    assert_eq!(err, AuditChainDriftError::PreviousEventHashMismatch);
    assert!(err.to_string().starts_with(AUDIT_CHAIN_DRIFT_REJECTED_CLASS));
}

#[test]
fn writeback_rejects_missing_event_hash_column() {
    let event_id = Uuid::new_v4();
    let record = sample_record(event_id, None, Some("sha256:genesis"));
    let err = audit_event_matches_writeback(
        event_id,
        Some("sha256:0011aabbcc"),
        Some("sha256:genesis"),
        Some(&record),
    )
    .expect_err("null event_hash column must reject when canonical event has one");
    assert_eq!(err, AuditChainDriftError::EventHashMismatch);
}
