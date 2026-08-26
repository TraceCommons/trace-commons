-- Shadow-mode value of a contributor correction (S5).
--
-- A correction is scored through the machinery a trace already faces: the same
-- token simhash, the same cluster assignment, and the same concave saturating
-- curve as the credit-quality score. correction_simhash = 64-bit token simhash
-- of outcome.human_correction (stored as BIGINT; bit pattern, may be negative
-- when interpreted as signed). correction_cluster_id = its cross-tenant
-- duplicate cluster; correction_cluster_size = the snapshot member count, so
-- dup_pen = 1 / correction_cluster_size. correction_novelty_micros = lexical
-- novelty against the corrections already in the corpus, * 1e6.
-- correction_value_micros = sat(novelty) * dup_pen, * 1e6.
--
-- SHADOW ONLY. Nothing reads these columns into a settlement, a credit, or a
-- gate decision, and there will be zero non-NULL rows until the collection UI
-- ships. They exist to be calibrated against real corrections rather than
-- guessed at. All nullable/backfillable.
--
-- No RLS change (columns on an already-RLS-forced table inherit the tenant
-- predicate). Column-level SELECT grants ARE required: the gate-driver reader
-- role holds column-level grants (V45), not table-level ones, so a new column
-- it must read is unreadable until it is granted here -- the drift V47 had to
-- repair for the V37 chunk columns.
ALTER TABLE trace_gate_decisions
    ADD COLUMN IF NOT EXISTS correction_simhash BIGINT,
    ADD COLUMN IF NOT EXISTS correction_cluster_id UUID,
    ADD COLUMN IF NOT EXISTS correction_cluster_size INTEGER,
    ADD COLUMN IF NOT EXISTS correction_novelty_micros BIGINT,
    ADD COLUMN IF NOT EXISTS correction_value_micros BIGINT,
    ADD COLUMN IF NOT EXISTS correction_value_version INTEGER;

-- The cross-tenant correction-cluster scan reads these two.
GRANT SELECT (correction_simhash) ON trace_gate_decisions TO trace_gate_driver;
GRANT SELECT (correction_cluster_id) ON trace_gate_decisions TO trace_gate_driver;
