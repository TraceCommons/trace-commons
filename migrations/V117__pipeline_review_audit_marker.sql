-- V117: the marker of a review audit event that the worker must still append.
-- A transaction that commits a Review decision or a human review assessment
-- sets it; the worker's review audit pass clears it. NULL for every run that
-- exists now: no event is appended for a decision made before this migration.
ALTER TABLE pipeline_runs
    ADD COLUMN IF NOT EXISTS review_audit_pending_at TIMESTAMPTZ;

CREATE INDEX IF NOT EXISTS idx_pipeline_runs_review_audit_work
    ON pipeline_runs (tenant_id, review_audit_pending_at)
    WHERE review_audit_pending_at IS NOT NULL;

GRANT UPDATE (review_audit_pending_at) ON pipeline_runs TO trace_ingest_runtime;
