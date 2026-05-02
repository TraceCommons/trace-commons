-- Persist ranking calibration gate metadata on finalized credit settlements.

ALTER TABLE trace_credit_settlement_batches
    ADD COLUMN IF NOT EXISTS ranking_model_version TEXT,
    ADD COLUMN IF NOT EXISTS ranking_target_use TEXT,
    ADD COLUMN IF NOT EXISTS ranking_calibration_run_id UUID,
    ADD COLUMN IF NOT EXISTS ranking_calibration_report_hash TEXT,
    ADD COLUMN IF NOT EXISTS ranking_credit_events_excluded_count INTEGER NOT NULL DEFAULT 0;
