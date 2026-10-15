-- V123 (#1346): the time a run's failure moved its submission to `rejected`.
--
-- A run that ends `failed` while its submission is still `received` (Review
-- never decided it) moves the submission to `rejected`, status reason
-- `pipeline_processing_failed`, in the same transaction, and records the
-- move here. The worker's review audit pass reads this column to append
-- `main`'s `lifecycle_status_change` event for the move: the submission row
-- cannot say it, because a later revocation rewrites its status and reason.
-- Only a failed run carries it. NULL for every run that exists now: no
-- submission was moved before this migration.
ALTER TABLE pipeline_runs
    ADD COLUMN IF NOT EXISTS submission_rejected_at TIMESTAMPTZ;

ALTER TABLE pipeline_runs
    DROP CONSTRAINT IF EXISTS pipeline_runs_submission_rejected_shape;
ALTER TABLE pipeline_runs
    ADD CONSTRAINT pipeline_runs_submission_rejected_shape
    CHECK (submission_rejected_at IS NULL OR state = 'failed');

GRANT UPDATE (submission_rejected_at) ON pipeline_runs TO trace_ingest_runtime;
