// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

use serde::{Serialize, de::DeserializeOwned};
use uuid::Uuid;

use crate::error::DatabaseError;
use crate::trace_corpus_storage::{
    TenantScopedTraceObjectRef, TraceAuditAction, TraceCorpusStatus,
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
pub(crate) fn validate_trace_audit_chain_resume(
    tenant_id: &str,
    audit_event_id: Uuid,
    latest_event_hash: Option<&str>,
    resumes_from_event_hash: &str,
    declared_resume_hash: Option<&str>,
    previous_event_hash: Option<&str>,
    event_hash_present: bool,
) -> Result<(), DatabaseError> {
    if !event_hash_present || previous_event_hash.is_none() {
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

    #[test]
    fn chain_resume_is_accepted_only_from_the_current_db_head_it_declares() {
        let id = Uuid::new_v4();
        let head = "sha256:head";
        let file_head = Some("sha256:file-head");
        assert!(
            validate_trace_audit_chain_resume(
                "t",
                id,
                Some(head),
                head,
                Some(head),
                file_head,
                true
            )
            .is_ok()
        );
        // The DB head moved since the plan, or there is none.
        assert!(
            validate_trace_audit_chain_resume(
                "t",
                id,
                Some("sha256:moved"),
                head,
                Some(head),
                file_head,
                true
            )
            .is_err()
        );
        assert!(
            validate_trace_audit_chain_resume("t", id, None, head, Some(head), file_head, true)
                .is_err()
        );
        // The row does not declare the head it resumes from.
        assert!(
            validate_trace_audit_chain_resume("t", id, Some(head), head, None, file_head, true)
                .is_err()
        );
        // An unchained row cannot resume anything.
        assert!(
            validate_trace_audit_chain_resume("t", id, Some(head), head, Some(head), None, true)
                .is_err()
        );
        assert!(
            validate_trace_audit_chain_resume(
                "t",
                id,
                Some(head),
                head,
                Some(head),
                file_head,
                false
            )
            .is_err()
        );
    }
}
