-- V116: gate decision rows written by the versioned pipeline's Settle phase
-- (spec 2026-10-08 pipeline production assembly, Slice C, C-D4).
--
-- The pipeline writes one trace_gate_decisions row per submission from its
-- Settle commit, so duplicate clustering, the contributor cap, credit
-- quality, per-author scoring and account trust see pipeline traffic with no
-- change to their readers. These two columns tell the writers apart. Both
-- default, so main's writers (insert_trace_gate_decision[_with_chunk_entries])
-- are unchanged and every existing row reads as `legacy_gate`.
ALTER TABLE trace_gate_decisions
    ADD COLUMN IF NOT EXISTS source TEXT NOT NULL DEFAULT 'legacy_gate'
        CHECK (source IN ('legacy_gate', 'pipeline_settle')),
    ADD COLUMN IF NOT EXISTS pipeline_run_id UUID NULL;

-- A pipeline row names the run that wrote it; a legacy row names none.
ALTER TABLE trace_gate_decisions
    DROP CONSTRAINT IF EXISTS trace_gate_decisions_pipeline_run_matches_source;
ALTER TABLE trace_gate_decisions
    ADD CONSTRAINT trace_gate_decisions_pipeline_run_matches_source
        CHECK ((source = 'pipeline_settle') = (pipeline_run_id IS NOT NULL));

-- At most one pipeline row per submission. main may still write several
-- rows for one submission (the Cached outcome); this binds only the
-- pipeline's own.
CREATE UNIQUE INDEX IF NOT EXISTS trace_gate_decisions_one_pipeline_row
    ON trace_gate_decisions (tenant_id, submission_id)
    WHERE source = 'pipeline_settle';

-- The credit-quality sweep (list_gate_decisions_for_credit_scoring) leaves
-- pipeline rows out: the pipeline's Score already computed their credit
-- quality under the bundle's pinned calibration, and the sweep would
-- overwrite it. trace_gate_driver holds column-scoped grants (V45), so the
-- column the filter references is granted here.
GRANT SELECT (source) ON trace_gate_decisions TO trace_gate_driver;
