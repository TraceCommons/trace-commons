-- Closed passkey accounts count against the unbound ceiling (#1135 review;
-- Z2 native passkey identity).
--
-- V98's ceiling (TRACE_COMMONS_UNBOUND_PASSKEY_ACCOUNT_CEILING) counted only
-- `unbound` binding rows. S3's refuse branch (V100) moves a passkey account
-- to `closed`, which took it out of the count at once, while the row itself
-- stays until V101's reaper removes it 30 days after the close. So a caller
-- could create a passkey account, bind it to a NEAR AI login that already has
-- an account (which closes it), and repeat, holding many more rows than the
-- ceiling allows. Decided 2026-09-30: a closed-but-not-yet-reaped account
-- still counts. The ceiling now bounds `unbound` plus `closed`, and a closed
-- slot frees when the reaper deletes the account. `bound` rows never count.
--
-- This supersedes the V101 header's remark that closed rows are "already
-- outside the unbound ceiling's count (V98)". V101 is not edited: a migration
-- that has run is never rewritten.
--
-- The function keeps its name, signature, owner and grants, so nothing that
-- calls it changes. Two things widen, and both must: the function's
-- predicate, and the guard's permissive policy. The definer runs as the
-- guard, and forced RLS shows the guard only what its policy admits, so
-- widening the predicate alone would still count unbound rows only.
--
-- Applied by the same non-superuser CREATEROLE migrator as V98, with the
-- same ownership dance: CREATE OR REPLACE on a function needs its owner, so
-- the migrator holds the guard for the replacement and drops it again.
-- Idempotent: re-applying drops and re-creates the same policy and replaces
-- the function with the same body.
--
-- Numbering: the next free version on main when this was written. It depends
-- only on V97 (the table) and V98 (the guard and the function).

DO $$ BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_unbound_account_count_guard'
               AND (rolsuper OR rolbypassrls)) THEN
        RAISE EXCEPTION 'V102: trace_unbound_account_count_guard is privileged';
    END IF;
END $$;

DROP POLICY IF EXISTS trace_unbound_account_count_guard_read ON trace_account_bindings;
CREATE POLICY trace_unbound_account_count_guard_read ON trace_account_bindings
    FOR SELECT TO trace_unbound_account_count_guard
    USING (state IN ('unbound', 'closed'));

GRANT trace_unbound_account_count_guard TO CURRENT_USER;
CREATE OR REPLACE FUNCTION public.trace_unbound_passkey_account_count()
RETURNS BIGINT
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog AS $$
    SELECT count(*) FROM public.trace_account_bindings WHERE state IN ('unbound', 'closed')
$$;
REVOKE trace_unbound_account_count_guard FROM CURRENT_USER;
