-- The unbound-account reaper (Z2, slice S5;
-- docs/superpowers/specs/2026-09-28-native-passkey-identity-design.md).
--
-- Cancel leaves an unbound passkey account inert. This function is what
-- eventually reclaims it: it deletes accounts whose binding is still
-- `unbound`, whose binding is older than the TTL, and that have no session
-- activity inside the TTL. Bound, closed and legacy accounts (no binding row)
-- are never candidates.
--
-- Renumbering: S1 is V97 and S2 takes V98, so this is V99. It depends only on
-- V30 (accounts, sessions), V32 (credentials), V97 (bindings) and the tables
-- the cascade reaches, so it may be renumbered freely at merge.
--
-- LAST ACTIVITY is the newest trace_sessions.last_seen_at for the account, or
-- the binding's created_at when there is no session. Neither column is new.
-- A session that is unrevoked and unexpired also blocks deletion, whatever
-- its last_seen_at, so a live session is never reaped from under its holder.
--
-- CROSS-TENANT, SO A DEFINER FUNCTION. The sweep spans tenants, and the
-- runtime pool is tenant-scoped under forced RLS, so nothing here grants the
-- runtime anything. The function is owned by a NOLOGIN NOBYPASSRLS guard that
-- holds column-scoped privileges and role-scoped permissive policies, as
-- V85's enumeration guard does; a separate NOLOGIN worker role holds EXECUTE
-- and nothing else. The deployer grants the worker to the login that runs
-- the reaper.
--
-- WHAT GOES. Deleting the account cascades (ON DELETE CASCADE) to its binding
-- (V97), passkey credential (V32), sessions (V30), login links and every other
-- account-keyed table that cascades. The tenant row is then deleted too, but
-- only when the account was the tenant's last one and the tenant holds no
-- submission, so a tenant that somehow holds anything else is left alone. A
-- passkey tenant is fresh and random, one account per tenant, so in practice
-- the tenant goes with the account, taking trace_account_audit and the other
-- tenant-keyed cascades with it.
--
-- AUDIT. trace_audit_events has no foreign key to trace_tenants, so those
-- rows are not deleted: they are hash-chained and hash-only, and every other
-- deletion in the repo (withdrawal, purge) retains them. trace_account_audit
-- cascades with the tenant. The reaper cannot audit into the tenant it
-- deletes, so it reports counts only, through its return value.
--
-- CONCURRENCY. Each candidate is handled in its own sub-transaction:
--   1. the binding row is locked FOR UPDATE SKIP LOCKED with state re-checked
--      in the same statement. A concurrent bind (S3) that has updated or is
--      updating the row either commits first (state is no longer 'unbound',
--      so the row is not returned) or holds the lock (the row is skipped).
--   2. the account row is locked FOR UPDATE SKIP LOCKED. A concurrent sign-in
--      inserting a session holds a key-share lock on it through the foreign
--      key, so the candidate is skipped; one that commits earlier is seen by
--      step 3.
--   3. activity is re-checked in a fresh statement, under both locks.
-- Nothing waits on a lock the reaper cannot take, so it cannot deadlock with
-- bind or sign-in. A foreign-key refusal (an account that somehow holds a row
-- in a non-cascading table) rolls that candidate back and counts it skipped.
-- Batches are bounded by p_limit.
--
-- Returns (reaped, skipped): counts only, no identifier.
--
-- Idempotent where it can be, and applied by a non-superuser CREATEROLE
-- migrator, following V85: the migrator holds the owning guard just long
-- enough to own the function and to grant EXECUTE, then drops it.

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
GRANT SELECT (tenant_id, account_id, created_at), DELETE
    ON trace_accounts TO trace_unbound_account_reaper_guard;
GRANT UPDATE (created_at) ON trace_accounts TO trace_unbound_account_reaper_guard;
GRANT SELECT (tenant_id, account_id, last_seen_at, expires_at, revoked_at)
    ON trace_sessions TO trace_unbound_account_reaper_guard;
GRANT SELECT (tenant_id) ON trace_submissions TO trace_unbound_account_reaper_guard;
GRANT SELECT (tenant_id), DELETE ON trace_tenants TO trace_unbound_account_reaper_guard;

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
    FOR DELETE TO trace_unbound_account_reaper_guard USING (state = 'unbound');

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
DROP POLICY IF EXISTS trace_unbound_reaper_read ON trace_submissions;
CREATE POLICY trace_unbound_reaper_read ON trace_submissions
    FOR SELECT TO trace_unbound_account_reaper_guard USING (TRUE);
DROP POLICY IF EXISTS trace_unbound_reaper_read ON trace_tenants;
CREATE POLICY trace_unbound_reaper_read ON trace_tenants
    FOR SELECT TO trace_unbound_account_reaper_guard USING (TRUE);
DROP POLICY IF EXISTS trace_unbound_reaper_delete ON trace_tenants;
CREATE POLICY trace_unbound_reaper_delete ON trace_tenants
    FOR DELETE TO trace_unbound_account_reaper_guard USING (TRUE);

CREATE OR REPLACE FUNCTION trace_reap_unbound_accounts(p_ttl_seconds BIGINT, p_limit INTEGER)
RETURNS TABLE(reaped BIGINT, skipped BIGINT)
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE
    v_now     TIMESTAMPTZ := clock_timestamp();
    v_cutoff  TIMESTAMPTZ;
    v_reaped  BIGINT := 0;
    v_skipped BIGINT := 0;
    c         RECORD;
BEGIN
    -- A one-day floor is defense in depth against a misconfigured TTL of a
    -- few seconds deleting every idle account.
    IF p_ttl_seconds IS NULL OR p_ttl_seconds < 86400
       OR p_limit IS NULL OR p_limit < 1 OR p_limit > 1000 THEN
        RAISE EXCEPTION 'unbound_reaper_refused';
    END IF;
    PERFORM set_config('lock_timeout', '3s', true);
    v_cutoff := v_now - (p_ttl_seconds * interval '1 second');

    FOR c IN
        SELECT b.tenant_id, b.account_id
          FROM public.trace_account_bindings b
         WHERE b.state = 'unbound'
           AND b.created_at < v_cutoff
           AND NOT EXISTS (
                SELECT 1 FROM public.trace_sessions s
                 WHERE s.tenant_id = b.tenant_id AND s.account_id = b.account_id
                   AND (s.last_seen_at >= v_cutoff
                        OR (s.revoked_at IS NULL AND s.expires_at > v_now)))
         ORDER BY b.created_at, b.tenant_id, b.account_id
         LIMIT p_limit
    LOOP
        BEGIN
            PERFORM 1 FROM public.trace_account_bindings b
             WHERE b.tenant_id = c.tenant_id AND b.account_id = c.account_id
               AND b.state = 'unbound'
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
            IF EXISTS (
                SELECT 1 FROM public.trace_sessions s
                 WHERE s.tenant_id = c.tenant_id AND s.account_id = c.account_id
                   AND (s.last_seen_at >= v_cutoff
                        OR (s.revoked_at IS NULL AND s.expires_at > clock_timestamp()))) THEN
                v_skipped := v_skipped + 1;
                CONTINUE;
            END IF;

            DELETE FROM public.trace_accounts a
             WHERE a.tenant_id = c.tenant_id AND a.account_id = c.account_id;
            IF NOT EXISTS (SELECT 1 FROM public.trace_accounts a WHERE a.tenant_id = c.tenant_id)
               AND NOT EXISTS (SELECT 1 FROM public.trace_submissions x WHERE x.tenant_id = c.tenant_id)
            THEN
                DELETE FROM public.trace_tenants t WHERE t.tenant_id = c.tenant_id;
            END IF;
            v_reaped := v_reaped + 1;
        EXCEPTION WHEN foreign_key_violation OR lock_not_available THEN
            v_skipped := v_skipped + 1;
        END;
    END LOOP;
    reaped := v_reaped;
    skipped := v_skipped;
    RETURN NEXT;
END $$;

GRANT CREATE ON SCHEMA public TO trace_unbound_account_reaper_guard;
ALTER FUNCTION trace_reap_unbound_accounts(BIGINT, INTEGER)
    OWNER TO trace_unbound_account_reaper_guard;
REVOKE CREATE ON SCHEMA public FROM trace_unbound_account_reaper_guard;
REVOKE ALL ON FUNCTION trace_reap_unbound_accounts(BIGINT, INTEGER) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION trace_reap_unbound_accounts(BIGINT, INTEGER)
    TO trace_unbound_account_reaper;
REVOKE trace_unbound_account_reaper_guard FROM CURRENT_USER;
