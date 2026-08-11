-- Community withdrawal → published-snapshot eviction (#190).
--
-- Withdrawal stamps withdrawn_at on the profile row, but the public
-- community surface serves a materialised snapshot. Without an
-- invalidation watermark the withdrawn handle remains visible until an
-- operator happens to recompute — breaking the documented ≤15-minute
-- removal bound.
--
-- These tables are cross-tenant by design (same rationale as
-- trace_leaderboard_snapshots): one deployment-wide pending flag
-- coalesces N withdrawals into one rebuild, and eviction receipts are
-- audited through the application layer rather than per-tenant RLS.

CREATE TABLE IF NOT EXISTS trace_community_snapshot_invalidations (
    window_label TEXT NOT NULL,
    metric TEXT NOT NULL,
    -- NULL means no undrained withdrawal since the last successful drain.
    pending_requested_at TIMESTAMPTZ,
    pending_withdrawal_count INTEGER NOT NULL DEFAULT 0
        CHECK (pending_withdrawal_count >= 0),
    last_drained_at TIMESTAMPTZ,
    last_drained_snapshot_id UUID,
    PRIMARY KEY (window_label, metric)
);

CREATE TABLE IF NOT EXISTS trace_community_withdrawal_evictions (
    eviction_id UUID PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    principal_ref TEXT NOT NULL,
    display_handle TEXT,
    handle_normalized TEXT,
    withdrawn_at TIMESTAMPTZ NOT NULL,
    invalidation_requested_at TIMESTAMPTZ NOT NULL,
    window_label TEXT NOT NULL,
    metric TEXT NOT NULL,
    drained_at TIMESTAMPTZ,
    drained_snapshot_id UUID,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_community_withdrawal_evictions_pending_drain
    ON trace_community_withdrawal_evictions (window_label, metric, created_at)
    WHERE drained_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_community_withdrawal_evictions_tenant_principal
    ON trace_community_withdrawal_evictions (tenant_id, principal_ref, created_at DESC);
