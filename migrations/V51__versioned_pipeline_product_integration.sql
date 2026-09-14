-- Phase 6 product projections and immutable customer export snapshots.
--
-- Submission status remains derived from authoritative pipeline, outcome,
-- credit, and outbox records. It intentionally has no read-model table.

CREATE TABLE pipeline_export_snapshots (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    snapshot_id UUID NOT NULL,
    request_idempotency_key TEXT NOT NULL CHECK (
        request_idempotency_key ~ '^sha256:[0-9a-f]{64}$'
    ),
    requester_principal_ref TEXT NOT NULL CHECK (
        requester_principal_ref ~ '^(principal|exporter)_sha256:[a-z0-9]{1,64}$'
    ),
    allowed_use TEXT NOT NULL CHECK (allowed_use ~ '^[a-z0-9_]{1,64}$'),
    purpose_hash TEXT NOT NULL CHECK (purpose_hash ~ '^sha256:[0-9a-f]{64}$'),
    selection_policy_id TEXT NOT NULL CHECK (
        selection_policy_id ~ '^[a-z0-9_.-]{1,128}$'
    ),
    source_list_hash TEXT NOT NULL CHECK (
        source_list_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    item_count INTEGER NOT NULL CHECK (item_count >= 0 AND item_count <= 500),
    state TEXT NOT NULL DEFAULT 'ready' CHECK (
        state IN ('ready', 'complete', 'invalidated')
    ),
    export_manifest_id UUID,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at TIMESTAMPTZ,
    invalidated_at TIMESTAMPTZ,
    PRIMARY KEY (tenant_id, snapshot_id),
    UNIQUE (tenant_id, request_idempotency_key),
    CHECK (
        (state = 'ready' AND export_manifest_id IS NULL AND completed_at IS NULL)
        OR
        (state = 'complete' AND export_manifest_id IS NOT NULL AND completed_at IS NOT NULL)
        OR
        (state = 'invalidated' AND invalidated_at IS NOT NULL)
    )
);

CREATE TABLE pipeline_export_snapshot_items (
    tenant_id TEXT NOT NULL,
    snapshot_id UUID NOT NULL,
    ordinal INTEGER NOT NULL CHECK (ordinal >= 0 AND ordinal < 500),
    run_id UUID NOT NULL,
    submission_id UUID NOT NULL,
    trace_id UUID NOT NULL,
    registry_revision_id UUID NOT NULL,
    source_object_ref_id UUID NOT NULL,
    source_content_hash TEXT NOT NULL CHECK (
        source_content_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    bundle_id TEXT NOT NULL CHECK (bundle_id ~ '^sha256:[0-9a-f]{64}$'),
    outcome_schema_id TEXT NOT NULL,
    outcome_schema_version INTEGER NOT NULL CHECK (outcome_schema_version > 0),
    authorized_view_schema_id TEXT NOT NULL,
    consent_scopes JSONB NOT NULL CHECK (jsonb_typeof(consent_scopes) = 'array'),
    allowed_uses JSONB NOT NULL CHECK (jsonb_typeof(allowed_uses) = 'array'),
    invalidated_at TIMESTAMPTZ,
    invalidation_reason TEXT CHECK (
        invalidation_reason IS NULL
        OR invalidation_reason IN ('withdrawn', 'revoked', 'expired', 'purged')
    ),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, snapshot_id, registry_revision_id),
    UNIQUE (tenant_id, snapshot_id, ordinal),
    FOREIGN KEY (tenant_id, snapshot_id)
        REFERENCES pipeline_export_snapshots (tenant_id, snapshot_id)
        ON DELETE RESTRICT,
    FOREIGN KEY (tenant_id, run_id)
        REFERENCES pipeline_runs (tenant_id, run_id)
        ON DELETE RESTRICT,
    FOREIGN KEY (tenant_id, submission_id)
        REFERENCES trace_submissions (tenant_id, submission_id)
        ON DELETE RESTRICT,
    FOREIGN KEY (tenant_id, submission_id, source_object_ref_id)
        REFERENCES trace_object_refs (tenant_id, submission_id, object_ref_id)
        ON DELETE RESTRICT,
    CHECK (
        (invalidated_at IS NULL AND invalidation_reason IS NULL)
        OR (invalidated_at IS NOT NULL AND invalidation_reason IS NOT NULL)
    )
);

CREATE INDEX idx_pipeline_export_snapshot_items_submission
    ON pipeline_export_snapshot_items (
        tenant_id, submission_id, invalidated_at, snapshot_id
    );

CREATE FUNCTION reject_pipeline_export_snapshot_identity_mutation()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF NEW.tenant_id IS DISTINCT FROM OLD.tenant_id
       OR NEW.snapshot_id IS DISTINCT FROM OLD.snapshot_id
       OR NEW.request_idempotency_key IS DISTINCT FROM OLD.request_idempotency_key
       OR NEW.requester_principal_ref IS DISTINCT FROM OLD.requester_principal_ref
       OR NEW.allowed_use IS DISTINCT FROM OLD.allowed_use
       OR NEW.purpose_hash IS DISTINCT FROM OLD.purpose_hash
       OR NEW.selection_policy_id IS DISTINCT FROM OLD.selection_policy_id
       OR NEW.source_list_hash IS DISTINCT FROM OLD.source_list_hash
       OR NEW.item_count IS DISTINCT FROM OLD.item_count
       OR NEW.created_at IS DISTINCT FROM OLD.created_at
    THEN
        RAISE EXCEPTION 'pipeline export snapshot identity is immutable';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER pipeline_export_snapshots_reject_identity_update
    BEFORE UPDATE ON pipeline_export_snapshots
    FOR EACH ROW EXECUTE FUNCTION reject_pipeline_export_snapshot_identity_mutation();

CREATE FUNCTION reject_pipeline_export_snapshot_item_identity_mutation()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF NEW.tenant_id IS DISTINCT FROM OLD.tenant_id
       OR NEW.snapshot_id IS DISTINCT FROM OLD.snapshot_id
       OR NEW.ordinal IS DISTINCT FROM OLD.ordinal
       OR NEW.run_id IS DISTINCT FROM OLD.run_id
       OR NEW.submission_id IS DISTINCT FROM OLD.submission_id
       OR NEW.trace_id IS DISTINCT FROM OLD.trace_id
       OR NEW.registry_revision_id IS DISTINCT FROM OLD.registry_revision_id
       OR NEW.source_object_ref_id IS DISTINCT FROM OLD.source_object_ref_id
       OR NEW.source_content_hash IS DISTINCT FROM OLD.source_content_hash
       OR NEW.bundle_id IS DISTINCT FROM OLD.bundle_id
       OR NEW.outcome_schema_id IS DISTINCT FROM OLD.outcome_schema_id
       OR NEW.outcome_schema_version IS DISTINCT FROM OLD.outcome_schema_version
       OR NEW.authorized_view_schema_id IS DISTINCT FROM OLD.authorized_view_schema_id
       OR NEW.consent_scopes IS DISTINCT FROM OLD.consent_scopes
       OR NEW.allowed_uses IS DISTINCT FROM OLD.allowed_uses
       OR NEW.created_at IS DISTINCT FROM OLD.created_at
    THEN
        RAISE EXCEPTION 'pipeline export snapshot item identity is immutable';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER pipeline_export_snapshot_items_reject_identity_update
    BEFORE UPDATE ON pipeline_export_snapshot_items
    FOR EACH ROW EXECUTE FUNCTION reject_pipeline_export_snapshot_item_identity_mutation();

CREATE FUNCTION reject_pipeline_export_snapshot_delete()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION 'pipeline export snapshots are retained';
END;
$$;

CREATE TRIGGER pipeline_export_snapshots_reject_delete
    BEFORE DELETE ON pipeline_export_snapshots
    FOR EACH ROW EXECUTE FUNCTION reject_pipeline_export_snapshot_delete();

CREATE TRIGGER pipeline_export_snapshot_items_reject_delete
    BEFORE DELETE ON pipeline_export_snapshot_items
    FOR EACH ROW EXECUTE FUNCTION reject_pipeline_export_snapshot_delete();

ALTER TABLE pipeline_export_snapshots ENABLE ROW LEVEL SECURITY;
ALTER TABLE pipeline_export_snapshots FORCE ROW LEVEL SECURITY;
CREATE POLICY trace_corpus_tenant_isolation ON pipeline_export_snapshots
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

ALTER TABLE pipeline_export_snapshot_items ENABLE ROW LEVEL SECURITY;
ALTER TABLE pipeline_export_snapshot_items FORCE ROW LEVEL SECURITY;
CREATE POLICY trace_corpus_tenant_isolation ON pipeline_export_snapshot_items
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

ALTER TABLE pipeline_index_invalidations
    ADD COLUMN attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
    ADD COLUMN max_attempts INTEGER NOT NULL DEFAULT 5 CHECK (max_attempts > 0),
    ADD COLUMN next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    ADD COLUMN last_error_label TEXT CHECK (
        last_error_label IS NULL
        OR last_error_label ~ '^[a-z0-9_]{1,64}$'
    ),
    ADD CONSTRAINT pipeline_index_invalidation_attempt_limit CHECK (
        attempt_count <= max_attempts
    );

CREATE INDEX idx_pipeline_index_invalidations_work
    ON pipeline_index_invalidations (
        state, next_attempt_at, requested_at, run_id
    );
