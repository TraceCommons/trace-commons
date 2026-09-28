-- Account merges carry NEAR anchors and provisioned devices to the survivor.
--
-- Rule, stated once here and in docs/operator/account-invite-trust.md:
--   * Every trace_near_account_anchors row of the absorbed account is re-keyed
--     onto the survivor, and so is every trace_near_provisioned_devices row
--     provisioned under those anchors (V58's foreign key ties a device row to
--     its anchor's account, so the two move together). No other column
--     changes and no row is created or deleted.
--   * A survivor that already holds an anchor ends up holding both. Every
--     reader of the table asks whether an account has an anchor, or which
--     account an anchor belongs to; none assumes one anchor per account.
--   * The same NEAR account cannot be anchored on both sides, and the same
--     device cannot be provisioned to both: anchor_hash is globally UNIQUE and
--     (tenant_id, device_key_id) is UNIQUE. account_id is in neither key, so
--     the re-key cannot collide.
--   * Revocation lives on device_keys, which this does not touch. A revoked
--     device moves with its anchor and stays revoked; the merge never makes a
--     device live that was not live before.
--
-- Without this, the absorbed account's anchor stays on a closed account:
-- admission's live-device check and the fleet readiness check both join the
-- device row to an OPEN account, so the absorbed account's NEAR devices stop
-- being live and readiness turns false, and a returning NEAR sign-in (which
-- looks the anchor up among open accounts, then refuses because the anchor is
-- already claimed) can no longer sign in at all.
--
-- The login executing a merge holds no privilege on these tables and gains
-- none: this SECURITY DEFINER function, owned by a NOLOGIN NOBYPASSRLS guard,
-- is the only way in. It acts only for the caller's tenant context and only
-- for a proposal whose consuming UPDATE is this very transaction (the xmin
-- proof trace_reward_accounts_merge, trace_source_sessions_merge and
-- trace_account_trust_merge use). Forced RLS still applies to the guard.
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_near_account_merge_guard') THEN
        CREATE ROLE trace_near_account_merge_guard NOLOGIN NOBYPASSRLS;
    END IF;
END $$;
-- Only NOLOGIN is re-asserted: changing BYPASSRLS needs a superuser, and the
-- migrating owner is not one (migration_atomicity_pg).
ALTER ROLE trace_near_account_merge_guard NOLOGIN;
GRANT trace_near_account_merge_guard TO CURRENT_USER;
GRANT USAGE ON SCHEMA public TO trace_near_account_merge_guard;
-- Table-wide SELECT, as V78 and V80 grant: the xmin proof reads a system column.
GRANT SELECT ON trace_account_merge_proposals TO trace_near_account_merge_guard;
-- Column-scoped: the guard never reads sealed_account_name or any pepper or
-- key reference, and can rewrite nothing but the owning account.
GRANT SELECT (tenant_id, account_id, anchor_hash), UPDATE (account_id)
    ON trace_near_account_anchors TO trace_near_account_merge_guard;
GRANT SELECT (tenant_id, account_id), UPDATE (account_id)
    ON trace_near_provisioned_devices TO trace_near_account_merge_guard;

CREATE FUNCTION public.trace_near_account_merge(
    p_tenant TEXT, p_surviving_account UUID, p_absorbed_account UUID, p_proposal UUID,
    OUT anchors_carried BIGINT, OUT devices_carried BIGINT
)
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
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
           -- See trace_reward_accounts_merge: the consuming UPDATE must be
           -- this transaction proper, not a subtransaction.
           AND proposal.xmin = pg_catalog.pg_current_xact_id()::xid
    ) THEN
        RAISE EXCEPTION USING MESSAGE = 'near_account_merge_unauthorized';
    END IF;

    -- Provisioning serializes on this same per-anchor advisory lock before it
    -- resolves an anchor to an open account and adds a principal and device
    -- under it. Taking it here, in a fixed order, means a sign-in racing the
    -- merge either commits first (and its new rows are carried below and by
    -- the principal re-key after) or waits and then finds the anchor on the
    -- survivor. execute_merge calls this before it locks the absorbed
    -- account's row, so provisioning never holds this lock while waiting on
    -- one the merge holds.
    PERFORM pg_catalog.pg_advisory_xact_lock(pg_catalog.hashtextextended(anchor.anchor_hash, 0))
       FROM (SELECT anchor_hash FROM public.trace_near_account_anchors
              WHERE tenant_id = p_tenant AND account_id = p_absorbed_account
              ORDER BY anchor_hash) anchor;

    -- One statement, so V58's (tenant_id, account_id, anchor_hash) foreign key
    -- is checked once both sides have moved.
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
END $$;
GRANT CREATE ON SCHEMA public TO trace_near_account_merge_guard;
ALTER FUNCTION public.trace_near_account_merge(TEXT, UUID, UUID, UUID)
    OWNER TO trace_near_account_merge_guard;
REVOKE CREATE ON SCHEMA public FROM trace_near_account_merge_guard;
-- Callable by any merge-executing login, as the reward, source-session and
-- trust hooks are: the in-transaction consumed-proposal proof is the
-- authorization.
GRANT EXECUTE ON FUNCTION public.trace_near_account_merge(TEXT, UUID, UUID, UUID) TO PUBLIC;
REVOKE trace_near_account_merge_guard FROM CURRENT_USER;
