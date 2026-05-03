-- Persist label-source diversity evidence for ranking calibration promotion gates.

ALTER TABLE trace_ranking_calibration_runs
    ADD COLUMN IF NOT EXISTS joined_label_source_count INTEGER NOT NULL DEFAULT 0
        CHECK (joined_label_source_count >= 0),
    ADD COLUMN IF NOT EXISTS min_label_source_count INTEGER NOT NULL DEFAULT 1
        CHECK (min_label_source_count >= 1 AND min_label_source_count <= 4);
