-- Account merges carry invite trust to the surviving account.
--
-- Rule, stated once here and in docs/operator/account-invite-trust.md:
--   * Every invite grant row of the absorbed account is copied onto the
--     survivor with its original invite hash, grant trust_version, granted_at
--     and revoked_at. The absorbed account's rows stay, unreferenced, on the
--     closed account, as source sessions do (V78).
--   * A revoked grant never confers trust, and a merge never clears a
--     revocation. When both accounts hold a grant from the same invite, a
--     revocation on either side wins.
--   * The survivor's authority is then derived from its combined grants:
--     'invited' when it holds any unrevoked grant, otherwise 'bounded' when
--     either account had a trust row, otherwise no row. This is the same
--     invariant admission enforces (authority = 'invited' iff an active grant
--     exists), so the survivor keeps the higher of the two trusts unless the
--     only grant behind it is revoked.
--   * An authority change (or a new survivor row) sets the survivor's
--     trust_version past both accounts' versions, so a reservation priced
--     under the old authority is refused at processing and no client that
--     cached either account's version sees it go backwards. An unchanged
--     authority keeps the survivor's version.
--   * Redemption idempotency events stay with the absorbed account. They
--     answer replays of that account's own requests; the survivor already
--     holds the grant, so a replay under the survivor is a repeat
--     presentation, not a spend.
--
-- The login executing a merge holds no privilege on the trust tables and
-- gains none: this SECURITY DEFINER function, owned by a NOLOGIN NOBYPASSRLS
-- guard, is the only way in. It acts only for the caller's tenant context and
-- only for a proposal whose consuming UPDATE is this very transaction (the
-- xmin proof trace_reward_accounts_merge and trace_source_sessions_merge
-- use), so it cannot be invoked to move trust outside an executing merge.
-- Forced RLS still applies to the guard.
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_account_trust_merge_guard') THEN
        CREATE ROLE trace_account_trust_merge_guard NOLOGIN NOBYPASSRLS;
    END IF;
END $$;
-- Only NOLOGIN is re-asserted: changing BYPASSRLS needs a superuser, and the
-- migrating owner is not one (migration_atomicity_pg).
ALTER ROLE trace_account_trust_merge_guard NOLOGIN;
GRANT trace_account_trust_merge_guard TO CURRENT_USER;
GRANT USAGE ON SCHEMA public TO trace_account_trust_merge_guard;
-- Table-wide SELECT, as V78 grants: the xmin proof reads a system column.
GRANT SELECT ON trace_account_merge_proposals TO trace_account_trust_merge_guard;
GRANT SELECT, INSERT, UPDATE (revoked_at)
    ON trace_account_invite_grants TO trace_account_trust_merge_guard;
-- UPDATE on these three columns is also what lets the function lock the row.
GRANT SELECT, INSERT, UPDATE (authority, trust_version, updated_at)
    ON trace_account_trust TO trace_account_trust_merge_guard;

CREATE FUNCTION public.trace_account_trust_merge(
    p_tenant TEXT, p_surviving_account UUID, p_absorbed_account UUID, p_proposal UUID
) RETURNS BIGINT
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE
    v_carried BIGINT;
    v_active BOOLEAN;
    v_survivor_authority TEXT;
    v_survivor_version BIGINT;
    v_absorbed_version BIGINT;
    v_target TEXT;
    v_next BIGINT;
BEGIN
    IF p_tenant IS NULL OR p_tenant = ''
        OR p_tenant IS DISTINCT FROM public.trace_current_tenant_id()
        OR p_surviving_account IS NULL OR p_absorbed_account IS NULL
        OR p_surviving_account = p_absorbed_account OR p_proposal IS NULL THEN
        RAISE EXCEPTION USING MESSAGE = 'account_trust_merge_unauthorized';
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
        RAISE EXCEPTION USING MESSAGE = 'account_trust_merge_unauthorized';
    END IF;

    -- Lock the survivor's trust row first, as admission does before it reads
    -- grants, so a concurrent reservation sees either the pre- or post-merge
    -- authority and never a mix.
    SELECT authority, trust_version INTO v_survivor_authority, v_survivor_version
      FROM public.trace_account_trust
     WHERE tenant_id = p_tenant AND account_id = p_surviving_account
     FOR UPDATE;
    SELECT trust_version INTO v_absorbed_version
      FROM public.trace_account_trust
     WHERE tenant_id = p_tenant AND account_id = p_absorbed_account;

    INSERT INTO public.trace_account_invite_grants AS existing
        (tenant_id, account_id, invite_subject_hash, trust_version, granted_at, revoked_at)
    SELECT tenant_id, p_surviving_account, invite_subject_hash, trust_version,
           granted_at, revoked_at
      FROM public.trace_account_invite_grants
     WHERE tenant_id = p_tenant AND account_id = p_absorbed_account
    ON CONFLICT (tenant_id, account_id, invite_subject_hash) DO UPDATE
       SET revoked_at = COALESCE(existing.revoked_at, EXCLUDED.revoked_at);
    GET DIAGNOSTICS v_carried = ROW_COUNT;

    SELECT EXISTS (
        SELECT 1 FROM public.trace_account_invite_grants
         WHERE tenant_id = p_tenant AND account_id = p_surviving_account
           AND revoked_at IS NULL
    ) INTO v_active;

    IF v_active THEN
        v_target := 'invited';
    ELSIF v_survivor_version IS NOT NULL OR v_absorbed_version IS NOT NULL THEN
        v_target := 'bounded';
    ELSE
        RETURN v_carried;
    END IF;

    v_next := GREATEST(COALESCE(v_survivor_version, 0), COALESCE(v_absorbed_version, 0)) + 1;
    IF v_survivor_version IS NULL THEN
        INSERT INTO public.trace_account_trust (tenant_id, account_id, authority, trust_version)
        VALUES (p_tenant, p_surviving_account, v_target, v_next);
    ELSIF v_survivor_authority IS DISTINCT FROM v_target THEN
        UPDATE public.trace_account_trust
           SET authority = v_target, trust_version = v_next,
               updated_at = pg_catalog.clock_timestamp()
         WHERE tenant_id = p_tenant AND account_id = p_surviving_account;
    END IF;
    RETURN v_carried;
END $$;
GRANT CREATE ON SCHEMA public TO trace_account_trust_merge_guard;
ALTER FUNCTION public.trace_account_trust_merge(TEXT, UUID, UUID, UUID)
    OWNER TO trace_account_trust_merge_guard;
REVOKE CREATE ON SCHEMA public FROM trace_account_trust_merge_guard;
-- Callable by any merge-executing login, as the reward and source-session
-- hooks are: the in-transaction consumed-proposal proof is the authorization.
GRANT EXECUTE ON FUNCTION public.trace_account_trust_merge(TEXT, UUID, UUID, UUID) TO PUBLIC;
REVOKE trace_account_trust_merge_guard FROM CURRENT_USER;
