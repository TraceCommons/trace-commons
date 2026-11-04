-- The first run's estimate requests (`POST /v1/public/first-run-copy`), one
-- row per request, kept for operator statistics (owner, 2026-11-04: "store
-- requests and response data in a database where we can query for user
-- stats").
--
-- WHAT A ROW HOLDS. The hour it arrived (truncated, set by the database
-- clock, never by the caller), the coarsened trace count and total size as
-- received (two significant figures; the size in whole MiB, as bytes), which
-- answer was served (`default`, `usd`, or `refused` with a refusal label),
-- the served lines' key (a label naming the line set, never the text), and
-- the app version and platform when the request carried them. A refused
-- request stores no figure: what it sent was not coarse. No address, no
-- device, account, tenant or session id, no request id: a row joins to
-- nothing.
--
-- NUMBERING. V152 is taken by open PR #1920. This migration depends on
-- nothing V152 adds, and the runner applies every unrecorded version in table
-- order and allows gaps, so it applies whether V152 lands before or after it.
--
-- RLS STANCE. The table has no tenant: a first run asks before any account
-- exists. It cannot carry the central tenant predicate and is not in
-- TRACE_COMMONS_RLS_TABLES (the same reason as trace_payout_rate_epochs,
-- V132). It forces RLS with exactly two policies:
--   * trace_first_run_stats_writer may INSERT, through a policy that only
--     checks the row's shape, and holds INSERT on every column but
--     received_hour, so no caller can date a row. It cannot SELECT, UPDATE
--     or DELETE.
--   * trace_first_run_stats_reader may SELECT every row, for the operator's
--     statistics route. It cannot write.
-- No role may UPDATE or DELETE: the log is append-only. Both roles are
-- NOLOGIN NOBYPASSRLS, assumed with SET LOCAL ROLE for one transaction,
-- never inherited, and not granted to trace_ingest_runtime here (that group's
-- memberships are pinned). The operator grants both to the ingest login:
--   GRANT trace_first_run_stats_writer, trace_first_run_stats_reader
--       TO <ingest login> WITH INHERIT FALSE, SET TRUE;
-- Until then, recording fails with a permission error, which the route logs
-- by label and answers anyway: a statistic never stops a first run.
--
-- Idempotent where a role is concerned, and applies under a non-superuser
-- CREATEROLE migrator (migration_atomicity_pg).

DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_first_run_stats_writer') THEN
        CREATE ROLE trace_first_run_stats_writer NOLOGIN NOBYPASSRLS;
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_first_run_stats_reader') THEN
        CREATE ROLE trace_first_run_stats_reader NOLOGIN NOBYPASSRLS;
    END IF;
END $$;
-- A pre-existing role is used as found, but never one that defeats forced
-- RLS or can log in.
DO $$ BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles
                WHERE rolname IN ('trace_first_run_stats_writer', 'trace_first_run_stats_reader')
                  AND (rolsuper OR rolbypassrls OR rolcanlogin)) THEN
        RAISE EXCEPTION 'V153: a first-run stats role is privileged or can log in';
    END IF;
END $$;
GRANT USAGE ON SCHEMA public TO trace_first_run_stats_writer, trace_first_run_stats_reader;

CREATE TABLE trace_first_run_estimate_requests (
    received_hour TIMESTAMPTZ NOT NULL
        DEFAULT date_trunc('hour', clock_timestamp())
        CHECK (received_hour = date_trunc('hour', received_hour)),
    outcome TEXT NOT NULL CHECK (outcome IN ('default', 'usd', 'refused')),
    refusal_label TEXT CHECK (refusal_label ~ '^[a-z0-9_]{1,64}$'),
    lines_key TEXT CHECK (lines_key ~ '^[a-z0-9_.:-]{1,80}$'),
    -- Two significant figures, at most 100,000,000 (the protocol's bound).
    trace_count BIGINT CHECK (trace_count BETWEEN 0 AND 100000000),
    -- Whole MiB, at most 1,000,000,000 MiB (the protocol's bound).
    total_bytes BIGINT CHECK (
        total_bytes BETWEEN 0 AND 1048576000000000 AND total_bytes % 1048576 = 0
    ),
    app_version TEXT CHECK (app_version ~ '^[0-9A-Za-z.+-]{1,32}$'),
    platform TEXT CHECK (platform IN ('macos', 'linux', 'windows', 'other')),
    -- A refusal names its label and stores no figure or lines; a served
    -- answer names its lines and both figures.
    CHECK (
        (outcome = 'refused'
            AND refusal_label IS NOT NULL AND lines_key IS NULL
            AND trace_count IS NULL AND total_bytes IS NULL)
        OR (outcome <> 'refused'
            AND refusal_label IS NULL AND lines_key IS NOT NULL
            AND trace_count IS NOT NULL AND total_bytes IS NOT NULL)
    )
);

CREATE INDEX trace_first_run_estimate_requests_hour
    ON trace_first_run_estimate_requests (received_hour);

ALTER TABLE trace_first_run_estimate_requests ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_first_run_estimate_requests FORCE ROW LEVEL SECURITY;

CREATE POLICY trace_first_run_estimate_requests_insert
    ON trace_first_run_estimate_requests
    FOR INSERT TO trace_first_run_stats_writer
    WITH CHECK (true);

CREATE POLICY trace_first_run_estimate_requests_read
    ON trace_first_run_estimate_requests
    FOR SELECT TO trace_first_run_stats_reader
    USING (true);

REVOKE ALL ON trace_first_run_estimate_requests FROM PUBLIC;
GRANT INSERT (outcome, refusal_label, lines_key, trace_count, total_bytes, app_version, platform)
    ON trace_first_run_estimate_requests TO trace_first_run_stats_writer;
GRANT SELECT ON trace_first_run_estimate_requests TO trace_first_run_stats_reader;
