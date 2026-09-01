-- Row-level security for community withdrawal eviction receipts.
--
-- V46 shipped `trace_community_withdrawal_evictions` with no RLS at all --
-- `relrowsecurity = false`, zero policies -- while every row carries
-- `tenant_id`, `principal_ref`, `display_handle` and `handle_normalized`.
-- Any connection with the runtime role could read every tenant's withdrawn
-- contributor handles. V46's header called the table cross-tenant "by design",
-- but that rationale belongs to its two sibling tables (see the exclusion note
-- at the bottom of this file), not to the one row that names contributors.
-- The gap survived because the table was also absent from
-- `TRACE_COMMONS_RLS_TABLES`, so `production_ready()` never considered it.
--
-- Ordinary access is tenant-scoped. The one path that is not is the drain:
-- `drain_community_snapshot_invalidation` takes no tenant id and deliberately
-- marks every pending eviction for a (window_label, metric) in one statement.
-- It is gated by a transaction-local GUC rather than a tenant predicate,
-- mirroring V35's `trace_current_instance_subject()` and V42's
-- `trace_current_invite_subject()`.

ALTER TABLE trace_community_withdrawal_evictions ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_community_withdrawal_evictions FORCE ROW LEVEL SECURITY;

DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_community_withdrawal_evictions;
CREATE POLICY trace_corpus_tenant_isolation ON trace_community_withdrawal_evictions
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

-- Drain-path predicate. The drain worker sets this GUC transaction-locally for
-- the one transaction that marks evictions drained; every other connection
-- leaves it unset and stays confined to the tenant policy above. A GUC rather
-- than a role because the drain runs on the shared runtime pool, not on a
-- narrow second pool like `trace_gate_driver` (V36) or `trace_invite_registry`
-- (V42).
CREATE OR REPLACE FUNCTION trace_community_drain_scope()
RETURNS BOOLEAN
LANGUAGE SQL
STABLE
AS $$
    SELECT COALESCE(current_setting('trace_commons.community_drain', true), '') = 'on';
$$;

-- The drain's WHERE clause reads columns, so PostgreSQL requires SELECT rights
-- and applies SELECT policies in addition to the UPDATE policy. Without this
-- the drain does not error -- it silently updates zero rows, because the
-- tenant SELECT predicate hides every row from a connection with no tenant
-- context. Measured on a live server: the identical statement reports
-- `UPDATE 0` with the UPDATE policy alone and `UPDATE 2` with both.
--
-- This policy cannot be narrowed to `drained_at IS NULL`. When an UPDATE
-- requires SELECT rights, the *new* row is re-checked against the SELECT
-- policies, and the new row is by definition drained -- so the narrower form
-- makes the drain fail with "new row violates row-level security policy".
-- Also measured. Column-level narrowing is not expressible either: that is a
-- GRANT, which is role-scoped, and the drain shares the runtime role.
DROP POLICY IF EXISTS trace_community_drain_read ON trace_community_withdrawal_evictions;
CREATE POLICY trace_community_drain_read ON trace_community_withdrawal_evictions
    FOR SELECT
    USING (trace_community_drain_scope());

-- The write allowance is as narrow as RLS permits: UPDATE only (no INSERT, no
-- DELETE), only rows not yet drained, and only into a drained state. Drain
-- scope therefore cannot create a receipt, delete one, or un-drain one.
DROP POLICY IF EXISTS trace_community_drain_mark ON trace_community_withdrawal_evictions;
CREATE POLICY trace_community_drain_mark ON trace_community_withdrawal_evictions
    FOR UPDATE
    USING (trace_community_drain_scope() AND drained_at IS NULL)
    WITH CHECK (trace_community_drain_scope() AND drained_at IS NOT NULL);

-- Deliberately excluded from RLS, recorded here so the next audit does not have
-- to re-derive it:
--
-- `trace_community_snapshot_invalidations` (V46) -- one row per
-- (window_label, metric) holding a pending flag, a count and drain
-- watermarks. No tenant_id, no principal, no handle: a deployment-wide
-- rebuild watermark that coalesces N tenants' withdrawals into one rebuild.
-- A tenant predicate has nothing to compare against.
--
-- `trace_leaderboard_snapshots` (V27) -- the published community leaderboard,
-- one row per (window, metric, computed_at) merging every tenant in
-- `community_tenant_ids` into a single deployment-wide ranking. A tenant
-- predicate is not merely unnecessary here, it is inexpressible: there is no
-- tenant_id column and no per-row tenant to compare against.
--
-- This is NOT a "carries no identity" exclusion, and the distinction matters
-- enough to spell out so the next reader can check it rather than trust it.
-- `contents_jsonb` holds a serialized `CommunitySnapshotContents` whose
-- `leaderboard` entries and `contributors` map both carry `display_handle`
-- (and `bio`), copied verbatim from `trace_contributor_profiles` by
-- `compute_leaderboard_inputs`. It is identity-bearing -- but as opt-in
-- published data:
--
--   * writes reach it only behind the `public_attribution` consent scope,
--     enforced by `enforce_public_attribution_scope` in
--     `bin/trace-commons-ingest.rs`; and
--   * the same bytes are served verbatim to the UNAUTHENTICATED
--     `GET /v1/community/*` endpoints.
--
-- Withdrawal removes a contributor from the next snapshot via the eviction
-- receipts above, and the already-published snapshot is not left exposed in
-- the meantime: the read path refuses it with
-- `snapshot_invalidated_by_withdrawal` (`community_snapshot_freshness_failure`)
-- for as long as the pending watermark postdates `computed_at`.
