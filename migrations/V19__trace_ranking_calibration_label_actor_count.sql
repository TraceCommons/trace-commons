-- Persist distinct label-actor diversity evidence for ranking calibration runs.

ALTER TABLE trace_ranking_calibration_runs
    ADD COLUMN IF NOT EXISTS joined_label_actor_count INTEGER NOT NULL DEFAULT 0
        CHECK (joined_label_actor_count >= 0);
