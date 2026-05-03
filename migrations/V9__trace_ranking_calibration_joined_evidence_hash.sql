-- Persist a deterministic hash of the prediction/label evidence joined into
-- each ranking calibration run without exposing trace bodies or raw lab refs.

ALTER TABLE trace_ranking_calibration_runs
    ADD COLUMN IF NOT EXISTS joined_evidence_hash TEXT NOT NULL DEFAULT 'sha256:legacy'
        CHECK (joined_evidence_hash LIKE 'sha256:%');
