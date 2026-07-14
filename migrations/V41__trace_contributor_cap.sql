-- Per-contributor concave cap (credit pipeline sub-project #3). Shadow-only
-- per-decision snapshot: the marginal cap factor, the in-epoch cumulative raw
-- credit R it was computed against, the epoch bucket, and the calibration
-- version. All nullable; written only by the recompute-contributor-caps pass.
ALTER TABLE trace_gate_decisions
    ADD COLUMN IF NOT EXISTS contributor_factor_micros INTEGER,
    ADD COLUMN IF NOT EXISTS contributor_cumulative_raw_micros BIGINT,
    ADD COLUMN IF NOT EXISTS contributor_cap_epoch BIGINT,
    ADD COLUMN IF NOT EXISTS contributor_cap_version INTEGER;
