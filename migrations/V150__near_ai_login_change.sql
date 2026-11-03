-- V150: change the NEAR AI login an account uses (F10).
--
-- Spec: docs/superpowers/specs/2026-11-03-f10-change-account-design.md,
-- section 4.1. Owner decisions of 2026-11-03: one live identity per
-- account (decision 3), a seven-day cooldown between changes (decision 9).
--
-- WHAT IT ADDS
--   * trace_near_account_anchors.retired_at. A change retires the old anchor
--     and never deletes it: the retired row keeps UNIQUE (anchor_hash), so
--     the identity it indexes cannot claim a fresh account (T5). Switching
--     back clears it.
--   * At most one live anchor per account, as a partial unique index.
--   * trace_account_login_changes: one row per change, with no anchor,
--     subject, provider or wallet name. It exists for the cooldown.
--   * trace_admission_anchor_busy(): whether an evidence-ledger admission
--     reservation is in progress for an anchor. The runtime cannot read the
--     ledger; a change must not retire an anchor under a live lease.
--   * trace_near_account_merge() replaced (V82): an absorbed live anchor
--     lands RETIRED on a survivor that already holds a live one, and the
--     survivor's devices point at its live anchor.
--
-- THE PRE-CHECK. Before V150 every anchor is live, so an account holding
-- two (a V82 merge left both) would violate the new index. This migration
-- does not choose which identity such an account keeps: it fails, naming
-- only the count, and an operator resolves it first. Pilot expected 0
-- (2026-11-03). FORCE is lifted for the read (forced RLS applies to the
-- table owner, so a non-superuser migrator would otherwise count zero rows
-- in every tenant, V144) and put straight back.
--
-- GRANTS. Table-level grants made before V150 cover the new column. The
-- new table and function are granted to trace_ingest_runtime and, as V149
-- does, to CURRENT_USER, so an ingest login that is also the migrator
-- (staging) can call the function too.
--
-- Idempotent where it can be, and appliable by a non-superuser CREATEROLE
-- migrator.

SET LOCAL lock_timeout = '10s';

ALTER TABLE trace_near_account_anchors
    ADD COLUMN IF NOT EXISTS retired_at TIMESTAMPTZ;

ALTER TABLE trace_near_account_anchors NO FORCE ROW LEVEL SECURITY;
DO $$
DECLARE
    doubled BIGINT;
BEGIN
    SELECT count(*) INTO doubled FROM (
        SELECT 1 FROM trace_near_account_anchors
         WHERE retired_at IS NULL
         GROUP BY tenant_id, account_id
        HAVING count(*) > 1) d;
    IF doubled > 0 THEN
        RAISE EXCEPTION 'V150: % account(s) hold more than one live anchor; resolve them before this migration runs', doubled
            USING ERRCODE = 'check_violation';
    END IF;
END $$;
ALTER TABLE trace_near_account_anchors FORCE ROW LEVEL SECURITY;

CREATE UNIQUE INDEX IF NOT EXISTS trace_near_account_anchors_one_live
    ON trace_near_account_anchors (tenant_id, account_id)
    WHERE retired_at IS NULL;

CREATE TABLE IF NOT EXISTS trace_account_login_changes (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id),
    account_id UUID NOT NULL,
    change_id UUID NOT NULL,
    changed_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    from_source TEXT NOT NULL CHECK (from_source IN ('wallet', 'near_ai_login')),
    switched_back BOOLEAN NOT NULL,
    PRIMARY KEY (tenant_id, change_id),
    FOREIGN KEY (tenant_id, account_id) REFERENCES trace_accounts(tenant_id, account_id)
);
CREATE INDEX IF NOT EXISTS trace_account_login_changes_account
    ON trace_account_login_changes (tenant_id, account_id, changed_at DESC);
ALTER TABLE trace_account_login_changes ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_account_login_changes FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_account_login_changes;
CREATE POLICY trace_corpus_tenant_isolation ON trace_account_login_changes
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());
REVOKE ALL ON trace_account_login_changes FROM PUBLIC;
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_ingest_runtime') THEN
        GRANT SELECT, INSERT ON trace_account_login_changes TO trace_ingest_runtime;
    END IF;
END $$;

-- The evidence-ledger lease check. Raises, rather than answering "not
-- busy", for a tenant other than the caller's, so a mistake fails closed.
GRANT trace_admission_guard TO CURRENT_USER;
CREATE OR REPLACE FUNCTION public.trace_admission_anchor_busy(p_tenant TEXT, p_anchor TEXT)
RETURNS BOOLEAN
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path = pg_catalog AS $$
BEGIN
    IF p_tenant IS NULL OR p_tenant IS DISTINCT FROM public.trace_current_tenant_id()
        OR p_anchor IS NULL OR p_anchor !~ '^[0-9a-f]{64}$' THEN
        RAISE EXCEPTION USING MESSAGE = 'admission_anchor_busy_unauthorized';
    END IF;
    RETURN EXISTS (
        SELECT 1 FROM public.trace_admission_submissions s
         WHERE s.tenant_id = p_tenant AND s.anchor_hash = p_anchor
           AND s.status IN ('reserved', 'processing')
           AND s.lease_expires_at > pg_catalog.clock_timestamp());
END $$;
GRANT CREATE ON SCHEMA public TO trace_admission_guard;
ALTER FUNCTION public.trace_admission_anchor_busy(TEXT, TEXT) OWNER TO trace_admission_guard;
REVOKE CREATE ON SCHEMA public FROM trace_admission_guard;
REVOKE ALL ON FUNCTION public.trace_admission_anchor_busy(TEXT, TEXT) FROM PUBLIC;
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_ingest_runtime') THEN
        GRANT EXECUTE ON FUNCTION public.trace_admission_anchor_busy(TEXT, TEXT)
            TO trace_ingest_runtime;
    END IF;
END $$;
GRANT EXECUTE ON FUNCTION public.trace_admission_anchor_busy(TEXT, TEXT) TO CURRENT_USER;
REVOKE trace_admission_guard FROM CURRENT_USER;

-- V82's merge, under one live anchor per account. Replaced as its own guard
-- role (membership borrowed and handed back, as V82 did), never as the
-- migration role, so ownership and the column grants stay the guard's.
GRANT trace_near_account_merge_guard TO CURRENT_USER;
GRANT SELECT (retired_at), UPDATE (retired_at)
    ON trace_near_account_anchors TO trace_near_account_merge_guard;
GRANT SELECT (anchor_hash), UPDATE (anchor_hash)
    ON trace_near_provisioned_devices TO trace_near_account_merge_guard;
CREATE OR REPLACE FUNCTION public.trace_near_account_merge(
    p_tenant TEXT, p_surviving_account UUID, p_absorbed_account UUID, p_proposal UUID,
    OUT anchors_carried BIGINT, OUT devices_carried BIGINT
)
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE
    survivor_live TEXT;
BEGIN
    IF p_tenant IS NULL OR p_tenant = ''
        OR p_tenant IS DISTINCT FROM public.trace_current_tenant_id()
        OR p_surviving_account IS NULL OR p_absorbed_account IS NULL
        OR p_surviving_account = p_absorbed_account OR p_proposal IS NULL THEN
        RAISE EXCEPTION USING MESSAGE = 'near_account_merge_unauthorized';
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM public.trace_account_merge_proposals proposal
         WHERE proposal.tenant_id = p_tenant
           AND proposal.proposal_id = p_proposal
           AND proposal.surviving_account_id = p_surviving_account
           AND proposal.absorbed_account_id = p_absorbed_account
           AND proposal.consumed_at IS NOT NULL
           AND proposal.xmin = pg_catalog.pg_current_xact_id()::xid
    ) THEN
        RAISE EXCEPTION USING MESSAGE = 'near_account_merge_unauthorized';
    END IF;

    -- As V82: the provisioning lock on every anchor that moves, in order.
    PERFORM pg_catalog.pg_advisory_xact_lock(pg_catalog.hashtextextended(anchor.anchor_hash, 0))
       FROM (SELECT anchor_hash FROM public.trace_near_account_anchors
              WHERE tenant_id = p_tenant AND account_id = p_absorbed_account
              ORDER BY anchor_hash) anchor;

    -- One live anchor per account (V150): when the survivor already holds a
    -- live anchor, the absorbed one arrives retired. The survivor keeps its
    -- own identity; the absorbed identity can no longer sign in, and its row
    -- still blocks a fresh sign-up with it.
    SELECT anchor_hash INTO survivor_live
      FROM public.trace_near_account_anchors
     WHERE tenant_id = p_tenant AND account_id = p_surviving_account
       AND retired_at IS NULL;
    IF survivor_live IS NOT NULL THEN
        UPDATE public.trace_near_account_anchors
           SET retired_at = pg_catalog.clock_timestamp()
         WHERE tenant_id = p_tenant AND account_id = p_absorbed_account
           AND retired_at IS NULL;
    END IF;

    WITH anchors AS (
        UPDATE public.trace_near_account_anchors
           SET account_id = p_surviving_account
         WHERE tenant_id = p_tenant AND account_id = p_absorbed_account
        RETURNING 1
    ), devices AS (
        UPDATE public.trace_near_provisioned_devices
           SET account_id = p_surviving_account
         WHERE tenant_id = p_tenant AND account_id = p_absorbed_account
        RETURNING 1
    )
    SELECT (SELECT count(*) FROM anchors), (SELECT count(*) FROM devices)
      INTO anchors_carried, devices_carried;

    -- Admission reads a device's anchor: every device of the survivor points
    -- at its one live anchor, so a retired identity grants nothing.
    IF survivor_live IS NOT NULL THEN
        UPDATE public.trace_near_provisioned_devices
           SET anchor_hash = survivor_live
         WHERE tenant_id = p_tenant AND account_id = p_surviving_account
           AND anchor_hash <> survivor_live;
    END IF;
END $$;
REVOKE trace_near_account_merge_guard FROM CURRENT_USER;
