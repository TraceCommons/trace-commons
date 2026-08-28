// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Audit hash-chain verification helpers shared between the ingest binary and
//! tests.
//!
//! The canonical truth for the audit chain is the per-tenant append-only
//! `events.jsonl` file. When the optional PostgreSQL mirror is enabled, every
//! file append is followed by a DB insert that carries the same
//! `event_hash` / `previous_event_hash` pair. This module exposes the pure
//! check that the mirrored row matches the canonical hashes so we can
//! fail-closed on column drift at write time rather than waiting for the
//! periodic admin drill to notice.

use uuid::Uuid;

use crate::trace_corpus_storage::TraceAuditEventRecord;

/// Stable error class for hash-chain drift between the canonical file event
/// and the mirrored DB row. Lives in the existing `*Rejected` family
/// (KekContextMismatch, KekDowngradeRejected, TenantDriftRejected).
pub const AUDIT_CHAIN_DRIFT_REJECTED_CLASS: &str = "AuditChainDriftRejected";

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AuditChainDriftError {
    #[error("AuditChainDriftRejected: mirrored audit row missing for tenant")]
    RowMissing,
    #[error("AuditChainDriftRejected: mirrored event_hash differs from canonical chain head")]
    EventHashMismatch,
    #[error("AuditChainDriftRejected: mirrored previous_event_hash differs from canonical chain")]
    PreviousEventHashMismatch,
}

/// Assert that the just-mirrored DB row carries the same `event_hash` and
/// `previous_event_hash` we computed for the canonical file-format event.
///
/// The caller is expected to look up the row by `(tenant_id, audit_event_id)`
/// after `mirror_audit_event_to_db` returns Ok and pass the result here. The
/// file append remains canonical on any error path; this check guards against
/// silent column drift (canonicalization bugs, partial column writes,
/// out-of-band edits to the mirrored row).
pub fn audit_event_matches_writeback(
    expected_event_id: Uuid,
    expected_event_hash: Option<&str>,
    expected_previous_event_hash: Option<&str>,
    actual: Option<&TraceAuditEventRecord>,
) -> Result<(), AuditChainDriftError> {
    let Some(actual) = actual else {
        return Err(AuditChainDriftError::RowMissing);
    };
    debug_assert_eq!(actual.audit_event_id, expected_event_id);
    if actual.event_hash.as_deref() != expected_event_hash {
        return Err(AuditChainDriftError::EventHashMismatch);
    }
    if actual.previous_event_hash.as_deref() != expected_previous_event_hash {
        return Err(AuditChainDriftError::PreviousEventHashMismatch);
    }
    Ok(())
}
