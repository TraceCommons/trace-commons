-- Repair the public-run function ACL and trigger that V64 left to a
-- non-superuser migrator.
--
-- V64 handed its four definer functions to trace_public_run_reader and
-- trace_public_run_graph_guard, left both roles, and only then revoked PUBLIC's
-- EXECUTE and granted its own. Only an owner, a member of the owner role, or a
-- superuser may change a function's ACL, and for anyone else PostgreSQL warns
-- (`no privileges could be revoked`, `no privileges were granted`) instead of
-- failing, so the migration recorded as applied with PUBLIC still holding
-- EXECUTE on all four and the migrator holding none. V64 is recorded wherever
-- it ran, so the repair is here: rejoin, revoke and grant, leave.
--
-- The runtime no longer gets EXECUTE by being whoever ran the migration. It
-- gets it as a member of trace_public_run_runtime, the way trace_reward_runtime
-- works, so a deployment that migrates as one role and serves as another can
-- name the serving role. CURRENT_USER keeps EXECUTE too, for the deployments
-- where the two are the same role.
--
-- V64's trigger function ran as the caller and updated trace_public_runs, on
-- which the caller -- the ingest login -- holds nothing in a least-privilege
-- deployment: revoking an accepted submission failed with `permission denied
-- for table trace_public_runs`. It now runs as trace_public_run_unpublisher,
-- which holds exactly the columns that one UPDATE touches. The caller's tenant
-- is deliberately not cleared: the forced trace_corpus_tenant_isolation policy
-- reads it, and that is what keeps the update to the caller's own tenant.

DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_catalog.pg_roles WHERE rolname = 'trace_public_run_runtime') THEN
        CREATE ROLE trace_public_run_runtime NOLOGIN NOBYPASSRLS;
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_catalog.pg_roles WHERE rolname = 'trace_public_run_unpublisher') THEN
        CREATE ROLE trace_public_run_unpublisher NOLOGIN NOBYPASSRLS;
    END IF;
END $$;

-- As V69: a role of either name may already exist on this server with more than
-- this migration would have given it, and only a superuser may take SUPERUSER
-- or BYPASSRLS away. Look, and refuse to build on one that holds either.
ALTER ROLE trace_public_run_runtime NOLOGIN;
ALTER ROLE trace_public_run_unpublisher NOLOGIN;
DO $$
DECLARE
    v_role TEXT;
BEGIN
    SELECT rolname INTO v_role FROM pg_catalog.pg_roles
     WHERE rolname IN ('trace_public_run_runtime', 'trace_public_run_unpublisher')
       AND (rolsuper OR rolbypassrls)
     ORDER BY rolname LIMIT 1;
    IF v_role IS NOT NULL THEN
        RAISE EXCEPTION 'V74: role % already exists with SUPERUSER or BYPASSRLS; refusing to build on it', v_role;
    END IF;
END $$;

GRANT USAGE ON SCHEMA public TO trace_public_run_runtime, trace_public_run_unpublisher;

-- The four V64 functions, from inside their owner roles this time.
GRANT trace_public_run_reader TO CURRENT_USER;
REVOKE ALL ON FUNCTION trace_public_run_page(TEXT, INTEGER) FROM PUBLIC;
REVOKE ALL ON FUNCTION trace_resolve_public_run_source(TEXT) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION trace_public_run_page(TEXT, INTEGER)
    TO trace_public_run_runtime, CURRENT_USER;
GRANT EXECUTE ON FUNCTION trace_resolve_public_run_source(TEXT)
    TO trace_public_run_runtime, CURRENT_USER;
REVOKE trace_public_run_reader FROM CURRENT_USER;

GRANT trace_public_run_graph_guard TO CURRENT_USER;
REVOKE ALL ON FUNCTION trace_public_run_would_cycle(UUID, UUID) FROM PUBLIC;
REVOKE ALL ON FUNCTION trace_public_run_retained_source(TEXT, UUID, UUID) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION trace_public_run_would_cycle(UUID, UUID)
    TO trace_public_run_runtime, CURRENT_USER;
GRANT EXECUTE ON FUNCTION trace_public_run_retained_source(TEXT, UUID, UUID)
    TO trace_public_run_runtime, CURRENT_USER;
REVOKE trace_public_run_graph_guard FROM CURRENT_USER;

-- The trigger's one UPDATE: it sets three columns and reads four (a column an
-- UPDATE reads needs SELECT, and `version = version + 1` reads version).
GRANT SELECT (tenant_id, submission_id, unpublished_at, version),
      UPDATE (unpublished_at, updated_at, version)
    ON trace_public_runs TO trace_public_run_unpublisher;

CREATE OR REPLACE FUNCTION trace_unpublish_run_when_submission_leaves_accepted()
RETURNS TRIGGER
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, public
AS $$
BEGIN
    IF OLD.status = 'accepted' AND NEW.status <> 'accepted' THEN
        UPDATE trace_public_runs
        SET unpublished_at = COALESCE(unpublished_at, NOW()),
            updated_at = NOW(),
            version = version + 1
        WHERE tenant_id = NEW.tenant_id
          AND submission_id = NEW.submission_id
          AND unpublished_at IS NULL;
    END IF;
    RETURN NEW;
END;
$$;

-- CREATE ON SCHEMA for the length of the ownership transfer, as V59 and V64 do:
-- a function's new owner must hold it, and from PostgreSQL 15 PUBLIC does not.
GRANT trace_public_run_unpublisher TO CURRENT_USER;
GRANT CREATE ON SCHEMA public TO trace_public_run_unpublisher;
ALTER FUNCTION trace_unpublish_run_when_submission_leaves_accepted()
    OWNER TO trace_public_run_unpublisher;
REVOKE CREATE ON SCHEMA public FROM trace_public_run_unpublisher;
REVOKE ALL ON FUNCTION trace_unpublish_run_when_submission_leaves_accepted() FROM PUBLIC;
REVOKE trace_public_run_unpublisher FROM CURRENT_USER;

-- The migrator must end up holding ADMIN on trace_public_run_runtime, so it can
-- hand the role to the login the deployment serves as. Only when it does not
-- hold it already: since PostgreSQL 16 the role's creator holds ADMIN from the
-- moment of creation, and granting it to yourself a second time is refused to
-- anyone but a superuser -- `ADMIN option cannot be granted back to your own
-- grantor`. As V69 does for trace_reward_runtime.
DO $$ BEGIN
    IF NOT EXISTS (
        SELECT 1
          FROM pg_catalog.pg_auth_members membership
          JOIN pg_catalog.pg_roles granted ON granted.oid = membership.roleid
          JOIN pg_catalog.pg_roles holder ON holder.oid = membership.member
         WHERE granted.rolname = 'trace_public_run_runtime'
           AND holder.rolname = CURRENT_USER
           AND membership.admin_option
    ) THEN
        GRANT trace_public_run_runtime TO CURRENT_USER WITH ADMIN OPTION;
    END IF;
END $$;
