-- Cross-trace dedup: per-decision duplicate-cluster assignment (shadow mode).
-- dedup_simhash = 64-bit token simhash (stored as BIGINT; bit pattern, may be
-- negative when interpreted as signed). dedup_cluster_id = assigned cluster.
-- dedup_cluster_size = snapshot of the cluster's cross-tenant member count;
-- dup_pen = 1 / dedup_cluster_size. All nullable/backfillable. No RLS change
-- (columns on an already-RLS-forced table inherit the tenant predicate); no new
-- grants beyond the existing trace_gate_decisions grants.
ALTER TABLE trace_gate_decisions
    ADD COLUMN IF NOT EXISTS dedup_simhash BIGINT,
    ADD COLUMN IF NOT EXISTS dedup_cluster_id UUID,
    ADD COLUMN IF NOT EXISTS dedup_cluster_size INTEGER;
