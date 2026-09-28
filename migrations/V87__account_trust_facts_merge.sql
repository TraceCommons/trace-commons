-- Earned account trust, M4 (docs/superpowers/specs/2026-09-26-earned-account-trust-design.md):
-- facts follow a merge.
--
-- trace_account_trust_merge (V80) is replaced in place: same signature, same
-- owner (trace_account_trust_merge_guard, NOLOGIN NOBYPASSRLS), same
-- in-transaction consumed-proposal proof. Its body is V80's unchanged, plus
-- one statement: every trace_account_trust_facts row of the absorbed account
-- is copied onto the survivor, keyed by the fact's primary key
-- (tenant_id, account_id, source_kind, source_id) with ON CONFLICT DO
-- NOTHING, so a fact both accounts hold is never duplicated. The copy keeps
-- occurred_at and outcome, so the rule reads the same history. The absorbed
-- account's rows stay, unreferenced, on the closed account, as its invite
-- grants and source sessions do.
--
-- The copy runs before V80's early return (neither account has a trust row),
-- because an account can have facts without ever having reserved.
--
-- Evaluations are not copied. The survivor is re-evaluated by the next shadow
-- run, over the union of facts, so the weekly cap and active_weeks are
-- computed once over the union and two merged accounts do not count a week
-- twice.
--
-- The merge-executing login still holds no privilege on the facts table; the
-- guard gains SELECT and INSERT on it.
GRANT trace_account_trust_merge_guard TO CURRENT_USER;
GRANT SELECT, INSERT ON trace_account_trust_facts TO trace_account_trust_merge_guard;

CREATE OR REPLACE FUNCTION public.trace_account_trust_merge(
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

    -- V87: earned-trust facts follow the merge, each source once.
    INSERT INTO public.trace_account_trust_facts
        (tenant_id, account_id, source_kind, source_id, submission_id, outcome,
         evaluator_version, recorded_at, occurred_at)
    SELECT tenant_id, p_surviving_account, source_kind, source_id, submission_id, outcome,
           evaluator_version, recorded_at, occurred_at
      FROM public.trace_account_trust_facts
     WHERE tenant_id = p_tenant AND account_id = p_absorbed_account
    ON CONFLICT DO NOTHING;

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
REVOKE trace_account_trust_merge_guard FROM CURRENT_USER;
