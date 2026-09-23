-- Approved-content reference for the Review commit, and the admission usage
-- rows the receipt path counts before it stores anything.

ALTER TABLE pipeline_runs
    ADD COLUMN approved_object_ref_id UUID,
    ADD COLUMN approved_content_hash TEXT CHECK (
        approved_content_hash IS NULL OR approved_content_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    ADD CONSTRAINT pipeline_runs_approved_content_shape CHECK (
        (approved_object_ref_id IS NULL) = (approved_content_hash IS NULL)
        AND (approved_object_ref_id IS NULL) = (approved_revision_id IS NULL)
    ),
    ADD CONSTRAINT pipeline_runs_approved_object_ref_fk
        FOREIGN KEY (tenant_id, submission_id, approved_object_ref_id)
        REFERENCES trace_object_refs (tenant_id, submission_id, object_ref_id)
        ON DELETE RESTRICT;

CREATE TABLE pipeline_admission_usage (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    request_idempotency_key TEXT NOT NULL CHECK (
        request_idempotency_key ~ '^sha256:[0-9a-f]{64}$'
    ),
    principal_ref_hash TEXT NOT NULL CHECK (principal_ref_hash ~ '^sha256:[0-9a-f]{64}$'),
    counted_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, request_idempotency_key)
);

CREATE INDEX idx_pipeline_admission_usage_tenant_window
    ON pipeline_admission_usage (tenant_id, counted_at);
CREATE INDEX idx_pipeline_admission_usage_principal_window
    ON pipeline_admission_usage (tenant_id, principal_ref_hash, counted_at);

ALTER TABLE pipeline_admission_usage ENABLE ROW LEVEL SECURITY;
ALTER TABLE pipeline_admission_usage FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON pipeline_admission_usage;
CREATE POLICY trace_corpus_tenant_isolation ON pipeline_admission_usage
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());
