CREATE TABLE IF NOT EXISTS trace_tenants (
    tenant_id TEXT PRIMARY KEY,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE IF NOT EXISTS trace_tenant_policies (
    tenant_id TEXT PRIMARY KEY REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    policy_version TEXT NOT NULL,
    allowed_consent_scopes TEXT NOT NULL DEFAULT '[]',
    allowed_uses TEXT NOT NULL DEFAULT '[]',
    updated_by_principal_ref TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE IF NOT EXISTS trace_tenant_access_grants (
    tenant_id TEXT NOT NULL,
    grant_id TEXT NOT NULL,
    principal_ref TEXT NOT NULL,
    role TEXT NOT NULL,
    status TEXT NOT NULL,
    allowed_consent_scopes TEXT NOT NULL DEFAULT '[]',
    allowed_uses TEXT NOT NULL DEFAULT '[]',
    issuer TEXT,
    audience TEXT,
    subject TEXT,
    issued_at TEXT NOT NULL,
    expires_at TEXT,
    revoked_at TEXT,
    created_by_principal_ref TEXT,
    revoked_by_principal_ref TEXT,
    reason TEXT,
    metadata_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (tenant_id, grant_id),
    FOREIGN KEY (tenant_id)
        REFERENCES trace_tenants (tenant_id)
        ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_trace_tenant_access_grants_principal
    ON trace_tenant_access_grants (tenant_id, principal_ref, status, expires_at);
CREATE INDEX IF NOT EXISTS idx_trace_tenant_access_grants_role
    ON trace_tenant_access_grants (tenant_id, role, status, expires_at);
CREATE INDEX IF NOT EXISTS idx_trace_tenant_access_grants_issuer_subject
    ON trace_tenant_access_grants (tenant_id, issuer, subject);

CREATE TABLE IF NOT EXISTS trace_submissions (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    submission_id TEXT NOT NULL,
    trace_id TEXT NOT NULL,
    auth_principal_ref TEXT NOT NULL,
    contributor_pseudonym TEXT,
    submitted_tenant_scope_ref TEXT,
    schema_version TEXT NOT NULL,
    consent_policy_version TEXT NOT NULL,
    consent_scopes TEXT NOT NULL DEFAULT '[]',
    allowed_uses TEXT NOT NULL DEFAULT '[]',
    retention_policy_id TEXT NOT NULL,
    status TEXT NOT NULL,
    privacy_risk TEXT NOT NULL,
    redaction_pipeline_version TEXT NOT NULL,
    redaction_hash TEXT NOT NULL,
    redaction_counts TEXT NOT NULL DEFAULT '{}',
    canonical_summary_hash TEXT,
    submission_score REAL,
    credit_points_pending REAL,
    credit_points_final REAL,
    received_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    reviewed_at TEXT,
    review_assigned_to_principal_ref TEXT,
    review_assigned_at TEXT,
    review_lease_expires_at TEXT,
    review_due_at TEXT,
    revoked_at TEXT,
    expires_at TEXT,
    purged_at TEXT,
    PRIMARY KEY (tenant_id, submission_id)
);

CREATE INDEX IF NOT EXISTS idx_trace_submissions_tenant_status
    ON trace_submissions (tenant_id, status, received_at DESC);
CREATE INDEX IF NOT EXISTS idx_trace_submissions_trace_id
    ON trace_submissions (tenant_id, trace_id);
CREATE INDEX IF NOT EXISTS idx_trace_submissions_contributor
    ON trace_submissions (tenant_id, contributor_pseudonym)
    WHERE contributor_pseudonym IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_trace_submissions_review_lease
    ON trace_submissions (tenant_id, status, review_lease_expires_at, received_at DESC);

CREATE TABLE IF NOT EXISTS trace_object_refs (
    tenant_id TEXT NOT NULL,
    submission_id TEXT NOT NULL,
    object_ref_id TEXT NOT NULL,
    artifact_kind TEXT NOT NULL,
    object_store TEXT NOT NULL,
    object_key TEXT NOT NULL,
    content_sha256 TEXT NOT NULL,
    encryption_key_ref TEXT NOT NULL,
    size_bytes INTEGER NOT NULL,
    compression TEXT,
    created_by_job_id TEXT,
    invalidated_at TEXT,
    deleted_at TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (tenant_id, submission_id, object_ref_id),
    FOREIGN KEY (tenant_id, submission_id)
        REFERENCES trace_submissions (tenant_id, submission_id)
        ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_trace_object_refs_kind
    ON trace_object_refs (tenant_id, artifact_kind, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_trace_object_refs_lifecycle
    ON trace_object_refs (tenant_id, submission_id, invalidated_at, deleted_at);

CREATE TABLE IF NOT EXISTS trace_derived_records (
    tenant_id TEXT NOT NULL,
    derived_id TEXT NOT NULL,
    submission_id TEXT NOT NULL,
    trace_id TEXT NOT NULL,
    status TEXT NOT NULL,
    worker_kind TEXT NOT NULL,
    worker_version TEXT NOT NULL,
    input_object_ref_id TEXT,
    input_hash TEXT NOT NULL,
    output_object_ref_id TEXT,
    canonical_summary TEXT,
    canonical_summary_hash TEXT,
    summary_model TEXT NOT NULL DEFAULT 'redacted-summary-hash-precheck-v1',
    task_success TEXT,
    privacy_risk TEXT,
    event_count INTEGER,
    tool_sequence TEXT NOT NULL DEFAULT '[]',
    tool_categories TEXT NOT NULL DEFAULT '[]',
    coverage_tags TEXT NOT NULL DEFAULT '[]',
    duplicate_score REAL,
    novelty_score REAL,
    cluster_id TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (tenant_id, derived_id),
    FOREIGN KEY (tenant_id, submission_id)
        REFERENCES trace_submissions (tenant_id, submission_id)
        ON DELETE CASCADE,
    FOREIGN KEY (tenant_id, submission_id, input_object_ref_id)
        REFERENCES trace_object_refs (tenant_id, submission_id, object_ref_id),
    FOREIGN KEY (tenant_id, submission_id, output_object_ref_id)
        REFERENCES trace_object_refs (tenant_id, submission_id, object_ref_id)
);

CREATE INDEX IF NOT EXISTS idx_trace_derived_records_submission
    ON trace_derived_records (tenant_id, submission_id, worker_kind);
CREATE INDEX IF NOT EXISTS idx_trace_derived_records_cluster
    ON trace_derived_records (tenant_id, cluster_id)
    WHERE cluster_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS trace_vector_entries (
    tenant_id TEXT NOT NULL,
    submission_id TEXT NOT NULL,
    derived_id TEXT NOT NULL,
    vector_entry_id TEXT NOT NULL,
    vector_store TEXT NOT NULL,
    embedding_model TEXT NOT NULL,
    embedding_dimension INTEGER NOT NULL CHECK (embedding_dimension > 0),
    embedding_version TEXT NOT NULL,
    source_projection TEXT NOT NULL CHECK (source_projection IN ('canonical_summary', 'redacted_messages', 'redacted_tool_sequence')),
    source_hash TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('active', 'invalidated', 'deleted')),
    nearest_trace_ids TEXT NOT NULL DEFAULT '[]',
    cluster_id TEXT,
    duplicate_score REAL,
    novelty_score REAL,
    indexed_at TEXT,
    invalidated_at TEXT,
    deleted_at TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (tenant_id, submission_id, vector_entry_id),
    FOREIGN KEY (tenant_id, submission_id)
        REFERENCES trace_submissions (tenant_id, submission_id)
        ON DELETE CASCADE,
    FOREIGN KEY (tenant_id, derived_id)
        REFERENCES trace_derived_records (tenant_id, derived_id)
        ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_trace_vector_entries_source
    ON trace_vector_entries (tenant_id, submission_id, status);
CREATE INDEX IF NOT EXISTS idx_trace_vector_entries_cluster
    ON trace_vector_entries (tenant_id, cluster_id, status)
    WHERE cluster_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS trace_audit_events (
    tenant_id TEXT NOT NULL,
    audit_sequence INTEGER NOT NULL,
    audit_event_id TEXT NOT NULL,
    actor_principal_ref TEXT NOT NULL,
    actor_role TEXT NOT NULL,
    action TEXT NOT NULL,
    reason TEXT,
    request_id TEXT,
    submission_id TEXT,
    object_ref_id TEXT,
    export_manifest_id TEXT,
    decision_inputs_hash TEXT,
    previous_event_hash TEXT,
    event_hash TEXT,
    canonical_event_json TEXT,
    metadata_json TEXT NOT NULL DEFAULT '{}',
    occurred_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (tenant_id, audit_event_id)
);

CREATE INDEX IF NOT EXISTS idx_trace_audit_events_submission
    ON trace_audit_events (tenant_id, submission_id, occurred_at DESC);
CREATE INDEX IF NOT EXISTS idx_trace_audit_events_action
    ON trace_audit_events (tenant_id, action, occurred_at DESC);
CREATE UNIQUE INDEX IF NOT EXISTS idx_trace_audit_events_tenant_sequence
    ON trace_audit_events (tenant_id, audit_sequence);

CREATE TABLE IF NOT EXISTS trace_export_manifests (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    export_manifest_id TEXT NOT NULL,
    artifact_kind TEXT NOT NULL,
    purpose_code TEXT,
    audit_event_id TEXT,
    source_submission_ids TEXT NOT NULL DEFAULT '[]',
    source_submission_ids_hash TEXT NOT NULL,
    item_count INTEGER NOT NULL CHECK (item_count >= 0),
    generated_at TEXT NOT NULL,
    invalidated_at TEXT,
    deleted_at TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (tenant_id, export_manifest_id)
);

CREATE INDEX IF NOT EXISTS idx_trace_export_manifests_generated
    ON trace_export_manifests (tenant_id, generated_at DESC);
CREATE INDEX IF NOT EXISTS idx_trace_export_manifests_hash
    ON trace_export_manifests (tenant_id, source_submission_ids_hash);

CREATE TABLE IF NOT EXISTS trace_export_manifest_items (
    tenant_id TEXT NOT NULL,
    export_manifest_id TEXT NOT NULL,
    submission_id TEXT NOT NULL,
    trace_id TEXT NOT NULL,
    derived_id TEXT,
    object_ref_id TEXT,
    vector_entry_id TEXT,
    source_status_at_export TEXT NOT NULL,
    source_hash_at_export TEXT NOT NULL,
    source_invalidated_at TEXT,
    source_invalidation_reason TEXT CHECK (
        source_invalidation_reason IS NULL
        OR source_invalidation_reason IN ('revoked', 'expired', 'purged')
    ),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (tenant_id, export_manifest_id, submission_id),
    FOREIGN KEY (tenant_id, export_manifest_id)
        REFERENCES trace_export_manifests (tenant_id, export_manifest_id)
        ON DELETE CASCADE,
    FOREIGN KEY (tenant_id, submission_id)
        REFERENCES trace_submissions (tenant_id, submission_id)
        ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_trace_export_manifest_items_source
    ON trace_export_manifest_items (tenant_id, submission_id, source_invalidated_at);
CREATE INDEX IF NOT EXISTS idx_trace_export_manifest_items_manifest
    ON trace_export_manifest_items (tenant_id, export_manifest_id, created_at ASC);

CREATE TABLE IF NOT EXISTS trace_credit_ledger (
    tenant_id TEXT NOT NULL,
    credit_event_id TEXT NOT NULL,
    submission_id TEXT NOT NULL,
    trace_id TEXT NOT NULL,
    credit_account_ref TEXT NOT NULL,
    event_type TEXT NOT NULL,
    points_delta TEXT NOT NULL,
    reason TEXT NOT NULL,
    external_ref TEXT,
    actor_principal_ref TEXT NOT NULL,
    actor_role TEXT NOT NULL,
    settlement_state TEXT NOT NULL,
    occurred_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (tenant_id, credit_event_id),
    FOREIGN KEY (tenant_id, submission_id)
        REFERENCES trace_submissions (tenant_id, submission_id)
        ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_trace_credit_ledger_account
    ON trace_credit_ledger (tenant_id, credit_account_ref, occurred_at DESC);
CREATE INDEX IF NOT EXISTS idx_trace_credit_ledger_submission
    ON trace_credit_ledger (tenant_id, submission_id, occurred_at DESC);

CREATE TABLE IF NOT EXISTS trace_tombstones (
    tenant_id TEXT NOT NULL,
    tombstone_id TEXT NOT NULL,
    submission_id TEXT NOT NULL,
    trace_id TEXT,
    redaction_hash TEXT,
    canonical_summary_hash TEXT,
    reason TEXT NOT NULL,
    effective_at TEXT NOT NULL,
    retain_until TEXT,
    created_by_principal_ref TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (tenant_id, tombstone_id),
    UNIQUE (tenant_id, submission_id),
    FOREIGN KEY (tenant_id, submission_id)
        REFERENCES trace_submissions (tenant_id, submission_id)
        ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_trace_tombstones_effective
    ON trace_tombstones (tenant_id, effective_at DESC);

CREATE TABLE IF NOT EXISTS trace_retention_jobs (
    tenant_id TEXT NOT NULL,
    retention_job_id TEXT NOT NULL,
    purpose TEXT NOT NULL,
    dry_run INTEGER NOT NULL,
    status TEXT NOT NULL,
    requested_by_principal_ref TEXT NOT NULL,
    requested_by_role TEXT NOT NULL,
    purge_expired_before TEXT,
    prune_export_cache INTEGER NOT NULL DEFAULT 1,
    max_export_age_hours INTEGER,
    audit_event_id TEXT,
    action_counts TEXT NOT NULL DEFAULT '{}',
    selected_revoked_count INTEGER NOT NULL DEFAULT 0,
    selected_expired_count INTEGER NOT NULL DEFAULT 0,
    started_at TEXT,
    completed_at TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (tenant_id, retention_job_id),
    FOREIGN KEY (tenant_id)
        REFERENCES trace_tenants (tenant_id)
        ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_trace_retention_jobs_created
    ON trace_retention_jobs (tenant_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_trace_retention_jobs_status
    ON trace_retention_jobs (tenant_id, status, updated_at DESC);

CREATE TABLE IF NOT EXISTS trace_retention_job_items (
    tenant_id TEXT NOT NULL,
    retention_job_id TEXT NOT NULL,
    submission_id TEXT NOT NULL,
    action TEXT NOT NULL,
    status TEXT NOT NULL,
    reason TEXT NOT NULL,
    action_counts TEXT NOT NULL DEFAULT '{}',
    verified_at TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (tenant_id, retention_job_id, submission_id, action),
    FOREIGN KEY (tenant_id, retention_job_id)
        REFERENCES trace_retention_jobs (tenant_id, retention_job_id)
        ON DELETE CASCADE,
    FOREIGN KEY (tenant_id, submission_id)
        REFERENCES trace_submissions (tenant_id, submission_id)
        ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_trace_retention_job_items_submission
    ON trace_retention_job_items (tenant_id, submission_id, created_at DESC);

CREATE TABLE IF NOT EXISTS trace_export_access_grants (
    tenant_id TEXT NOT NULL,
    export_job_id TEXT NOT NULL,
    grant_id TEXT NOT NULL,
    caller_principal_ref TEXT NOT NULL,
    requested_dataset_kind TEXT NOT NULL,
    purpose TEXT NOT NULL,
    max_item_cap INTEGER,
    status TEXT NOT NULL,
    requested_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    metadata_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (tenant_id, grant_id),
    UNIQUE (tenant_id, export_job_id, grant_id),
    FOREIGN KEY (tenant_id)
        REFERENCES trace_tenants (tenant_id)
        ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_trace_export_access_grants_job
    ON trace_export_access_grants (tenant_id, export_job_id);
CREATE INDEX IF NOT EXISTS idx_trace_export_access_grants_principal
    ON trace_export_access_grants (tenant_id, caller_principal_ref, expires_at);
CREATE INDEX IF NOT EXISTS idx_trace_export_access_grants_status
    ON trace_export_access_grants (tenant_id, status, expires_at);

CREATE TABLE IF NOT EXISTS trace_export_jobs (
    tenant_id TEXT NOT NULL,
    export_job_id TEXT NOT NULL,
    grant_id TEXT NOT NULL,
    caller_principal_ref TEXT NOT NULL,
    requested_dataset_kind TEXT NOT NULL,
    purpose TEXT NOT NULL,
    max_item_cap INTEGER,
    status TEXT NOT NULL,
    requested_at TEXT NOT NULL,
    started_at TEXT,
    finished_at TEXT,
    expires_at TEXT NOT NULL,
    result_manifest_id TEXT,
    item_count INTEGER,
    last_error TEXT,
    metadata_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (tenant_id, export_job_id),
    FOREIGN KEY (tenant_id, grant_id)
        REFERENCES trace_export_access_grants (tenant_id, grant_id)
        ON DELETE RESTRICT
);

CREATE INDEX IF NOT EXISTS idx_trace_export_jobs_requested
    ON trace_export_jobs (tenant_id, requested_at DESC);
CREATE INDEX IF NOT EXISTS idx_trace_export_jobs_status
    ON trace_export_jobs (tenant_id, status, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_trace_export_jobs_grant
    ON trace_export_jobs (tenant_id, grant_id);

CREATE TABLE IF NOT EXISTS trace_revocation_propagation_items (
    tenant_id TEXT NOT NULL,
    propagation_item_id TEXT NOT NULL,
    source_submission_id TEXT NOT NULL,
    trace_id TEXT NOT NULL,
    target_kind TEXT NOT NULL CHECK (
        target_kind IN (
            'object_ref',
            'export_manifest',
            'export_manifest_item',
            'vector_entry',
            'derived_record',
            'benchmark_artifact',
            'ranker_artifact',
            'credit_settlement',
            'physical_delete_receipt'
        )
    ),
    target_json TEXT NOT NULL,
    action TEXT NOT NULL CHECK (
        action IN (
            'invalidate_metadata',
            'invalidate_export_membership',
            'invalidate_vector',
            'invalidate_benchmark_artifact',
            'invalidate_ranker_artifact',
            'reverse_credit_settlement',
            'delete_object_payload',
            'record_physical_delete_receipt'
        )
    ),
    status TEXT NOT NULL CHECK (
        status IN ('pending', 'in_progress', 'done', 'failed', 'skipped')
    ),
    idempotency_key TEXT NOT NULL,
    reason TEXT NOT NULL,
    attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
    last_error TEXT,
    next_attempt_at TEXT,
    completed_at TEXT,
    evidence_hash TEXT,
    metadata_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (tenant_id, propagation_item_id),
    UNIQUE (tenant_id, idempotency_key),
    FOREIGN KEY (tenant_id, source_submission_id)
        REFERENCES trace_submissions (tenant_id, submission_id)
        ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_trace_revocation_propagation_source
    ON trace_revocation_propagation_items (tenant_id, source_submission_id, created_at ASC);
CREATE INDEX IF NOT EXISTS idx_trace_revocation_propagation_due
    ON trace_revocation_propagation_items (tenant_id, status, next_attempt_at, created_at ASC);
CREATE INDEX IF NOT EXISTS idx_trace_revocation_propagation_target
    ON trace_revocation_propagation_items (tenant_id, target_kind, updated_at DESC);
