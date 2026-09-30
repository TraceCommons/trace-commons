-- Review claims and assessments, index invalidations, the pipeline_runs
-- index-invalidation state, and the NEAR payout work index for the
-- versioned pipeline (delivery PR 3).
--
-- The NEAR payout needs no grant here: it reads and writes
-- trace_near_credit_outbox and reads trace_credit_settlement_batches, both
-- older than V62 (the pilot's runtime holds table-wide privileges on them,
-- as V94 notes), and it updates only pipeline_run_settlements columns V94
-- already grants (payout_state, last_error_label, updated_at).
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
    -- Cascade, not restrict: like `pipeline_bundle_policy_status` (V93) and
    -- `pipeline_review_claims` above, this table has no foreign key of its
    -- own straight to `trace_tenants` or `trace_submissions`, so an
    -- assessment has no other path out when its run goes away. The `UNIQUE
    -- (tenant_id, run_id)` above means this table only ever holds one row
    -- per live run.
    FOREIGN KEY (tenant_id, run_id)
        REFERENCES pipeline_runs (tenant_id, run_id)
        ON DELETE CASCADE
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

-- The NEAR payout pass's work list (PgPipelineStore::
-- list_runs_with_pending_payout): a tenant's batched Trace Credit legs on
-- the `near` rail whose payout is still to make or to confirm, least
-- recently updated first.
CREATE INDEX idx_pipeline_run_settlements_payout_work
    ON pipeline_run_settlements (tenant_id, updated_at, run_id)
    WHERE payout_rail = 'near'
      AND payout_state IN ('pending', 'submitted')
      AND settlement_batch_id IS NOT NULL;

-- Assessments are append-only, the same shape as `phase_outcomes` (V92) and
-- `pipeline_bundle_packages` (V93): neither an UPDATE nor a direct DELETE is
-- ever allowed, but a DELETE arriving through a cascade (the run this
-- assessment belongs to was deleted, taking the whole submission or tenant
-- with it) is let through, so a tenant or a submission with a recorded
-- assessment can still be removed. See `reject_phase_outcome_mutation`'s
-- comment (V92) for why `pg_trigger_depth() > 1` is the direct/cascade
-- boundary.
CREATE FUNCTION reject_pipeline_review_assessment_mutation()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF TG_OP = 'DELETE' AND pg_trigger_depth() > 1 THEN
        RETURN OLD;
    END IF;
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
        RAISE EXCEPTION 'V102: trace_ingest_runtime is missing; V90 creates it';
    END IF;
END $$;

-- pipeline_index_invalidations: stopping an index write that may have
-- written entries queues the revision's removal (INSERT ... ON CONFLICT DO
-- NOTHING), and the lifecycle and operational summaries count the tenant's
-- pending and failed invalidations. The invalidation worker lists the due
-- rows, claims one by moving next_attempt_at to its lease's end, and
-- records the attempt: a removal sets state, completed_at and
-- last_error_label; a failure charges attempt_count and sets state,
-- last_error_label and next_attempt_at. Nothing updates an invalidation's
-- run, submission, revision, reason, requested_at or max_attempts.
GRANT SELECT, INSERT ON pipeline_index_invalidations TO trace_ingest_runtime;
GRANT UPDATE (state, completed_at, attempt_count, next_attempt_at, last_error_label)
    ON pipeline_index_invalidations TO trace_ingest_runtime;

-- pipeline_runs: queueing an invalidation marks the run's
-- index_invalidation_state pending, and the invalidation worker marks it
-- complete or failed.
GRANT UPDATE (index_invalidation_state) ON pipeline_runs TO trace_ingest_runtime;

-- pipeline_review_claims: a reviewer's claim inserts the row, or takes over
-- an expired one or renews its own (INSERT ... ON CONFLICT DO UPDATE of the
-- reviewer, the lease and claimed_at), the assessment locks it
-- (`FOR UPDATE OF c`) and deletes the spent claim. Nothing updates a
-- claim's tenant or run.
GRANT SELECT, INSERT, DELETE ON pipeline_review_claims TO trace_ingest_runtime;
GRANT UPDATE (reviewer_principal_ref, lease_token, lease_expires_at, claimed_at)
    ON pipeline_review_claims TO trace_ingest_runtime;

-- pipeline_review_assessments: append-only. An assessment inserts its row,
-- and the claim, the review queue and every Review attempt read it. No
-- UPDATE or DELETE: the triggers above refuse both, and an assessment
-- leaves only with its run, through the foreign key's cascade, which runs
-- as the table owner.
GRANT SELECT, INSERT ON pipeline_review_assessments TO trace_ingest_runtime;
