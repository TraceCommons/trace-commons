-- The passkey-account reaper (Z2, slice S5;
-- docs/superpowers/specs/2026-09-28-native-passkey-identity-design.md).
--
-- Cancel leaves an unbound passkey account inert, and S3's refuse branch
-- leaves a closed one. This function is what eventually reclaims both. Bound
-- and legacy accounts (no binding row) are never candidates.
--
-- TWO KINDS, ONE CLOCK EACH (decided 2026-09-29):
--   * `unbound`: reaped once the binding's created_at is older than
--     p_unbound_ttl_seconds (default 7 days), whether or not the account
--     signed in again. Reaping keys on bound status, not on use: an earlier
--     draft moved an account that signed in again to a 30-day idle window,
--     which let one sign-in hold an unbound account (and a slot under the
--     unbound ceiling) for a month. That rule is gone.
--   * `closed`: reaped once trace_accounts.closed_at is older than
--     p_closed_ttl_seconds (default 30 days). S3's refuse branch
--     (near_ai_bind_close_refused, V100) sets the binding to `closed` and
--     closed_at = now() in one transaction, so closed_at is the close time.
--     The binding row has no close timestamp of its own, and needs none.
--     Closed rows are already outside the unbound ceiling's count (V98), so
--     without this they accumulated without bound.
-- Either kind is reaped only when it holds no LIVE SESSION (unrevoked and
-- unexpired), so a session is never reaped from under its holder.
--
-- Numbering: S1 is V97, S2 V98 and S3 V100. This was drafted as V99 and
-- became V101 when it was rebased onto a main that already held V100, since
-- migrations must be applied in strictly increasing order. It depends only on
-- V30 (accounts, sessions), V32 (credentials), V97 (bindings) and the tables
-- the cascade reaches, so it may be renumbered freely at merge.
--
-- CROSS-TENANT, SO A DEFINER FUNCTION. The sweep spans tenants, and the
-- runtime pool is tenant-scoped under forced RLS, so nothing here grants the
-- runtime anything. The function is owned by a NOLOGIN NOBYPASSRLS guard that
-- holds column-scoped privileges and role-scoped permissive policies, as
-- V85's enumeration guard does; a separate NOLOGIN worker role holds EXECUTE
-- and nothing else. The deployer grants the worker to the login that runs
-- the reaper.
--
-- WHAT GOES (decided 2026-09-29): THE ACCOUNT, NEVER THE TENANT. A
-- candidate is deleted by deleting its trace_accounts row, and nothing else is
-- deleted directly. The tenant row is left in place, possibly empty, and so is
-- every tenant-keyed row (trace_account_audit among them). An earlier draft
-- also deleted the tenant, which cascades into about fifty tables; to make
-- that safe it ran a catalog-driven scope check that needed a grant and a
-- policy for the guard on every tenant-keyed table, and so put an obligation
-- on every later migration. Not deleting the tenant removes the check, its
-- grants and policies, and the obligation. A LATER MIGRATION OWES THE REAPER
-- NOTHING.
--
-- The account delete takes whatever cascades from trace_accounts. For a
-- passkey account that is its binding, credentials, sessions and login links.
-- The same ON DELETE CASCADE also reaches trace_account_principals,
-- trace_near_identities, trace_account_merge_proposals, trace_public_runs,
-- trace_source_sessions and trace_account_inference_connections. The unbound
-- gate keeps an unbound or closed account's session away from every route
-- that writes those, so a candidate is not expected to hold any, but if one
-- does, the cascade deletes it. unbound_account_reaper_pg pins that exact set
-- of cascading tables, so a change to it is a visible test edit, not a
-- silent widening of what a reap deletes.
--
-- Any account-keyed row whose foreign key does NOT cascade (ON DELETE
-- RESTRICT or NO ACTION -- trace_reward_principal_accounts, the account trust
-- and admission tables, the legacy invite link tables, trace_near_account_
-- anchors, and any added later) refuses the delete with 23503. The candidate
-- is rolled back and counted skipped, nothing of it is deleted, and the
-- batch moves on.
--
-- A second account in the same tenant does not matter. The delete is keyed
-- to (tenant_id, account_id) and cascades only from that account row, so it
-- cannot touch another account; refusing on one existed only to protect the
-- tenant delete, which is gone.
--
-- AUDIT. trace_audit_events and trace_account_audit are keyed to the tenant,
-- not the account, so both are retained with the tenant row. The reaper
-- writes no audit row of its own; it reports counts only, through its return
-- value.
--
-- CONCURRENCY. Each candidate is handled in its own sub-transaction:
--   1. the binding row is locked FOR UPDATE SKIP LOCKED with its state
--      re-checked in the same statement. A concurrent bind (S3) that has
--      updated or is updating the row either commits first (the state no
--      longer matches, so the row is not returned) or holds the lock (the row
--      is skipped).
--   2. the account row is locked FOR UPDATE SKIP LOCKED. Inserting any row
--      keyed to the account takes a key-share lock on it through the foreign
--      key, so a concurrent sign-in makes the candidate skipped; one that
--      commits earlier is seen by 3.
--   3. live sessions are re-checked in a fresh statement, under both locks.
-- Nothing waits on a lock the reaper cannot take, but the cascade itself can
-- wait on a child row (a session being updated) held by a transaction that
-- then waits on the reaper's account lock. That is a deadlock; the
-- detector aborts one side, and when it is the reaper the candidate is
-- rolled back and counted skipped, like a foreign-key refusal (23503) or a
-- lock timeout (lock_timeout is 3s).
--
-- Batches are bounded twice: at most p_limit accounts are deleted, and at
-- most 10 * p_limit candidates are examined. SKIPPED CANDIDATES DO NOT STALL
-- THE BATCH: candidates come oldest-first, and the scan steps past a skipped
-- one and keeps going until p_limit deletions or the scan cap. A permanently
-- refused backlog larger than the cap is what the `skipped` counter exists to
-- surface (see the operator runbook).
--
-- Returns (reaped_unbound, reaped_closed, skipped): counts only, no
-- identifier.
--
-- Idempotent where it can be, and applied by a non-superuser CREATEROLE
-- migrator, following V85: the migrator holds the owning guard just long
-- enough to own the function and to grant EXECUTE, then drops it. Every
-- earlier unreleased signature of the function is dropped first so no stale
-- overload remains.

DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_unbound_account_reaper_guard') THEN
        CREATE ROLE trace_unbound_account_reaper_guard NOLOGIN NOBYPASSRLS;
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_unbound_account_reaper') THEN
        CREATE ROLE trace_unbound_account_reaper NOLOGIN NOBYPASSRLS;
    END IF;
END $$;
ALTER ROLE trace_unbound_account_reaper_guard NOLOGIN;
ALTER ROLE trace_unbound_account_reaper NOLOGIN;
GRANT USAGE ON SCHEMA public TO trace_unbound_account_reaper;

GRANT trace_unbound_account_reaper_guard TO CURRENT_USER;
GRANT USAGE ON SCHEMA public TO trace_unbound_account_reaper_guard;

-- Column-scoped grants. UPDATE on created_at exists only because FOR UPDATE
-- needs some UPDATE privilege; the function never writes it.
GRANT SELECT (tenant_id, account_id, state, created_at), DELETE
    ON trace_account_bindings TO trace_unbound_account_reaper_guard;
GRANT UPDATE (created_at) ON trace_account_bindings TO trace_unbound_account_reaper_guard;
GRANT SELECT (tenant_id, account_id, created_at, closed_at), DELETE
    ON trace_accounts TO trace_unbound_account_reaper_guard;
GRANT UPDATE (created_at) ON trace_accounts TO trace_unbound_account_reaper_guard;
GRANT SELECT (tenant_id, account_id, last_seen_at, expires_at, revoked_at)
    ON trace_sessions TO trace_unbound_account_reaper_guard;

-- Role-scoped permissive policies; the shared tenant-isolation policies stay
-- for everyone else.
DROP POLICY IF EXISTS trace_unbound_reaper_read ON trace_account_bindings;
CREATE POLICY trace_unbound_reaper_read ON trace_account_bindings
    FOR SELECT TO trace_unbound_account_reaper_guard USING (TRUE);
DROP POLICY IF EXISTS trace_unbound_reaper_lock ON trace_account_bindings;
CREATE POLICY trace_unbound_reaper_lock ON trace_account_bindings
    FOR UPDATE TO trace_unbound_account_reaper_guard USING (TRUE) WITH CHECK (TRUE);
DROP POLICY IF EXISTS trace_unbound_reaper_delete ON trace_account_bindings;
CREATE POLICY trace_unbound_reaper_delete ON trace_account_bindings
    FOR DELETE TO trace_unbound_account_reaper_guard USING (state IN ('unbound', 'closed'));

DROP POLICY IF EXISTS trace_unbound_reaper_read ON trace_accounts;
CREATE POLICY trace_unbound_reaper_read ON trace_accounts
    FOR SELECT TO trace_unbound_account_reaper_guard USING (TRUE);
DROP POLICY IF EXISTS trace_unbound_reaper_lock ON trace_accounts;
CREATE POLICY trace_unbound_reaper_lock ON trace_accounts
    FOR UPDATE TO trace_unbound_account_reaper_guard USING (TRUE) WITH CHECK (TRUE);
DROP POLICY IF EXISTS trace_unbound_reaper_delete ON trace_accounts;
CREATE POLICY trace_unbound_reaper_delete ON trace_accounts
    FOR DELETE TO trace_unbound_account_reaper_guard USING (TRUE);

DROP POLICY IF EXISTS trace_unbound_reaper_read ON trace_sessions;
CREATE POLICY trace_unbound_reaper_read ON trace_sessions
    FOR SELECT TO trace_unbound_account_reaper_guard USING (TRUE);
-- An earlier draft read trace_submissions under this name.
DROP POLICY IF EXISTS trace_unbound_reaper_read ON trace_submissions;

-- Converge a database that ran an earlier draft of this migration (CI and
-- scratch databases only; the draft, numbered V99, was never released).
-- That draft deleted the tenant, so it gave the guard SELECT/DELETE/UPDATE
-- on trace_tenants with three policies there, and a SELECT grant plus a trace_unbound_reaper_scope
-- policy on every tenant-keyed table. None of it is used now. Each step is a
-- no-op on a fresh database.
DROP POLICY IF EXISTS trace_unbound_reaper_read ON trace_tenants;
DROP POLICY IF EXISTS trace_unbound_reaper_lock ON trace_tenants;
DROP POLICY IF EXISTS trace_unbound_reaper_delete ON trace_tenants;
DO $$
DECLARE
    r RECORD;
BEGIN
    FOR r IN
        SELECT p.polrelid::regclass AS rel
          FROM pg_policy p
         WHERE p.polname = 'trace_unbound_reaper_scope'
    LOOP
        EXECUTE format('DROP POLICY IF EXISTS trace_unbound_reaper_scope ON %s', r.rel);
    END LOOP;
    -- Every table the guard holds any privilege on, other than the three it
    -- needs. A table-level REVOKE ALL also revokes its column privileges.
    FOR r IN
        SELECT DISTINCT format('%I.%I', cp.table_schema, cp.table_name) AS rel
          FROM information_schema.column_privileges cp
         WHERE cp.grantee = 'trace_unbound_account_reaper_guard'
           AND cp.table_schema = 'public'
           AND cp.table_name NOT IN ('trace_account_bindings', 'trace_accounts', 'trace_sessions')
        UNION
        SELECT DISTINCT format('%I.%I', tp.table_schema, tp.table_name)
          FROM information_schema.table_privileges tp
         WHERE tp.grantee = 'trace_unbound_account_reaper_guard'
           AND tp.table_schema = 'public'
           AND tp.table_name NOT IN ('trace_account_bindings', 'trace_accounts', 'trace_sessions')
    LOOP
        EXECUTE format('REVOKE ALL ON %s FROM trace_unbound_account_reaper_guard', r.rel);
    END LOOP;
END $$;

DROP FUNCTION IF EXISTS trace_reap_unbound_accounts(BIGINT, INTEGER);
DROP FUNCTION IF EXISTS trace_reap_unbound_accounts(BIGINT, BIGINT, INTEGER);

CREATE FUNCTION trace_reap_unbound_accounts(
    p_unbound_ttl_seconds BIGINT, p_closed_ttl_seconds BIGINT, p_limit INTEGER)
RETURNS TABLE(reaped_unbound BIGINT, reaped_closed BIGINT, skipped BIGINT)
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE
    v_now            TIMESTAMPTZ := clock_timestamp();
    v_unbound_cutoff TIMESTAMPTZ;
    v_closed_cutoff  TIMESTAMPTZ;
    v_reaped_unbound BIGINT := 0;
    v_reaped_closed  BIGINT := 0;
    v_skipped        BIGINT := 0;
    c                RECORD;
BEGIN
    -- A one-day floor is defense in depth against a misconfigured TTL of a
    -- few seconds deleting every account.
    IF p_unbound_ttl_seconds IS NULL OR p_unbound_ttl_seconds < 86400
       OR p_closed_ttl_seconds IS NULL OR p_closed_ttl_seconds < 86400
       OR p_limit IS NULL OR p_limit < 1 OR p_limit > 1000 THEN
        RAISE EXCEPTION 'unbound_reaper_refused';
    END IF;
    PERFORM set_config('lock_timeout', '3s', true);
    v_unbound_cutoff := v_now - (p_unbound_ttl_seconds * interval '1 second');
    v_closed_cutoff := v_now - (p_closed_ttl_seconds * interval '1 second');

    FOR c IN
        SELECT k.tenant_id, k.account_id, k.state
          FROM (
            SELECT b.tenant_id, b.account_id, b.state, b.created_at AS since
              FROM public.trace_account_bindings b
             WHERE b.state = 'unbound' AND b.created_at < v_unbound_cutoff
            UNION ALL
            SELECT b.tenant_id, b.account_id, b.state, a.closed_at AS since
              FROM public.trace_account_bindings b
              JOIN public.trace_accounts a
                ON a.tenant_id = b.tenant_id AND a.account_id = b.account_id
             WHERE b.state = 'closed' AND a.closed_at < v_closed_cutoff
          ) k
         WHERE NOT EXISTS (
                SELECT 1 FROM public.trace_sessions s
                 WHERE s.tenant_id = k.tenant_id AND s.account_id = k.account_id
                   AND s.revoked_at IS NULL AND s.expires_at > v_now)
         ORDER BY k.since, k.tenant_id, k.account_id
         LIMIT p_limit::BIGINT * 10
    LOOP
        -- Stop at p_limit deletions. Skipped candidates do not count toward it.
        EXIT WHEN v_reaped_unbound + v_reaped_closed >= p_limit;
        BEGIN
            PERFORM 1 FROM public.trace_account_bindings b
             WHERE b.tenant_id = c.tenant_id AND b.account_id = c.account_id
               AND b.state = c.state
             FOR UPDATE SKIP LOCKED;
            IF NOT FOUND THEN
                v_skipped := v_skipped + 1;
                CONTINUE;
            END IF;
            PERFORM 1 FROM public.trace_accounts a
             WHERE a.tenant_id = c.tenant_id AND a.account_id = c.account_id
             FOR UPDATE SKIP LOCKED;
            IF NOT FOUND THEN
                v_skipped := v_skipped + 1;
                CONTINUE;
            END IF;
            -- Re-check under both locks.
            IF EXISTS (
                SELECT 1 FROM public.trace_sessions s
                 WHERE s.tenant_id = c.tenant_id AND s.account_id = c.account_id
                   AND s.revoked_at IS NULL AND s.expires_at > clock_timestamp()) THEN
                v_skipped := v_skipped + 1;
                CONTINUE;
            END IF;
            -- The account alone. Its tenant row is never deleted. A
            -- non-cascading account-keyed row raises 23503 below.
            DELETE FROM public.trace_accounts a
             WHERE a.tenant_id = c.tenant_id AND a.account_id = c.account_id;
            IF c.state = 'unbound' THEN
                v_reaped_unbound := v_reaped_unbound + 1;
            ELSE
                v_reaped_closed := v_reaped_closed + 1;
            END IF;
        EXCEPTION WHEN foreign_key_violation OR lock_not_available OR deadlock_detected THEN
            v_skipped := v_skipped + 1;
        END;
    END LOOP;
    reaped_unbound := v_reaped_unbound;
    reaped_closed := v_reaped_closed;
    skipped := v_skipped;
    RETURN NEXT;
END $$;

GRANT CREATE ON SCHEMA public TO trace_unbound_account_reaper_guard;
ALTER FUNCTION trace_reap_unbound_accounts(BIGINT, BIGINT, INTEGER)
    OWNER TO trace_unbound_account_reaper_guard;
REVOKE CREATE ON SCHEMA public FROM trace_unbound_account_reaper_guard;
REVOKE ALL ON FUNCTION trace_reap_unbound_accounts(BIGINT, BIGINT, INTEGER) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION trace_reap_unbound_accounts(BIGINT, BIGINT, INTEGER)
    TO trace_unbound_account_reaper;
REVOKE trace_unbound_account_reaper_guard FROM CURRENT_USER;
