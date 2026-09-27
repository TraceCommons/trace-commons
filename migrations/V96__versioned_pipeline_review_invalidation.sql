-- Review claims and assessments, index invalidations, and the
-- pipeline_runs index-invalidation state for the versioned pipeline
-- (delivery PR 3).
--
-- pipeline_runs.admission_reason already exists (V92,
-- pipeline_runs_admission_reason_shape); this migration does not touch it.

ALTER TABLE pipeline_runs
    ADD COLUMN index_invalidation_state TEXT NOT NULL DEFAULT 'none'
        CHECK (index_invalidation_state IN ('none', 'pending', 'complete', 'failed'));

CREATE TABLE pipeline_review_claims (
    tenant_id TEXT NOT NULL,
    run_id UUID NOT NULL,
    reviewer_principal_ref TEXT NOT NULL CHECK (
        reviewer_principal_ref ~ '^reviewer_sha256:[0-9a-f]{64}$'
    ),
    lease_token UUID NOT NULL,
    lease_expires_at TIMESTAMPTZ NOT NULL,
    claimed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, run_id),
    FOREIGN KEY (tenant_id, run_id)
        REFERENCES pipeline_runs (tenant_id, run_id)
        ON DELETE CASCADE
);

CREATE TABLE pipeline_review_assessments (
    tenant_id TEXT NOT NULL,
    assessment_id UUID NOT NULL,
    run_id UUID NOT NULL,
    reviewer_principal_ref TEXT NOT NULL CHECK (
        reviewer_principal_ref ~ '^reviewer_sha256:[0-9a-f]{64}$'
    ),
    recommendation TEXT NOT NULL CHECK (
        recommendation IN ('approve', 'reject')
    ),
    reason_code TEXT NOT NULL CHECK (reason_code ~ '^[a-z0-9_]{1,64}$'),
    resolved_quarantine_reasons JSONB NOT NULL DEFAULT '[]'::JSONB,
    evidence_hash TEXT NOT NULL CHECK (
        evidence_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, assessment_id),
    UNIQUE (tenant_id, run_id),
    FOREIGN KEY (tenant_id, run_id)
        REFERENCES pipeline_runs (tenant_id, run_id)
        ON DELETE RESTRICT
);

CREATE TABLE pipeline_index_invalidations (
    tenant_id TEXT NOT NULL,
    run_id UUID NOT NULL,
    submission_id UUID NOT NULL,
    registry_revision_id UUID NOT NULL,
    reason_code TEXT NOT NULL CHECK (reason_code ~ '^[a-z0-9_]{1,64}$'),
    state TEXT NOT NULL DEFAULT 'pending' CHECK (
        state IN ('pending', 'complete', 'failed')
    ),
    requested_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at TIMESTAMPTZ,
    attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
    max_attempts INTEGER NOT NULL DEFAULT 5 CHECK (max_attempts > 0),
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_error_label TEXT CHECK (
        last_error_label IS NULL
        OR last_error_label ~ '^[a-z0-9_]{1,64}$'
    ),
    PRIMARY KEY (tenant_id, run_id),
    CONSTRAINT pipeline_index_invalidation_attempt_limit CHECK (
        attempt_count <= max_attempts
    ),
    FOREIGN KEY (tenant_id, run_id)
        REFERENCES pipeline_runs (tenant_id, run_id)
        ON DELETE CASCADE,
    FOREIGN KEY (tenant_id, submission_id)
        REFERENCES trace_submissions (tenant_id, submission_id)
        ON DELETE CASCADE
);

CREATE INDEX idx_pipeline_index_invalidations_work
    ON pipeline_index_invalidations (
        state, next_attempt_at, requested_at, run_id
    );

CREATE FUNCTION reject_pipeline_review_assessment_mutation()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION 'pipeline review assessments are immutable';
END;
$$;

CREATE TRIGGER pipeline_review_assessments_reject_update
    BEFORE UPDATE ON pipeline_review_assessments
    FOR EACH ROW EXECUTE FUNCTION reject_pipeline_review_assessment_mutation();

CREATE TRIGGER pipeline_review_assessments_reject_delete
    BEFORE DELETE ON pipeline_review_assessments
    FOR EACH ROW EXECUTE FUNCTION reject_pipeline_review_assessment_mutation();

ALTER TABLE pipeline_review_claims ENABLE ROW LEVEL SECURITY;
ALTER TABLE pipeline_review_claims FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON pipeline_review_claims;
CREATE POLICY trace_corpus_tenant_isolation ON pipeline_review_claims
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

ALTER TABLE pipeline_review_assessments ENABLE ROW LEVEL SECURITY;
ALTER TABLE pipeline_review_assessments FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON pipeline_review_assessments;
CREATE POLICY trace_corpus_tenant_isolation ON pipeline_review_assessments
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

ALTER TABLE pipeline_index_invalidations ENABLE ROW LEVEL SECURITY;
ALTER TABLE pipeline_index_invalidations FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON pipeline_index_invalidations;
CREATE POLICY trace_corpus_tenant_isolation ON pipeline_index_invalidations
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

-- The ingest runtime's grants on what this migration adds, on V92's terms:
-- what the pipeline code reads and writes, and nothing broader. A table or
-- column the code does not use gets no grant.
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_ingest_runtime') THEN
        RAISE EXCEPTION 'V96: trace_ingest_runtime is missing; V90 creates it';
    END IF;
END $$;
