-- Lifecycle fields for ranking automation run accounting.

ALTER TABLE trace_ranking_worker_runs
    ADD COLUMN IF NOT EXISTS status TEXT NOT NULL DEFAULT 'completed',
    ADD COLUMN IF NOT EXISTS completed_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS last_error_hash TEXT;

UPDATE trace_ranking_worker_runs
   SET completed_at = created_at
 WHERE status = 'completed'
   AND completed_at IS NULL;
