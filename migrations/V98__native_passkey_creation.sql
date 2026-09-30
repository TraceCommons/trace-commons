-- Native passkey creation (Z2, slice S2;
-- docs/superpowers/specs/2026-09-28-native-passkey-identity-design.md).
--
-- S2 adds the first writer of trace_account_bindings: the unauthenticated
-- POST /v1/account/native/passkey/create/finish, which writes a fresh tenant,
-- the account and its `unbound` binding row, the credential, a native session
-- and the audit row in one transaction. Everything else that transaction
-- writes is a table the ingest runtime already writes (sessions, credentials,
-- accounts, tenants, account audit), so the only new grant is INSERT on
-- trace_account_bindings. No UPDATE and no DELETE: flipping a row to `bound`
-- is S3's, and removing one is the cascade from its account.
--
-- The unbound-account ceiling (TRACE_COMMONS_UNBOUND_PASSKEY_ACCOUNT_CEILING)
-- is checked against the count of `unbound` rows in EVERY tenant. The runtime
-- only ever sees one tenant through forced RLS, so the count is a SECURITY
-- DEFINER function owned by a NOLOGIN NOBYPASSRLS guard, following V82's
-- pattern. The guard holds SELECT on the `state` column alone, and a
-- permissive policy scoped to the guard alone lets it see rows in every
-- tenant. The function returns one number and takes no argument, so it
-- discloses nothing a caller could narrow to a tenant or an account.
--
-- Applied by a non-superuser CREATEROLE migrator: CREATE ROLE is guarded,
-- GRANT and CREATE POLICY on a table need only its ownership, and the owner
-- transfer below needs membership in the guard, which the migrator grants
-- itself (ADMIN comes from having created the role, or from CREATEROLE on 15
-- and earlier) and drops again at the end.

DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_unbound_account_count_guard') THEN
        CREATE ROLE trace_unbound_account_count_guard NOLOGIN NOBYPASSRLS;
    END IF;
END $$;
-- Only NOLOGIN is re-asserted: changing BYPASSRLS needs a superuser, and the
-- migrating owner is not one (migration_atomicity_pg). Refuse a pre-existing
-- privileged role rather than own a function with it.
ALTER ROLE trace_unbound_account_count_guard NOLOGIN;
DO $$ BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_unbound_account_count_guard'
               AND (rolsuper OR rolbypassrls)) THEN
        RAISE EXCEPTION 'V98: trace_unbound_account_count_guard is privileged';
    END IF;
END $$;
GRANT trace_unbound_account_count_guard TO CURRENT_USER;
GRANT USAGE ON SCHEMA public TO trace_unbound_account_count_guard;
GRANT SELECT (state) ON trace_account_bindings TO trace_unbound_account_count_guard;

-- Permissive policies OR together. For the guard this adds cross-tenant
-- visibility of unbound rows; every other role still sees only its tenant's.
DROP POLICY IF EXISTS trace_unbound_account_count_guard_read ON trace_account_bindings;
CREATE POLICY trace_unbound_account_count_guard_read ON trace_account_bindings
    FOR SELECT TO trace_unbound_account_count_guard
    USING (state = 'unbound');

CREATE FUNCTION public.trace_unbound_passkey_account_count()
RETURNS BIGINT
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog AS $$
    SELECT count(*) FROM public.trace_account_bindings WHERE state = 'unbound'
$$;
GRANT CREATE ON SCHEMA public TO trace_unbound_account_count_guard;
ALTER FUNCTION public.trace_unbound_passkey_account_count()
    OWNER TO trace_unbound_account_count_guard;
REVOKE CREATE ON SCHEMA public FROM trace_unbound_account_count_guard;
REVOKE ALL ON FUNCTION public.trace_unbound_passkey_account_count() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.trace_unbound_passkey_account_count()
    TO trace_ingest_runtime;
REVOKE trace_unbound_account_count_guard FROM CURRENT_USER;

-- The one new runtime write: create/finish inserts the binding row.
GRANT INSERT ON trace_account_bindings TO trace_ingest_runtime;
