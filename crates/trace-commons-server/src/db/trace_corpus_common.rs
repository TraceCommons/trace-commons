// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

use serde::{Serialize, de::DeserializeOwned};
use uuid::Uuid;

use crate::error::DatabaseError;
use crate::trace_corpus_storage::{
    TRACE_AUDIT_CHAIN_REPAIR_KIND, TenantScopedTraceObjectRef, TraceAuditAction,
    TraceAuditEventWrite, TraceAuditSafeMetadata, TraceCorpusStatus,
};

pub(crate) const TRACE_AUDIT_EVENT_GENESIS_HASH: &str = "sha256:genesis";

pub(crate) fn enum_to_storage<T: Serialize>(value: T) -> Result<String, DatabaseError> {
    let value = serde_json::to_value(value)
        .map_err(|e| DatabaseError::Serialization(format!("trace enum encode failed: {e}")))?;
    value.as_str().map(str::to_string).ok_or_else(|| {
        DatabaseError::Serialization("trace enum did not serialize to a string".to_string())
    })
}

pub(crate) fn enum_from_storage<T: DeserializeOwned>(
    value: &str,
    type_name: &str,
) -> Result<T, DatabaseError> {
    serde_json::from_value(serde_json::Value::String(value.to_string())).map_err(|e| {
        DatabaseError::Serialization(format!(
            "invalid trace {type_name} storage value {value:?}: {e}"
        ))
    })
}

pub(crate) fn audit_action_for_status(status: TraceCorpusStatus) -> TraceAuditAction {
    match status {
        TraceCorpusStatus::Accepted
        | TraceCorpusStatus::Quarantined
        | TraceCorpusStatus::AwaitingPiiBackstop
        | TraceCorpusStatus::Rejected => TraceAuditAction::Review,
        TraceCorpusStatus::Revoked => TraceAuditAction::Revoke,
        TraceCorpusStatus::Purged => TraceAuditAction::Purge,
        TraceCorpusStatus::Expired => TraceAuditAction::Retain,
        TraceCorpusStatus::Received => TraceAuditAction::Submit,
    }
}

pub(crate) fn validate_trace_audit_append_chain(
    tenant_id: &str,
    audit_event_id: Uuid,
    latest_event_hash: Option<&str>,
    previous_event_hash: Option<&str>,
    event_hash_present: bool,
) -> Result<(), DatabaseError> {
    if !event_hash_present {
        return Ok(());
    }

    if let Some(expected_previous) = latest_event_hash {
        let provided_previous = previous_event_hash.unwrap_or(TRACE_AUDIT_EVENT_GENESIS_HASH);
        if provided_previous == expected_previous {
            return Ok(());
        }

        return Err(DatabaseError::Constraint(format!(
            "trace audit append for tenant {tenant_id} event {audit_event_id} has stale previous_event_hash: expected {expected_previous}, got {provided_previous}"
        )));
    }

    Ok(())
}

/// The append-time check for a row that resumes the tenant's hashed audit
/// chain across a legacy segment: file events a build from before the DB
/// carried chain fields wrote with unhashed rows, or none. Such a row chains
/// from the file log's head, so its `previous_event_hash` is not the DB's
/// latest hashed row; instead it names that row, as its
/// `decision_inputs_hash`, and is accepted only while that row is still the
/// latest. A caller that planned against a DB head that has since moved is
/// refused, as a stale ordinary append is.
///
/// Only the operator audit-chain repair's own row may resume the chain: a
/// `Retain` row with the repair's `Maintenance` surface and its canonical
/// payload, which the chain verifiers recompute the hash of. The store
/// checks that itself rather than trusting its one caller to.
pub(crate) fn validate_trace_audit_chain_resume(
    audit_event: &TraceAuditEventWrite,
    latest_event_hash: Option<&str>,
    resumes_from_event_hash: &str,
) -> Result<(), DatabaseError> {
    let tenant_id = &audit_event.tenant_id;
    let audit_event_id = audit_event.audit_event_id;
    let is_repair_row = audit_event.action == TraceAuditAction::Retain
        && audit_event.canonical_event_json.is_some()
        && matches!(
            &audit_event.metadata,
            TraceAuditSafeMetadata::Maintenance {
                surface: Some(surface),
                dry_run: false,
                ..
            } if surface == TRACE_AUDIT_CHAIN_REPAIR_KIND
        );
    if !is_repair_row {
        return Err(DatabaseError::Constraint(format!(
            "trace audit chain resume for tenant {tenant_id} event {audit_event_id} is not an audit chain repair row"
        )));
    }
    let declared_resume_hash = audit_event.decision_inputs_hash.as_deref();
    if audit_event.event_hash.is_none() || audit_event.previous_event_hash.is_none() {
        return Err(DatabaseError::Constraint(format!(
            "trace audit chain resume for tenant {tenant_id} event {audit_event_id} is not chained"
        )));
    }
    if declared_resume_hash != Some(resumes_from_event_hash) {
        return Err(DatabaseError::Constraint(format!(
            "trace audit chain resume for tenant {tenant_id} event {audit_event_id} does not declare the head it resumes from"
        )));
    }
    if latest_event_hash != Some(resumes_from_event_hash) {
        return Err(DatabaseError::Constraint(format!(
            "trace audit chain resume for tenant {tenant_id} event {audit_event_id} has a stale DB head"
        )));
    }
    Ok(())
}

pub(crate) fn validate_tenant_scoped_trace_object_ref(
    field: &str,
    object_ref: &TenantScopedTraceObjectRef,
    tenant_id: &str,
    submission_id: Uuid,
) -> Result<(), DatabaseError> {
    if object_ref.tenant_id == tenant_id && object_ref.submission_id == submission_id {
        return Ok(());
    }

    Err(DatabaseError::Constraint(format!(
        "trace {field} object_ref_id {} does not belong to tenant {tenant_id} submission {submission_id}",
        object_ref.object_ref_id
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEAD: &str = "sha256:head";

    /// The row the operator audit-chain repair writes to resume the chain.
    fn resume_row() -> TraceAuditEventWrite {
        TraceAuditEventWrite {
            audit_event_id: Uuid::new_v4(),
            tenant_id: "t".to_string(),
            actor_principal_ref: "principal".to_string(),
            actor_role: "admin".to_string(),
            action: TraceAuditAction::Retain,
            reason: None,
            request_id: None,
            submission_id: None,
            object_ref_id: None,
            export_manifest_id: None,
            decision_inputs_hash: Some(HEAD.to_string()),
            previous_event_hash: Some("sha256:file-head".to_string()),
            event_hash: Some("sha256:resume".to_string()),
            canonical_event_json: Some("{}".to_string()),
            metadata: TraceAuditSafeMetadata::Maintenance {
                surface: Some(TRACE_AUDIT_CHAIN_REPAIR_KIND.to_string()),
                purpose_hash: None,
                dry_run: false,
                action_counts: Default::default(),
            },
        }
    }

    #[test]
    fn chain_resume_is_accepted_only_from_the_current_db_head_it_declares() {
        assert!(validate_trace_audit_chain_resume(&resume_row(), Some(HEAD), HEAD).is_ok());
        // The DB head moved since the plan, or there is none.
        assert!(
            validate_trace_audit_chain_resume(&resume_row(), Some("sha256:moved"), HEAD).is_err()
        );
        assert!(validate_trace_audit_chain_resume(&resume_row(), None, HEAD).is_err());
        // The row does not declare the head it resumes from.
        let mut undeclared = resume_row();
        undeclared.decision_inputs_hash = None;
        assert!(validate_trace_audit_chain_resume(&undeclared, Some(HEAD), HEAD).is_err());
        // An unchained row cannot resume anything.
        let mut unchained = resume_row();
        unchained.previous_event_hash = None;
        assert!(validate_trace_audit_chain_resume(&unchained, Some(HEAD), HEAD).is_err());
        let mut unhashed = resume_row();
        unhashed.event_hash = None;
        assert!(validate_trace_audit_chain_resume(&unhashed, Some(HEAD), HEAD).is_err());
    }

    /// #1100 review nit: the store itself refuses a resume by any row but the
    /// repair's own, whatever its one caller does.
    #[test]
    fn chain_resume_is_accepted_only_for_the_repair_row() {
        let mut wrong_action = resume_row();
        wrong_action.action = TraceAuditAction::Read;
        let mut wrong_surface = resume_row();
        wrong_surface.metadata = TraceAuditSafeMetadata::Maintenance {
            surface: Some("maintenance".to_string()),
            purpose_hash: None,
            dry_run: false,
            action_counts: Default::default(),
        };
        let mut no_surface = resume_row();
        no_surface.metadata = TraceAuditSafeMetadata::Empty;
        let mut dry_run = resume_row();
        dry_run.metadata = TraceAuditSafeMetadata::Maintenance {
            surface: Some(TRACE_AUDIT_CHAIN_REPAIR_KIND.to_string()),
            purpose_hash: None,
            dry_run: true,
            action_counts: Default::default(),
        };
        let mut no_payload = resume_row();
        no_payload.canonical_event_json = None;
        for (name, row) in [
            ("action", wrong_action),
            ("surface", wrong_surface),
            ("metadata", no_surface),
            ("dry_run", dry_run),
            ("payload", no_payload),
        ] {
            assert!(
                validate_trace_audit_chain_resume(&row, Some(HEAD), HEAD).is_err(),
                "a row with the wrong {name} must not resume the chain"
            );
        }
    }
}
