-- Anonymous usage counts, US only (DRAFT, NEEDS APPROVAL, 2026-11-04).
--
-- One row per (install, ISO week, event): how many times an install opened
-- a screen or used a control that week. Written by POST /v1/usage-events,
-- which takes a batch from the contributor app with no credential.
--
-- install_id is a random UUIDv4 the app makes the first time counting turns
-- on. It is not a device key, account, enrollment, path or hardware id, and
-- nothing in this database joins to it. The table holds nothing else about
-- an install: no address, no tenant, no timestamps finer than the week the
-- app reported and the time the row was last written.
--
-- No tenant column and no tenant policy, by design: these rows belong to no
-- tenant and are written before anyone has an account, like V145's
-- cross-tenant table. Row security is enabled and forced, PUBLIC has
-- nothing, and the only role with any privilege or policy on the table is
-- trace_usage_events_guard, a NOLOGIN, NOSUPERUSER, NOBYPASSRLS role. The
-- ingest runtime cannot read, update or delete a row: it may only call
-- trace_usage_events_record(), a SECURITY DEFINER function owned by the
-- guard that upserts one batch. Reading the totals is an operator query run
-- as the guard (or a later admin route), never the runtime.
--
-- A week's count is replaced, not added to: the app sends each finished
-- week once and drops it on success, so a resend after a lost response must
-- not double it.
--
-- Applied by a non-superuser CREATEROLE migrator, as V101, V114 and V145
-- are: it holds the guard just long enough to own the function, then
-- relinquishes it.

DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_usage_events_guard') THEN
        CREATE ROLE trace_usage_events_guard NOLOGIN NOSUPERUSER NOBYPASSRLS;
    END IF;
    -- CREATEROLE cannot restate NOSUPERUSER or NOBYPASSRLS in ALTER ROLE.
    IF EXISTS (SELECT 1 FROM pg_roles
               WHERE rolname = 'trace_usage_events_guard' AND (rolsuper OR rolbypassrls)) THEN
        RAISE EXCEPTION 'trace_usage_events_guard must be NOSUPERUSER NOBYPASSRLS';
    END IF;
END $$;
ALTER ROLE trace_usage_events_guard NOLOGIN;
GRANT USAGE ON SCHEMA public TO trace_usage_events_guard;
GRANT trace_usage_events_guard TO CURRENT_USER;

CREATE TABLE IF NOT EXISTS trace_usage_event_weeks (
    install_id  UUID NOT NULL,
    week        TEXT NOT NULL CHECK (week ~ '^[0-9]{4}-W(0[1-9]|[1-4][0-9]|5[0-3])$'),
    event       TEXT NOT NULL CHECK (event ~ '^[a-z][a-z_]{0,62}$'),
    event_count BIGINT NOT NULL CHECK (event_count > 0 AND event_count <= 100000),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (install_id, week, event)
);

ALTER TABLE trace_usage_event_weeks ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_usage_event_weeks FORCE ROW LEVEL SECURITY;
REVOKE ALL ON trace_usage_event_weeks FROM PUBLIC;
GRANT SELECT, INSERT, UPDATE, DELETE ON trace_usage_event_weeks TO trace_usage_events_guard;
DROP POLICY IF EXISTS trace_usage_events_guard_write ON trace_usage_event_weeks;
CREATE POLICY trace_usage_events_guard_write ON trace_usage_event_weeks
    TO trace_usage_events_guard USING (TRUE) WITH CHECK (TRUE);

-- One batch, all or nothing. The arrays are parallel: row i is
-- (p_weeks[i], p_events[i], p_counts[i]). The service validates the batch
-- against the protocol's allow-list before calling; the table's checks are
-- the backstop. Returns the number of rows written.
CREATE OR REPLACE FUNCTION trace_usage_events_record(
    p_install_id UUID,
    p_weeks      TEXT[],
    p_events     TEXT[],
    p_counts     BIGINT[]
) RETURNS INTEGER
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE
    v_rows INTEGER;
BEGIN
    IF p_install_id IS NULL
       OR pg_catalog.cardinality(p_weeks) IS DISTINCT FROM pg_catalog.cardinality(p_events)
       OR pg_catalog.cardinality(p_weeks) IS DISTINCT FROM pg_catalog.cardinality(p_counts)
       OR pg_catalog.cardinality(p_weeks) > 256 THEN
        RAISE EXCEPTION 'usage_batch_invalid' USING ERRCODE = '22023';
    END IF;
    INSERT INTO public.trace_usage_event_weeks (install_id, week, event, event_count)
    SELECT p_install_id, r.week, r.event, r.event_count
      FROM unnest(p_weeks, p_events, p_counts) AS r(week, event, event_count)
    ON CONFLICT (install_id, week, event) DO UPDATE
       SET event_count = EXCLUDED.event_count,
           updated_at = pg_catalog.clock_timestamp();
    GET DIAGNOSTICS v_rows = ROW_COUNT;
    RETURN v_rows;
END $$;

GRANT CREATE ON SCHEMA public TO trace_usage_events_guard;
ALTER FUNCTION trace_usage_events_record(UUID, TEXT[], TEXT[], BIGINT[])
    OWNER TO trace_usage_events_guard;
REVOKE CREATE ON SCHEMA public FROM trace_usage_events_guard;
REVOKE ALL ON FUNCTION trace_usage_events_record(UUID, TEXT[], TEXT[], BIGINT[]) FROM PUBLIC;
-- The runtime, and the migrator for a deployment whose ingest login is the
-- migrator (V149's reason).
GRANT EXECUTE ON FUNCTION trace_usage_events_record(UUID, TEXT[], TEXT[], BIGINT[])
    TO trace_ingest_runtime;
GRANT EXECUTE ON FUNCTION trace_usage_events_record(UUID, TEXT[], TEXT[], BIGINT[])
    TO CURRENT_USER;

REVOKE trace_usage_events_guard FROM CURRENT_USER;
