-- Submissions a contributor's daemon could not send because the redaction
-- witness was busy, did not answer in time, or could not be reached
-- (owner design 2026-11-05). The witness cannot count these itself: a
-- refusal at the gateway in front of it, or a client that gave up waiting,
-- never reaches its own counters. The device counts them by label and
-- reports the totals on its next history read
-- (`POST /v1/contributors/me/submission-status`, header
-- `x-trace-witness-blocked`, `trace_commons_protocol::witness_blocked`).
--
-- WHAT A ROW HOLDS. One tenant, one UTC day, one label, and the total
-- reported for them. The tenant is the one the request authenticated as,
-- never a value from the request. The day is set by the database clock when
-- the row is first written; the runtime cannot write it. No device, account,
-- principal, trace or submission id, and no time finer than the day.
--
-- BOUNDS. A report carries at most 1,000 per label (the protocol's
-- WITNESS_BLOCKED_REPORT_MAX); the service refuses a larger one before it
-- gets here. A day's total for one tenant and label stops at 100,000: an
-- upsert past it holds the total there, and the CHECK is the backstop.
--
-- RLS STANCE. Forced, with the central tenant predicate
-- (trace_corpus_tenant_isolation) for the runtime, which writes inside the
-- authenticated tenant's context and may SELECT, INSERT (tenant_id, label,
-- blocked_count) and UPDATE (blocked_count) only. The operator's totals are
-- across tenants, which that predicate forbids, so they are read through
-- trace_witness_blocked_totals(), a SECURITY DEFINER function owned by
-- trace_witness_blocked_reader, a NOLOGIN NOBYPASSRLS role whose only
-- privilege is SELECT on this table through its own policy. The function
-- returns sums and distinct-tenant counts, never a tenant id. The runtime
-- may EXECUTE it and gains no read of other tenants' rows.
--
-- Applied by a non-superuser CREATEROLE migrator, as V152 is: it holds the
-- reader just long enough to own the function, then relinquishes it.

DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_witness_blocked_reader') THEN
        CREATE ROLE trace_witness_blocked_reader NOLOGIN NOSUPERUSER NOBYPASSRLS;
    END IF;
    -- CREATEROLE cannot restate NOSUPERUSER or NOBYPASSRLS in ALTER ROLE.
    IF EXISTS (SELECT 1 FROM pg_roles
               WHERE rolname = 'trace_witness_blocked_reader' AND (rolsuper OR rolbypassrls)) THEN
        RAISE EXCEPTION 'trace_witness_blocked_reader must be NOSUPERUSER NOBYPASSRLS';
    END IF;
END $$;
ALTER ROLE trace_witness_blocked_reader NOLOGIN;
GRANT USAGE ON SCHEMA public TO trace_witness_blocked_reader;
GRANT trace_witness_blocked_reader TO CURRENT_USER;

CREATE TABLE IF NOT EXISTS trace_witness_blocked_days (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    day DATE NOT NULL DEFAULT ((clock_timestamp() AT TIME ZONE 'UTC')::date),
    label TEXT NOT NULL CHECK (label IN ('busy', 'timed_out', 'unreachable')),
    blocked_count BIGINT NOT NULL CHECK (blocked_count BETWEEN 1 AND 100000),
    PRIMARY KEY (tenant_id, day, label)
);

CREATE INDEX IF NOT EXISTS trace_witness_blocked_days_day
    ON trace_witness_blocked_days (day);

ALTER TABLE trace_witness_blocked_days ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_witness_blocked_days FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_witness_blocked_days;
CREATE POLICY trace_corpus_tenant_isolation ON trace_witness_blocked_days
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());
DROP POLICY IF EXISTS trace_witness_blocked_reader_read ON trace_witness_blocked_days;
CREATE POLICY trace_witness_blocked_reader_read ON trace_witness_blocked_days
    FOR SELECT TO trace_witness_blocked_reader
    USING (TRUE);

REVOKE ALL ON trace_witness_blocked_days FROM PUBLIC;
GRANT SELECT ON trace_witness_blocked_days TO trace_ingest_runtime;
GRANT INSERT (tenant_id, label, blocked_count) ON trace_witness_blocked_days
    TO trace_ingest_runtime;
GRANT UPDATE (blocked_count) ON trace_witness_blocked_days TO trace_ingest_runtime;
GRANT SELECT ON trace_witness_blocked_days TO trace_witness_blocked_reader;

-- The operator's totals since a day: one row per (day, label), one per
-- label over the whole window (day NULL), and one for everything (both
-- NULL). `tenants` counts distinct tenants with a non-zero total in the
-- row's group. Sums and counts only.
CREATE OR REPLACE FUNCTION trace_witness_blocked_totals(p_since DATE)
RETURNS TABLE (day DATE, label TEXT, blocked BIGINT, tenants BIGINT)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog AS $$
    SELECT d.day, d.label, COALESCE(pg_catalog.sum(d.blocked_count), 0)::BIGINT,
           pg_catalog.count(DISTINCT d.tenant_id)
      FROM public.trace_witness_blocked_days d
     WHERE d.day >= p_since
     GROUP BY GROUPING SETS ((d.day, d.label), (d.label), ())
     ORDER BY d.day NULLS LAST, d.label NULLS LAST
$$;

GRANT CREATE ON SCHEMA public TO trace_witness_blocked_reader;
ALTER FUNCTION trace_witness_blocked_totals(DATE) OWNER TO trace_witness_blocked_reader;
REVOKE CREATE ON SCHEMA public FROM trace_witness_blocked_reader;
REVOKE ALL ON FUNCTION trace_witness_blocked_totals(DATE) FROM PUBLIC;
-- The runtime, and the migrator for a deployment whose ingest login is the
-- migrator (V149's reason).
GRANT EXECUTE ON FUNCTION trace_witness_blocked_totals(DATE) TO trace_ingest_runtime;
GRANT EXECUTE ON FUNCTION trace_witness_blocked_totals(DATE) TO CURRENT_USER;

REVOKE trace_witness_blocked_reader FROM CURRENT_USER;
