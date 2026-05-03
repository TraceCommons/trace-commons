-- Persist per-label-source calibration error evidence for ranking promotion gates.

ALTER TABLE trace_ranking_calibration_runs
    ADD COLUMN IF NOT EXISTS max_label_source_average_absolute_error_micros BIGINT,
    ADD COLUMN IF NOT EXISTS max_error_label_source TEXT
        CHECK (
            max_error_label_source IS NULL
            OR max_error_label_source IN ('frontier_lab', 'reviewer', 'benchmark', 'system')
        );
