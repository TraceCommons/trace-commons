-- Shadow-mode graded-credit quality score persisted per gate decision.
-- All nullable/backfillable: q is computed inline for new decisions and by the
-- POST /v1/admin/score-credit-quality batch route for existing rows. No RLS
-- change is needed (columns on an already-RLS-forced table inherit the tenant
-- predicate); no new grants beyond the existing trace_gate_decisions grants.
ALTER TABLE trace_gate_decisions
    ADD COLUMN IF NOT EXISTS credit_quality_micros BIGINT,
    ADD COLUMN IF NOT EXISTS credit_quality_anomaly_ratio_micros BIGINT,
    ADD COLUMN IF NOT EXISTS credit_quality_calibration_version INTEGER;
