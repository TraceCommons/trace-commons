-- The mutable versioned-pipeline run queue and immutable outcomes live beside
-- the existing submission, artifact, and registry rows.

CREATE TABLE pipeline_runs (
    tenant_id TEXT NOT NULL,
    run_id UUID NOT NULL,
    submission_id UUID NOT NULL,
    trace_id UUID NOT NULL,
    bundle_id TEXT NOT NULL CHECK (bundle_id ~ '^sha256:[0-9a-f]{64}$'),
    request_idempotency_key TEXT NOT NULL CHECK (
        request_idempotency_key ~ '^sha256:[0-9a-f]{64}$'
    ),
    request_content_hash TEXT NOT NULL CHECK (
        request_content_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    source_object_ref_id UUID NOT NULL,
    approved_revision_id UUID,
    next_phase TEXT NOT NULL CHECK (
        next_phase IN ('admission', 'review', 'score', 'settle', 'none')
    ),
    state TEXT NOT NULL CHECK (
        state IN ('pending', 'leased', 'complete', 'failed')
    ),
    last_error_label TEXT,
    index_membership TEXT NOT NULL DEFAULT 'undecided' CHECK (
        index_membership IN ('undecided', 'excluded', 'included')
    ),
    admission_decision TEXT NOT NULL CHECK (
        admission_decision IN ('admit', 'quarantine', 'reject')
    ),
    admission_reason TEXT CHECK (
        admission_reason IS NULL OR admission_reason ~ '^[a-z0-9_]{1,64}$'
    ),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, run_id),
    UNIQUE (tenant_id, request_idempotency_key),
    CONSTRAINT pipeline_runs_admission_reason_shape CHECK (
        (admission_decision = 'admit' AND admission_reason IS NULL)
        OR (admission_decision <> 'admit' AND admission_reason IS NOT NULL)
    ),
    FOREIGN KEY (tenant_id, submission_id)
        REFERENCES trace_submissions (tenant_id, submission_id)
        ON DELETE CASCADE,
    FOREIGN KEY (tenant_id, submission_id, source_object_ref_id)
        REFERENCES trace_object_refs (tenant_id, submission_id, object_ref_id)
        ON DELETE RESTRICT
);

CREATE INDEX idx_pipeline_runs_work
    ON pipeline_runs (tenant_id, state, next_phase, created_at ASC);
CREATE INDEX idx_pipeline_runs_submission
    ON pipeline_runs (tenant_id, submission_id, created_at DESC);

CREATE TABLE phase_outcomes (
    tenant_id TEXT NOT NULL,
    outcome_id UUID NOT NULL,
    run_id UUID NOT NULL,
    trace_id UUID NOT NULL,
    phase TEXT NOT NULL CHECK (
        phase IN ('admission', 'review', 'score', 'settle')
    ),
    bundle_id TEXT NOT NULL CHECK (bundle_id ~ '^sha256:[0-9a-f]{64}$'),
    outcome_schema_id TEXT NOT NULL,
    outcome_schema_version INTEGER NOT NULL CHECK (outcome_schema_version > 0),
    decision JSONB NOT NULL,
    evidence JSONB NOT NULL,
    evaluation JSONB NOT NULL,
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, outcome_id),
    UNIQUE (tenant_id, run_id, phase),
    FOREIGN KEY (tenant_id, run_id)
        REFERENCES pipeline_runs (tenant_id, run_id)
        ON DELETE CASCADE
);

CREATE INDEX idx_phase_outcomes_run
    ON phase_outcomes (tenant_id, run_id, recorded_at ASC);

ALTER TABLE pipeline_runs ENABLE ROW LEVEL SECURITY;
ALTER TABLE pipeline_runs FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON pipeline_runs;
CREATE POLICY trace_corpus_tenant_isolation ON pipeline_runs
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

ALTER TABLE phase_outcomes ENABLE ROW LEVEL SECURITY;
ALTER TABLE phase_outcomes FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON phase_outcomes;
CREATE POLICY trace_corpus_tenant_isolation ON phase_outcomes
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

CREATE FUNCTION reject_phase_outcome_mutation()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION 'phase outcomes are immutable';
END;
$$;

CREATE TRIGGER phase_outcomes_reject_update
    BEFORE UPDATE ON phase_outcomes
    FOR EACH ROW EXECUTE FUNCTION reject_phase_outcome_mutation();

CREATE TRIGGER phase_outcomes_reject_delete
    BEFORE DELETE ON phase_outcomes
    FOR EACH ROW EXECUTE FUNCTION reject_phase_outcome_mutation();
