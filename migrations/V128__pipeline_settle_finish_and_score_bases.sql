-- V128 (#1313): a Settle run whose legs are all terminal finishes instead of
-- failing; a paid run whose Settle commit is refused for good waits for an
-- operator; and a Score that runs under the runtime controls records the
-- unapplied runs it counted.
-- Design: docs/superpowers/specs/2026-10-14-pipeline-duplicate-basis-repair-design.md.

-- `awaiting_operator`: a Settle run with a paid leg whose commit was refused
-- for good (`pipeline_gate_decision_conflict`,
-- `pipeline_gate_decision_evidence_incomplete`). It is not failed, so its
-- paid revision stays in the index (N-7 applies only to a run that fails),
-- and no claim query selects it. An operator ends it
-- (`PgPipelineStore::resolve_operator_held_run`).
ALTER TABLE pipeline_runs
    DROP CONSTRAINT IF EXISTS pipeline_runs_state_check;
ALTER TABLE pipeline_runs
    ADD CONSTRAINT pipeline_runs_state_check CHECK (
        state IN ('pending', 'leased', 'retry', 'awaiting_review',
                  'awaiting_operator', 'complete', 'failed')
    );

-- The time a failure path gave a finishable Settle run its one extra attempt
-- (an attempt given back, `attempt_count = max_attempts - 1`). Set once:
-- a run that has it fails as before when it runs out again.
ALTER TABLE pipeline_runs
    ADD COLUMN IF NOT EXISTS settle_finish_granted_at TIMESTAMPTZ;

GRANT UPDATE (settle_finish_granted_at) ON pipeline_runs TO trace_ingest_runtime;

-- The time the run was last held for an operator. Set by every hold, never
-- cleared: the review audit pass reads it, not the run's current state, so a
-- hold's audit event is appended even after an operator has ended the hold,
-- and each hold gets its own event. A run that has been held is one an
-- operator chose to keep: an automatic path that would fail it after a
-- release (a crash, a charged retry) holds it again instead.
ALTER TABLE pipeline_runs
    ADD COLUMN IF NOT EXISTS operator_held_at TIMESTAMPTZ;

GRANT UPDATE (operator_held_at) ON pipeline_runs TO trace_ingest_runtime;

-- The unapplied index commands of other runs that a Score running under the
-- runtime controls counted (`unapplied_index_commands_on_tx`), written by the Score commit on
-- the transaction that holds the tenant's Score lock. Only a Score that
-- passed quality and failed novelty writes rows. A basis run that later
-- fails with its command never applied names a Score that may have been
-- judged a duplicate of nothing (#1313 case (a)); the rescore is deferred,
-- and these rows identify the affected runs for a later backfill.
-- `basis_run_id` has no foreign key on purpose: the record outlives a basis
-- run that is deleted with its submission.
CREATE TABLE IF NOT EXISTS pipeline_score_bases (
    tenant_id TEXT NOT NULL,
    run_id UUID NOT NULL,
    basis_run_id UUID NOT NULL,
    basis_revision_id UUID NOT NULL,
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, run_id, basis_run_id),
    CHECK (run_id <> basis_run_id),
    FOREIGN KEY (tenant_id, run_id)
        REFERENCES pipeline_runs (tenant_id, run_id)
        ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_pipeline_score_bases_basis
    ON pipeline_score_bases (tenant_id, basis_run_id);

ALTER TABLE pipeline_score_bases ENABLE ROW LEVEL SECURITY;
ALTER TABLE pipeline_score_bases FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON pipeline_score_bases;
CREATE POLICY trace_corpus_tenant_isolation ON pipeline_score_bases
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_ingest_runtime') THEN
        RAISE EXCEPTION 'V128: trace_ingest_runtime is missing; V90 creates it';
    END IF;
END $$;

-- The Score commit inserts the rows; the store reads them. Nothing updates
-- or deletes one (a run's rows leave with the run).
GRANT SELECT, INSERT ON pipeline_score_bases TO trace_ingest_runtime;
