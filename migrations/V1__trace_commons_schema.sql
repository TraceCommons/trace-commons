-- Trace Commons production-storage control plane.
--
-- The private corpus never stores raw local traces centrally. These tables
-- track tenant-scoped redacted submissions, derived artifacts, object
-- references, authorization policy, review/export/retention workflow state,
-- audit events, credit ledger entries, and revocation tombstones.
--
-- Payload bodies, vectors, and large export artifacts belong in encrypted
-- object/vector storage and are referenced here by hash and object metadata.

CREATE TABLE trace_tenants (
    tenant_id TEXT PRIMARY KEY,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE trace_tenant_policies (
    tenant_id TEXT PRIMARY KEY REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    policy_version TEXT NOT NULL,
    allowed_consent_scopes JSONB NOT NULL DEFAULT '[]'::JSONB,
    allowed_uses JSONB NOT NULL DEFAULT '[]'::JSONB,
    updated_by_principal_ref TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE trace_tenant_access_grants (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    grant_id UUID NOT NULL,
    principal_ref TEXT NOT NULL,
    role TEXT NOT NULL CHECK (
        role IN (
            'contributor',
            'reviewer',
            'admin',
            'export_worker',
            'retention_worker',
            'vector_worker',
            'benchmark_worker',
            'utility_worker',
            'process_eval_worker',
            'revocation_worker'
        )
    ),
    status TEXT NOT NULL CHECK (status IN ('active', 'revoked', 'expired')),
    allowed_consent_scopes JSONB NOT NULL DEFAULT '[]'::JSONB,
    allowed_uses JSONB NOT NULL DEFAULT '[]'::JSONB,
    issuer TEXT,
    audience TEXT,
    subject TEXT,
    issued_at TIMESTAMPTZ NOT NULL,
    expires_at TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ,
    created_by_principal_ref TEXT,
    revoked_by_principal_ref TEXT,
    reason TEXT,
    metadata_json JSONB NOT NULL DEFAULT '{}'::JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, grant_id)
);

CREATE INDEX idx_trace_tenant_access_grants_principal
    ON trace_tenant_access_grants (tenant_id, principal_ref, status, expires_at);
CREATE INDEX idx_trace_tenant_access_grants_role
    ON trace_tenant_access_grants (tenant_id, role, status, expires_at);
CREATE INDEX idx_trace_tenant_access_grants_issuer_subject
    ON trace_tenant_access_grants (tenant_id, issuer, subject)
    WHERE issuer IS NOT NULL OR subject IS NOT NULL;

CREATE TABLE trace_submissions (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    submission_id UUID NOT NULL,
    trace_id UUID NOT NULL,
    auth_principal_ref TEXT NOT NULL,
    contributor_pseudonym TEXT,
    submitted_tenant_scope_ref TEXT,
    schema_version TEXT NOT NULL,
    consent_policy_version TEXT NOT NULL,
    consent_scopes JSONB NOT NULL DEFAULT '[]'::JSONB,
    allowed_uses JSONB NOT NULL DEFAULT '[]'::JSONB,
    retention_policy_id TEXT NOT NULL,
    status TEXT NOT NULL,
    privacy_risk TEXT NOT NULL,
    redaction_pipeline_version TEXT NOT NULL,
    redaction_hash TEXT NOT NULL,
    redaction_counts JSONB NOT NULL DEFAULT '{}'::JSONB,
    canonical_summary_hash TEXT,
    submission_score REAL,
    credit_points_pending REAL,
    credit_points_final REAL,
    received_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    reviewed_at TIMESTAMPTZ,
    review_assigned_to_principal_ref TEXT,
    review_assigned_at TIMESTAMPTZ,
    review_lease_expires_at TIMESTAMPTZ,
    review_due_at TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ,
    expires_at TIMESTAMPTZ,
    purged_at TIMESTAMPTZ,
    PRIMARY KEY (tenant_id, submission_id)
);

CREATE INDEX idx_trace_submissions_tenant_status
    ON trace_submissions (tenant_id, status, received_at DESC);
CREATE INDEX idx_trace_submissions_trace_id
    ON trace_submissions (tenant_id, trace_id);
CREATE INDEX idx_trace_submissions_contributor
    ON trace_submissions (tenant_id, contributor_pseudonym)
    WHERE contributor_pseudonym IS NOT NULL;
CREATE INDEX idx_trace_submissions_review_lease
    ON trace_submissions (tenant_id, status, review_lease_expires_at, received_at DESC);

CREATE TABLE trace_object_refs (
    tenant_id TEXT NOT NULL,
    submission_id UUID NOT NULL,
    object_ref_id UUID NOT NULL,
    artifact_kind TEXT NOT NULL,
    object_store TEXT NOT NULL,
    object_key TEXT NOT NULL,
    content_sha256 TEXT NOT NULL,
    encryption_key_ref TEXT NOT NULL,
    size_bytes BIGINT NOT NULL,
    compression TEXT,
    created_by_job_id UUID,
    invalidated_at TIMESTAMPTZ,
    deleted_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, submission_id, object_ref_id),
    FOREIGN KEY (tenant_id, submission_id)
        REFERENCES trace_submissions (tenant_id, submission_id)
        ON DELETE CASCADE
);

CREATE INDEX idx_trace_object_refs_kind
    ON trace_object_refs (tenant_id, artifact_kind, created_at DESC);
CREATE INDEX idx_trace_object_refs_lifecycle
    ON trace_object_refs (tenant_id, submission_id, invalidated_at, deleted_at);

CREATE TABLE trace_derived_records (
    tenant_id TEXT NOT NULL,
    derived_id UUID NOT NULL,
    submission_id UUID NOT NULL,
    trace_id UUID NOT NULL,
    status TEXT NOT NULL,
    worker_kind TEXT NOT NULL,
    worker_version TEXT NOT NULL,
    input_object_ref_id UUID,
    input_hash TEXT NOT NULL,
    output_object_ref_id UUID,
    canonical_summary TEXT,
    canonical_summary_hash TEXT,
    summary_model TEXT NOT NULL DEFAULT 'redacted-summary-hash-precheck-v1',
    task_success TEXT,
    privacy_risk TEXT,
    event_count INTEGER,
    tool_sequence JSONB NOT NULL DEFAULT '[]'::JSONB,
    tool_categories JSONB NOT NULL DEFAULT '[]'::JSONB,
    coverage_tags JSONB NOT NULL DEFAULT '[]'::JSONB,
    duplicate_score REAL,
    novelty_score REAL,
    cluster_id TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, derived_id),
    FOREIGN KEY (tenant_id, submission_id)
        REFERENCES trace_submissions (tenant_id, submission_id)
        ON DELETE CASCADE,
    FOREIGN KEY (tenant_id, submission_id, input_object_ref_id)
        REFERENCES trace_object_refs (tenant_id, submission_id, object_ref_id),
    FOREIGN KEY (tenant_id, submission_id, output_object_ref_id)
        REFERENCES trace_object_refs (tenant_id, submission_id, object_ref_id)
);

CREATE INDEX idx_trace_derived_records_submission
    ON trace_derived_records (tenant_id, submission_id, worker_kind);
CREATE INDEX idx_trace_derived_records_cluster
    ON trace_derived_records (tenant_id, cluster_id)
    WHERE cluster_id IS NOT NULL;

CREATE TABLE trace_vector_entries (
    tenant_id TEXT NOT NULL,
    submission_id UUID NOT NULL,
    derived_id UUID NOT NULL,
    vector_entry_id UUID NOT NULL,
    vector_store TEXT NOT NULL,
    embedding_model TEXT NOT NULL,
    embedding_dimension INTEGER NOT NULL CHECK (embedding_dimension > 0),
    embedding_version TEXT NOT NULL,
    source_projection TEXT NOT NULL CHECK (source_projection IN ('canonical_summary', 'redacted_messages', 'redacted_tool_sequence')),
    source_hash TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('active', 'invalidated', 'deleted')),
    nearest_trace_ids TEXT[] NOT NULL DEFAULT '{}',
    cluster_id TEXT,
    duplicate_score REAL,
    novelty_score REAL,
    indexed_at TIMESTAMPTZ,
    invalidated_at TIMESTAMPTZ,
    deleted_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, submission_id, vector_entry_id),
    FOREIGN KEY (tenant_id, submission_id)
        REFERENCES trace_submissions (tenant_id, submission_id)
        ON DELETE CASCADE,
    FOREIGN KEY (tenant_id, derived_id)
        REFERENCES trace_derived_records (tenant_id, derived_id)
        ON DELETE CASCADE
);

CREATE INDEX idx_trace_vector_entries_source
    ON trace_vector_entries (tenant_id, submission_id, status);
CREATE INDEX idx_trace_vector_entries_cluster
    ON trace_vector_entries (tenant_id, cluster_id, status)
    WHERE cluster_id IS NOT NULL;

CREATE TABLE trace_export_manifests (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    export_manifest_id UUID NOT NULL,
    artifact_kind TEXT NOT NULL,
    purpose_code TEXT,
    audit_event_id UUID,
    source_submission_ids UUID[] NOT NULL DEFAULT '{}',
    source_submission_ids_hash TEXT NOT NULL,
    item_count INTEGER NOT NULL CHECK (item_count >= 0),
    generated_at TIMESTAMPTZ NOT NULL,
    invalidated_at TIMESTAMPTZ,
    deleted_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, export_manifest_id)
);

CREATE INDEX idx_trace_export_manifests_generated
    ON trace_export_manifests (tenant_id, generated_at DESC);
CREATE INDEX idx_trace_export_manifests_hash
    ON trace_export_manifests (tenant_id, source_submission_ids_hash);

CREATE TABLE trace_export_manifest_items (
    tenant_id TEXT NOT NULL,
    export_manifest_id UUID NOT NULL,
    submission_id UUID NOT NULL,
    trace_id UUID NOT NULL,
    derived_id UUID,
    object_ref_id UUID,
    vector_entry_id UUID,
    source_status_at_export TEXT NOT NULL,
    source_hash_at_export TEXT NOT NULL,
    source_invalidated_at TIMESTAMPTZ,
    source_invalidation_reason TEXT CHECK (
        source_invalidation_reason IS NULL
        OR source_invalidation_reason IN ('revoked', 'expired', 'purged')
    ),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, export_manifest_id, submission_id),
    FOREIGN KEY (tenant_id, export_manifest_id)
        REFERENCES trace_export_manifests (tenant_id, export_manifest_id)
        ON DELETE CASCADE,
    FOREIGN KEY (tenant_id, submission_id)
        REFERENCES trace_submissions (tenant_id, submission_id)
        ON DELETE CASCADE
);

CREATE INDEX idx_trace_export_manifest_items_source
    ON trace_export_manifest_items (tenant_id, submission_id, source_invalidated_at);
CREATE INDEX idx_trace_export_manifest_items_manifest
    ON trace_export_manifest_items (tenant_id, export_manifest_id, created_at ASC);

CREATE TABLE trace_audit_events (
    tenant_id TEXT NOT NULL,
    audit_sequence BIGINT NOT NULL,
    audit_event_id UUID NOT NULL,
    actor_principal_ref TEXT NOT NULL,
    actor_role TEXT NOT NULL,
    action TEXT NOT NULL,
    reason TEXT,
    request_id TEXT,
    submission_id UUID,
    object_ref_id UUID,
    export_manifest_id UUID,
    decision_inputs_hash TEXT,
    previous_event_hash TEXT,
    event_hash TEXT,
    canonical_event_json TEXT,
    metadata_json JSONB NOT NULL DEFAULT '{}'::JSONB,
    occurred_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, audit_event_id)
);

CREATE INDEX idx_trace_audit_events_submission
    ON trace_audit_events (tenant_id, submission_id, occurred_at DESC);
CREATE INDEX idx_trace_audit_events_action
    ON trace_audit_events (tenant_id, action, occurred_at DESC);
CREATE UNIQUE INDEX idx_trace_audit_events_tenant_sequence
    ON trace_audit_events (tenant_id, audit_sequence);

CREATE TABLE trace_credit_ledger (
    tenant_id TEXT NOT NULL,
    credit_event_id UUID NOT NULL,
    submission_id UUID NOT NULL,
    trace_id UUID NOT NULL,
    credit_account_ref TEXT NOT NULL,
    event_type TEXT NOT NULL,
    points_delta TEXT NOT NULL,
    reason TEXT NOT NULL,
    external_ref TEXT,
    actor_principal_ref TEXT NOT NULL,
    actor_role TEXT NOT NULL,
    settlement_state TEXT NOT NULL,
    occurred_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, credit_event_id),
    FOREIGN KEY (tenant_id, submission_id)
        REFERENCES trace_submissions (tenant_id, submission_id)
        ON DELETE CASCADE
);

CREATE INDEX idx_trace_credit_ledger_account
    ON trace_credit_ledger (tenant_id, credit_account_ref, occurred_at DESC);
CREATE INDEX idx_trace_credit_ledger_submission
    ON trace_credit_ledger (tenant_id, submission_id, occurred_at DESC);

CREATE TABLE trace_tombstones (
    tenant_id TEXT NOT NULL,
    tombstone_id UUID NOT NULL,
    submission_id UUID NOT NULL,
    trace_id UUID,
    redaction_hash TEXT,
    canonical_summary_hash TEXT,
    reason TEXT NOT NULL,
    effective_at TIMESTAMPTZ NOT NULL,
    retain_until TIMESTAMPTZ,
    created_by_principal_ref TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, tombstone_id),
    UNIQUE (tenant_id, submission_id),
    FOREIGN KEY (tenant_id, submission_id)
        REFERENCES trace_submissions (tenant_id, submission_id)
        ON DELETE CASCADE
);

CREATE INDEX idx_trace_tombstones_effective
    ON trace_tombstones (tenant_id, effective_at DESC);

CREATE TABLE trace_retention_jobs (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    retention_job_id UUID NOT NULL,
    purpose TEXT NOT NULL,
    dry_run BOOLEAN NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('planned', 'running', 'dry_run', 'complete', 'failed', 'paused')),
    requested_by_principal_ref TEXT NOT NULL,
    requested_by_role TEXT NOT NULL,
    purge_expired_before TIMESTAMPTZ,
    prune_export_cache BOOLEAN NOT NULL DEFAULT TRUE,
    max_export_age_hours BIGINT,
    audit_event_id UUID,
    action_counts JSONB NOT NULL DEFAULT '{}'::JSONB,
    selected_revoked_count INTEGER NOT NULL DEFAULT 0 CHECK (selected_revoked_count >= 0),
    selected_expired_count INTEGER NOT NULL DEFAULT 0 CHECK (selected_expired_count >= 0),
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, retention_job_id)
);

CREATE INDEX idx_trace_retention_jobs_created
    ON trace_retention_jobs (tenant_id, created_at DESC);
CREATE INDEX idx_trace_retention_jobs_status
    ON trace_retention_jobs (tenant_id, status, updated_at DESC);

CREATE TABLE trace_retention_job_items (
    tenant_id TEXT NOT NULL,
    retention_job_id UUID NOT NULL,
    submission_id UUID NOT NULL,
    action TEXT NOT NULL CHECK (action IN ('revoke', 'expire', 'purge')),
    status TEXT NOT NULL CHECK (status IN ('pending', 'done', 'failed', 'skipped')),
    reason TEXT NOT NULL,
    action_counts JSONB NOT NULL DEFAULT '{}'::JSONB,
    verified_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, retention_job_id, submission_id, action),
    FOREIGN KEY (tenant_id, retention_job_id)
        REFERENCES trace_retention_jobs (tenant_id, retention_job_id)
        ON DELETE CASCADE,
    FOREIGN KEY (tenant_id, submission_id)
        REFERENCES trace_submissions (tenant_id, submission_id)
        ON DELETE CASCADE
);

CREATE INDEX idx_trace_retention_job_items_submission
    ON trace_retention_job_items (tenant_id, submission_id, created_at DESC);

CREATE TABLE trace_export_access_grants (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    export_job_id UUID NOT NULL,
    grant_id UUID NOT NULL,
    caller_principal_ref TEXT NOT NULL,
    requested_dataset_kind TEXT NOT NULL,
    purpose TEXT NOT NULL,
    max_item_cap INTEGER CHECK (max_item_cap IS NULL OR max_item_cap >= 0),
    status TEXT NOT NULL CHECK (status IN ('active', 'consumed', 'revoked', 'expired')),
    requested_at TIMESTAMPTZ NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    metadata_json JSONB NOT NULL DEFAULT '{}'::JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, grant_id),
    UNIQUE (tenant_id, export_job_id, grant_id)
);

CREATE INDEX idx_trace_export_access_grants_job
    ON trace_export_access_grants (tenant_id, export_job_id);
CREATE INDEX idx_trace_export_access_grants_principal
    ON trace_export_access_grants (tenant_id, caller_principal_ref, expires_at);
CREATE INDEX idx_trace_export_access_grants_status
    ON trace_export_access_grants (tenant_id, status, expires_at);

CREATE TABLE trace_export_jobs (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    export_job_id UUID NOT NULL,
    grant_id UUID NOT NULL,
    caller_principal_ref TEXT NOT NULL,
    requested_dataset_kind TEXT NOT NULL,
    purpose TEXT NOT NULL,
    max_item_cap INTEGER CHECK (max_item_cap IS NULL OR max_item_cap >= 0),
    status TEXT NOT NULL CHECK (status IN ('queued', 'running', 'complete', 'failed', 'cancelled', 'expired')),
    requested_at TIMESTAMPTZ NOT NULL,
    started_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ,
    expires_at TIMESTAMPTZ NOT NULL,
    result_manifest_id UUID,
    item_count INTEGER CHECK (item_count IS NULL OR item_count >= 0),
    last_error TEXT,
    metadata_json JSONB NOT NULL DEFAULT '{}'::JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, export_job_id),
    FOREIGN KEY (tenant_id, grant_id)
        REFERENCES trace_export_access_grants (tenant_id, grant_id)
        ON DELETE RESTRICT
);

CREATE INDEX idx_trace_export_jobs_requested
    ON trace_export_jobs (tenant_id, requested_at DESC);
CREATE INDEX idx_trace_export_jobs_status
    ON trace_export_jobs (tenant_id, status, updated_at DESC);
CREATE INDEX idx_trace_export_jobs_grant
    ON trace_export_jobs (tenant_id, grant_id);

CREATE TABLE trace_revocation_propagation_items (
    tenant_id TEXT NOT NULL,
    propagation_item_id UUID NOT NULL,
    source_submission_id UUID NOT NULL,
    trace_id UUID NOT NULL,
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
    target_json JSONB NOT NULL,
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
    next_attempt_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    evidence_hash TEXT,
    metadata_json JSONB NOT NULL DEFAULT '{}'::JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, propagation_item_id),
    UNIQUE (tenant_id, idempotency_key),
    FOREIGN KEY (tenant_id, source_submission_id)
        REFERENCES trace_submissions (tenant_id, submission_id)
        ON DELETE CASCADE
);

CREATE INDEX idx_trace_revocation_propagation_source
    ON trace_revocation_propagation_items (tenant_id, source_submission_id, created_at ASC);
CREATE INDEX idx_trace_revocation_propagation_due
    ON trace_revocation_propagation_items (tenant_id, status, next_attempt_at, created_at ASC);
CREATE INDEX idx_trace_revocation_propagation_target
    ON trace_revocation_propagation_items (tenant_id, target_kind, updated_at DESC);

ALTER TABLE trace_tenants ENABLE ROW LEVEL SECURITY;
CREATE POLICY trace_corpus_tenant_isolation ON trace_tenants
    USING (tenant_id = current_setting('tracedao.trace_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('tracedao.trace_tenant_id', true));

ALTER TABLE trace_tenant_policies ENABLE ROW LEVEL SECURITY;
CREATE POLICY trace_corpus_tenant_isolation ON trace_tenant_policies
    USING (tenant_id = current_setting('tracedao.trace_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('tracedao.trace_tenant_id', true));

ALTER TABLE trace_tenant_access_grants ENABLE ROW LEVEL SECURITY;
CREATE POLICY trace_corpus_tenant_isolation ON trace_tenant_access_grants
    USING (tenant_id = current_setting('tracedao.trace_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('tracedao.trace_tenant_id', true));

ALTER TABLE trace_submissions ENABLE ROW LEVEL SECURITY;
CREATE POLICY trace_corpus_tenant_isolation ON trace_submissions
    USING (tenant_id = current_setting('tracedao.trace_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('tracedao.trace_tenant_id', true));

ALTER TABLE trace_object_refs ENABLE ROW LEVEL SECURITY;
CREATE POLICY trace_corpus_tenant_isolation ON trace_object_refs
    USING (tenant_id = current_setting('tracedao.trace_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('tracedao.trace_tenant_id', true));

ALTER TABLE trace_derived_records ENABLE ROW LEVEL SECURITY;
CREATE POLICY trace_corpus_tenant_isolation ON trace_derived_records
    USING (tenant_id = current_setting('tracedao.trace_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('tracedao.trace_tenant_id', true));

ALTER TABLE trace_vector_entries ENABLE ROW LEVEL SECURITY;
CREATE POLICY trace_corpus_tenant_isolation ON trace_vector_entries
    USING (tenant_id = current_setting('tracedao.trace_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('tracedao.trace_tenant_id', true));

ALTER TABLE trace_export_manifests ENABLE ROW LEVEL SECURITY;
CREATE POLICY trace_corpus_tenant_isolation ON trace_export_manifests
    USING (tenant_id = current_setting('tracedao.trace_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('tracedao.trace_tenant_id', true));

ALTER TABLE trace_export_manifest_items ENABLE ROW LEVEL SECURITY;
CREATE POLICY trace_corpus_tenant_isolation ON trace_export_manifest_items
    USING (tenant_id = current_setting('tracedao.trace_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('tracedao.trace_tenant_id', true));

ALTER TABLE trace_audit_events ENABLE ROW LEVEL SECURITY;
CREATE POLICY trace_corpus_tenant_isolation ON trace_audit_events
    USING (tenant_id = current_setting('tracedao.trace_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('tracedao.trace_tenant_id', true));

ALTER TABLE trace_credit_ledger ENABLE ROW LEVEL SECURITY;
CREATE POLICY trace_corpus_tenant_isolation ON trace_credit_ledger
    USING (tenant_id = current_setting('tracedao.trace_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('tracedao.trace_tenant_id', true));

ALTER TABLE trace_tombstones ENABLE ROW LEVEL SECURITY;
CREATE POLICY trace_corpus_tenant_isolation ON trace_tombstones
    USING (tenant_id = current_setting('tracedao.trace_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('tracedao.trace_tenant_id', true));

ALTER TABLE trace_retention_jobs ENABLE ROW LEVEL SECURITY;
CREATE POLICY trace_corpus_tenant_isolation ON trace_retention_jobs
    USING (tenant_id = current_setting('tracedao.trace_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('tracedao.trace_tenant_id', true));

ALTER TABLE trace_retention_job_items ENABLE ROW LEVEL SECURITY;
CREATE POLICY trace_corpus_tenant_isolation ON trace_retention_job_items
    USING (tenant_id = current_setting('tracedao.trace_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('tracedao.trace_tenant_id', true));

ALTER TABLE trace_export_access_grants ENABLE ROW LEVEL SECURITY;
CREATE POLICY trace_corpus_tenant_isolation ON trace_export_access_grants
    USING (tenant_id = current_setting('tracedao.trace_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('tracedao.trace_tenant_id', true));

ALTER TABLE trace_export_jobs ENABLE ROW LEVEL SECURITY;
CREATE POLICY trace_corpus_tenant_isolation ON trace_export_jobs
    USING (tenant_id = current_setting('tracedao.trace_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('tracedao.trace_tenant_id', true));

ALTER TABLE trace_revocation_propagation_items ENABLE ROW LEVEL SECURITY;
CREATE POLICY trace_corpus_tenant_isolation ON trace_revocation_propagation_items
    USING (tenant_id = current_setting('tracedao.trace_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('tracedao.trace_tenant_id', true));
