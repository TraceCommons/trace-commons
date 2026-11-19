-- Let the ingest runtime group assume the public register-stats role.
--
-- V55 granted trace_commons_public_read TO CURRENT_USER, the role that
-- applied it. That covers a deployment whose ingest connects as its own
-- migrator, and nothing else. The pilot's ingest connects as a login that
-- owns nothing and holds its privileges through the NOLOGIN group
-- trace_ingest_runtime (V90), so every GET /v1/public/register-stats failed
-- `SET LOCAL ROLE trace_commons_public_read` with "permission denied to set
-- role" and answered 500.
--
-- Granting the group, not a named login, follows V90: whichever login is
-- in trace_ingest_runtime can serve the endpoint, and no deployment's login
-- name is written into the schema. Membership confers only what the public
-- role holds: a column-scoped SELECT on the one aggregate row, under its
-- own NOBYPASSRLS policy. V90 and V105 both create the group where it is
-- missing, so it exists here; the guard keeps this idempotent anyway.

DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_ingest_runtime') THEN
        CREATE ROLE trace_ingest_runtime NOLOGIN NOBYPASSRLS;
    END IF;
END $$;

GRANT trace_commons_public_read TO trace_ingest_runtime;
