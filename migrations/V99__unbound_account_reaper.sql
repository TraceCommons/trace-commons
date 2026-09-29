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
-- Renumbering: S1 is V97, S2 takes V98 and S3 V100, so this is V99. Main
-- stood at V95 when this was written. It depends only on V30 (accounts,
-- sessions), V32 (credentials), V97 (bindings) and the tables the cascade
-- reaches, so it may be renumbered freely at merge.
--
-- CROSS-TENANT, SO A DEFINER FUNCTION. The sweep spans tenants, and the
-- runtime pool is tenant-scoped under forced RLS, so nothing here grants the
-- runtime anything. The function is owned by a NOLOGIN NOBYPASSRLS guard that
-- holds column-scoped privileges and role-scoped permissive policies, as
-- V85's enumeration guard does; a separate NOLOGIN worker role holds EXECUTE
-- and nothing else. The deployer grants the worker to the login that runs
-- the reaper.
--
-- WHAT GOES, AND THE TENANT REFUSAL. A candidate is deleted by deleting its
-- account and then its tenant, and both cascade. The tenant cascade alone
-- reaches about fifty tenant-keyed tables, so the function first proves the
-- tenant holds only what a passkey tenant may hold (the spec's "unbound"
-- state): its tenant row, the one account, that account's binding,
-- credential and sessions, and trace_account_audit rows. If the tenant holds
-- another account or ANY row in any other table keyed to trace_tenants or
-- trace_accounts, the whole candidate is refused -- nothing is deleted -- and
-- counted skipped.
--
-- That check is driven by the catalog, not a hand-kept list: every foreign
-- key into trace_tenants or trace_accounts (or the allowed tables) through a
-- tenant_id column puts its table IN SCOPE. The guard is NOBYPASSRLS, so a
-- scope table it cannot see into would read as empty and be cascaded away
-- unseen. The function therefore refuses the whole call, raising
-- 'unbound_reaper_scope_incomplete: <tables>', when any scope table lacks the
-- guard's SELECT on its tenant column or a permissive SELECT policy for the
-- guard. This migration grants both on every scope table that exists when it
-- runs. A LATER MIGRATION THAT ADDS A TENANT-KEYED TABLE MUST GRANT THE GUARD
-- THE SAME, or the reaper stops; unbound_account_reaper_pg fails in CI until
-- it does. Table names are schema, not data, so the message may carry them.
--
-- AUDIT. trace_audit_events has no foreign key to trace_tenants, so those
-- rows are not deleted: they are hash-chained and hash-only, and every other
-- deletion in the repo (withdrawal, purge) retains them. trace_account_audit
-- cascades with the tenant. The reaper cannot audit into the tenant it
-- deletes, so it reports counts only, through its return value.
--
-- CONCURRENCY. Each candidate is handled in its own sub-transaction:
--   1. the binding row is locked FOR UPDATE SKIP LOCKED with its state
--      re-checked in the same statement. A concurrent bind (S3) that has
--      updated or is updating the row either commits first (the state no
--      longer matches, so the row is not returned) or holds the lock (the row
--      is skipped).
--   2. the account row, then the tenant row, are locked FOR UPDATE SKIP
--      LOCKED. Inserting any row keyed to either takes a key-share lock on it
--      through the foreign key, so a concurrent sign-in or other tenant write
--      makes the candidate skipped; one that commits earlier is seen by 3.
--      Holding the tenant lock is what stops a row from slipping in between
--      the scope check and the delete.
--   3. live sessions, other accounts and the scope are re-checked in fresh
--      statements, under all three locks.
-- Nothing waits on a lock the reaper cannot take, but the cascade itself can
-- wait on a child row (a session being updated) held by a transaction that
-- then waits on the reaper's account or tenant lock. That is a deadlock; the
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
GRANT SELECT (tenant_id), DELETE ON trace_tenants TO trace_unbound_account_reaper_guard;
GRANT UPDATE (created_at) ON trace_tenants TO trace_unbound_account_reaper_guard;

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
DROP POLICY IF EXISTS trace_unbound_reaper_read ON trace_tenants;
CREATE POLICY trace_unbound_reaper_read ON trace_tenants
    FOR SELECT TO trace_unbound_account_reaper_guard USING (TRUE);
DROP POLICY IF EXISTS trace_unbound_reaper_lock ON trace_tenants;
CREATE POLICY trace_unbound_reaper_lock ON trace_tenants
    FOR UPDATE TO trace_unbound_account_reaper_guard USING (TRUE) WITH CHECK (TRUE);
DROP POLICY IF EXISTS trace_unbound_reaper_delete ON trace_tenants;
CREATE POLICY trace_unbound_reaper_delete ON trace_tenants
    FOR DELETE TO trace_unbound_account_reaper_guard USING (TRUE);
-- An earlier draft read trace_submissions under this name; the scope loop
-- below covers it now.
DROP POLICY IF EXISTS trace_unbound_reaper_read ON trace_submissions;

-- The tenant-refusal scope: SELECT on the tenant column and a permissive
-- SELECT policy for the guard on every table keyed to a tenant or account.
-- The query is the one the function's preflight runs; keep them identical.
DO $$
DECLARE
    r RECORD;
BEGIN
    FOR r IN
        SELECT DISTINCT con.conrelid::regclass AS rel, a.attname AS col
          FROM pg_constraint con
          CROSS JOIN LATERAL unnest(con.conkey, con.confkey) AS k(ck, fk)
          JOIN pg_attribute ra ON ra.attrelid = con.confrelid AND ra.attnum = k.fk
          JOIN pg_attribute a ON a.attrelid = con.conrelid AND a.attnum = k.ck
         WHERE con.contype = 'f'
           AND ra.attname = 'tenant_id'
           AND con.confrelid IN ('public.trace_tenants'::regclass,
                               'public.trace_accounts'::regclass,
                               'public.trace_account_bindings'::regclass,
                               'public.trace_webauthn_credentials'::regclass,
                               'public.trace_sessions'::regclass,
                               'public.trace_account_audit'::regclass)
           AND con.conrelid NOT IN ('public.trace_accounts'::regclass,
                                  'public.trace_account_bindings'::regclass,
                                  'public.trace_webauthn_credentials'::regclass,
                                  'public.trace_sessions'::regclass,
                                  'public.trace_account_audit'::regclass)
    LOOP
        EXECUTE format('GRANT SELECT (%I) ON %s TO trace_unbound_account_reaper_guard',
                       r.col, r.rel);
        EXECUTE format('DROP POLICY IF EXISTS trace_unbound_reaper_scope ON %s', r.rel);
        EXECUTE format('CREATE POLICY trace_unbound_reaper_scope ON %s
                            FOR SELECT TO trace_unbound_account_reaper_guard USING (TRUE)',
                       r.rel);
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
    v_me             OID;
    v_missing        TEXT;
    v_scope_sql      TEXT;
    v_held           BOOLEAN;
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

    -- Preflight: every table in scope must be visible to this role, or a
    -- tenant's rows in it would read as absent and be cascaded away.
    SELECT r.oid INTO v_me FROM pg_roles r WHERE r.rolname = current_user;
    WITH fk AS (
        SELECT DISTINCT con.conrelid AS rel, a.attname AS col
          FROM pg_constraint con
          CROSS JOIN LATERAL unnest(con.conkey, con.confkey) AS k(ck, fk)
          JOIN pg_attribute ra ON ra.attrelid = con.confrelid AND ra.attnum = k.fk
          JOIN pg_attribute a ON a.attrelid = con.conrelid AND a.attnum = k.ck
         WHERE con.contype = 'f'
           AND ra.attname = 'tenant_id'
           AND con.confrelid IN ('public.trace_tenants'::regclass,
                               'public.trace_accounts'::regclass,
                               'public.trace_account_bindings'::regclass,
                               'public.trace_webauthn_credentials'::regclass,
                               'public.trace_sessions'::regclass,
                               'public.trace_account_audit'::regclass)
           AND con.conrelid NOT IN ('public.trace_accounts'::regclass,
                                  'public.trace_account_bindings'::regclass,
                                  'public.trace_webauthn_credentials'::regclass,
                                  'public.trace_sessions'::regclass,
                                  'public.trace_account_audit'::regclass)
    ), checked AS (
        SELECT fk.rel, fk.col,
               has_column_privilege(fk.rel, fk.col, 'SELECT')
               AND (NOT cl.relrowsecurity OR EXISTS (
                    SELECT 1 FROM pg_policy p
                     WHERE p.polrelid = fk.rel AND p.polpermissive
                       AND p.polcmd IN ('r', '*') AND v_me = ANY (p.polroles))) AS visible
          FROM fk JOIN pg_class cl ON cl.oid = fk.rel
    )
    SELECT string_agg(DISTINCT rel::regclass::text, ', ') FILTER (WHERE NOT visible),
           'SELECT ' || coalesce(
               string_agg(format('EXISTS (SELECT 1 FROM %s WHERE %I = $1)', rel::regclass, col),
                          ' OR ') FILTER (WHERE visible),
               'FALSE')
      INTO v_missing, v_scope_sql
      FROM checked;
    IF v_missing IS NOT NULL THEN
        RAISE EXCEPTION 'unbound_reaper_scope_incomplete: %', v_missing;
    END IF;

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
            PERFORM 1 FROM public.trace_tenants t
             WHERE t.tenant_id = c.tenant_id
             FOR UPDATE SKIP LOCKED;
            IF NOT FOUND THEN
                v_skipped := v_skipped + 1;
                CONTINUE;
            END IF;
            -- Re-check under the locks.
            IF EXISTS (
                SELECT 1 FROM public.trace_sessions s
                 WHERE s.tenant_id = c.tenant_id AND s.account_id = c.account_id
                   AND s.revoked_at IS NULL AND s.expires_at > clock_timestamp()) THEN
                v_skipped := v_skipped + 1;
                CONTINUE;
            END IF;
            -- The tenant refusal: another account, or any row in scope.
            IF EXISTS (
                SELECT 1 FROM public.trace_accounts a
                 WHERE a.tenant_id = c.tenant_id AND a.account_id <> c.account_id) THEN
                v_skipped := v_skipped + 1;
                CONTINUE;
            END IF;
            EXECUTE v_scope_sql INTO v_held USING c.tenant_id;
            IF v_held THEN
                v_skipped := v_skipped + 1;
                CONTINUE;
            END IF;

            DELETE FROM public.trace_accounts a
             WHERE a.tenant_id = c.tenant_id AND a.account_id = c.account_id;
            DELETE FROM public.trace_tenants t WHERE t.tenant_id = c.tenant_id;
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
