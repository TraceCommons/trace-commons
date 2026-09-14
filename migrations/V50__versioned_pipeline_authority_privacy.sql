-- Phase 4 authority, lifecycle guards, admission limits, and human Review
-- evidence for the versioned pipeline.

ALTER TABLE pipeline_runs
    DROP CONSTRAINT pipeline_runs_index_write_state_check,
    DROP CONSTRAINT pipeline_runs_index_command_shape,
    ADD CONSTRAINT pipeline_runs_index_write_state_check CHECK (
        index_write_state IN ('none', 'pending', 'complete', 'failed', 'cancelled')
    ),
    ADD CONSTRAINT pipeline_runs_index_command_shape CHECK (
        (
            index_membership = 'included'
            AND index_command_ref IS NOT NULL
            AND index_command_hash IS NOT NULL
        )
        OR (
            index_membership = 'excluded'
            AND index_write_state = 'cancelled'
            AND index_command_ref IS NOT NULL
            AND index_command_hash IS NOT NULL
        )
        OR (
            index_membership <> 'included'
            AND index_command_ref IS NULL
            AND index_command_hash IS NULL
        )
    ),
    ADD COLUMN admission_decision TEXT NOT NULL DEFAULT 'admit'
        CHECK (admission_decision IN ('admit', 'quarantine', 'reject')),
    ADD COLUMN admission_reason TEXT
        CHECK (
            admission_reason IS NULL
            OR admission_reason ~ '^[a-z0-9_]{1,64}$'
        ),
    ADD COLUMN transformed_object_ref_id UUID,
    ADD COLUMN transformed_content_hash TEXT
        CHECK (
            transformed_content_hash IS NULL
            OR transformed_content_hash ~ '^sha256:[0-9a-f]{64}$'
        ),
    ADD COLUMN index_invalidation_state TEXT NOT NULL DEFAULT 'none'
        CHECK (
            index_invalidation_state IN ('none', 'pending', 'complete', 'failed')
        ),
    ADD CONSTRAINT pipeline_runs_admission_reason_shape CHECK (
        (admission_decision = 'admit' AND admission_reason IS NULL)
        OR (admission_decision <> 'admit' AND admission_reason IS NOT NULL)
    ),
    ADD CONSTRAINT pipeline_runs_transformed_artifact_shape CHECK (
        (transformed_object_ref_id IS NULL AND transformed_content_hash IS NULL)
        OR (transformed_object_ref_id IS NOT NULL AND transformed_content_hash IS NOT NULL)
    ),
    ADD CONSTRAINT pipeline_runs_transformed_object_ref_fk
        FOREIGN KEY (tenant_id, submission_id, transformed_object_ref_id)
        REFERENCES trace_object_refs (tenant_id, submission_id, object_ref_id)
        ON DELETE RESTRICT;

ALTER TABLE pipeline_bundle_policy_status
    ADD COLUMN operational_status TEXT NOT NULL DEFAULT 'runnable'
        CHECK (operational_status IN ('runnable', 'suspended', 'terminated')),
    ADD COLUMN updated_by_principal_ref TEXT,
    ADD COLUMN reason_code TEXT
        CHECK (
            reason_code IS NULL
            OR reason_code ~ '^[a-z0-9_]{1,64}$'
        ),
    ADD CONSTRAINT pipeline_policy_status_runnable_shape CHECK (
        runnable = (operational_status = 'runnable')
    );

CREATE TABLE pipeline_policy_interventions (
    tenant_id TEXT NOT NULL,
    intervention_id UUID NOT NULL,
    bundle_id TEXT NOT NULL,
    phase TEXT NOT NULL CHECK (
        phase IN ('admission', 'review', 'score', 'settle')
    ),
    action TEXT NOT NULL CHECK (
        action IN ('suspend', 'resume', 'terminate')
    ),
    actor_principal_ref TEXT NOT NULL CHECK (
        actor_principal_ref ~ '^(operator|admin)_sha256:[0-9a-f]{64}$'
    ),
    reason_code TEXT NOT NULL CHECK (reason_code ~ '^[a-z0-9_]{1,64}$'),
    previous_status TEXT NOT NULL CHECK (
        previous_status IN ('runnable', 'suspended', 'terminated')
    ),
    resulting_status TEXT NOT NULL CHECK (
        resulting_status IN ('runnable', 'suspended', 'terminated')
    ),
    evidence_hash TEXT NOT NULL CHECK (
        evidence_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, intervention_id),
    FOREIGN KEY (tenant_id, bundle_id, phase)
        REFERENCES pipeline_bundle_policy_status (tenant_id, bundle_id, phase)
        ON DELETE RESTRICT
);

CREATE INDEX idx_pipeline_policy_interventions_bundle
    ON pipeline_policy_interventions (
        tenant_id, bundle_id, phase, recorded_at DESC
    );

CREATE TABLE pipeline_admission_usage (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    request_idempotency_key TEXT NOT NULL CHECK (
        request_idempotency_key ~ '^sha256:[0-9a-f]{64}$'
    ),
    principal_ref TEXT NOT NULL CHECK (
        principal_ref ~ '^principal_sha256:[a-z0-9]{1,64}$'
    ),
    window_started_at TIMESTAMPTZ NOT NULL,
    counted_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, request_idempotency_key)
);

CREATE INDEX idx_pipeline_admission_usage_tenant_window
    ON pipeline_admission_usage (tenant_id, window_started_at);
CREATE INDEX idx_pipeline_admission_usage_principal_window
    ON pipeline_admission_usage (tenant_id, principal_ref, window_started_at);

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
    PRIMARY KEY (tenant_id, run_id),
    FOREIGN KEY (tenant_id, run_id)
        REFERENCES pipeline_runs (tenant_id, run_id)
        ON DELETE CASCADE,
    FOREIGN KEY (tenant_id, submission_id)
        REFERENCES trace_submissions (tenant_id, submission_id)
        ON DELETE CASCADE
);

CREATE FUNCTION reject_pipeline_policy_intervention_mutation()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION 'pipeline policy interventions are immutable';
END;
$$;

CREATE TRIGGER pipeline_policy_interventions_reject_update
    BEFORE UPDATE ON pipeline_policy_interventions
    FOR EACH ROW EXECUTE FUNCTION reject_pipeline_policy_intervention_mutation();

CREATE TRIGGER pipeline_policy_interventions_reject_delete
    BEFORE DELETE ON pipeline_policy_interventions
    FOR EACH ROW EXECUTE FUNCTION reject_pipeline_policy_intervention_mutation();

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

ALTER TABLE pipeline_policy_interventions ENABLE ROW LEVEL SECURITY;
ALTER TABLE pipeline_policy_interventions FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON pipeline_policy_interventions;
CREATE POLICY trace_corpus_tenant_isolation ON pipeline_policy_interventions
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

ALTER TABLE pipeline_admission_usage ENABLE ROW LEVEL SECURITY;
ALTER TABLE pipeline_admission_usage FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON pipeline_admission_usage;
CREATE POLICY trace_corpus_tenant_isolation ON pipeline_admission_usage
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

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
