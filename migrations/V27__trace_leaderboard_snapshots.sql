-- Community surface — pre-rendered leaderboard + analytics snapshots.
--
-- The snapshot worker computes leaderboard rows + corpus aggregates on
-- a schedule, applies min-cell suppression (and noise / privacy-budget
-- guards in follow-up slices), and writes the result here as
-- canonicalised JSON. The public read endpoints serve from these rows
-- with a single query, so the read path stays fast and the
-- aggregation guards happen exactly once at compute time.
--
-- Cross-tenant by design: snapshots span every public-attribution
-- contributor in the deployment. The min-cell guard ensures any
-- per-tenant inference is bounded by the configured threshold.
-- RLS is intentionally NOT enforced on this table — operator-only
-- access is via the runtime role; public reads happen through the
-- application layer which scopes by snapshot_id.

CREATE TABLE IF NOT EXISTS trace_leaderboard_snapshots (
    snapshot_id UUID PRIMARY KEY,
    computed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    window_label TEXT NOT NULL,
    metric TEXT NOT NULL,
    contents_jsonb JSONB NOT NULL,
    contents_sha256 TEXT NOT NULL,
    min_cell_count INTEGER NOT NULL,
    noise_seed_hash TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_trace_leaderboard_snapshots_latest
    ON trace_leaderboard_snapshots (window_label, metric, computed_at DESC);
