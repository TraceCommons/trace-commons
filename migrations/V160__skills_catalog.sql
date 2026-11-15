-- The public Skills catalog's working state: the repositories the crawler
-- visits, the skills it found in them with their check results, and the
-- skills apps reported as missing (POST /v1/public/skills-catalog/suggestions).
-- The operator binary `trace-commons-skills-catalog` crawls into these
-- tables and exports one signed document from them.
--
-- NUMBERING. V159 is taken by open PR #2075 (orgs). This migration depends on
-- nothing V159 adds, and the runner applies every unrecorded version in table
-- order and allows gaps.
--
-- WHAT A ROW HOLDS. Public GitHub data only: repository names, paths,
-- commits, hashes, frontmatter text, counts and dates. A suggestion row holds
-- a repository, a path, the last reported commit, the first and last *day* it
-- was reported (set by the database clock), a capped count and a status. No
-- address, account, tenant, install id or time finer than a day: two reports
-- of the same skill are one row, and a row joins to nothing.
--
-- RLS STANCE. No tenant column and no tenant policy, by design: catalog data
-- belongs to no account (as V152). Row security is enabled and forced on all
-- three tables, PUBLIC has nothing, and the only role with privileges or
-- policies is trace_skills_catalog_guard, a NOLOGIN, NOSUPERUSER,
-- NOBYPASSRLS role.
--   * The crawler runs as an operator login granted the guard
--     (GRANT trace_skills_catalog_guard TO <crawler login>) and takes it with
--     SET LOCAL ROLE for each transaction.
--   * The ingest runtime cannot read or write any table. It may only call
--     trace_skills_catalog_suggest(), a SECURITY DEFINER function owned by
--     the guard that validates one suggestion again and upserts its row,
--     refusing new rows while 10,000 are pending.
--
-- Applied by a non-superuser CREATEROLE migrator, as V152 is: it holds the
-- guard just long enough to own the function, then relinquishes it.

DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_skills_catalog_guard') THEN
        CREATE ROLE trace_skills_catalog_guard NOLOGIN NOSUPERUSER NOBYPASSRLS;
    END IF;
    -- CREATEROLE cannot restate NOSUPERUSER or NOBYPASSRLS in ALTER ROLE.
    IF EXISTS (SELECT 1 FROM pg_roles
               WHERE rolname = 'trace_skills_catalog_guard' AND (rolsuper OR rolbypassrls)) THEN
        RAISE EXCEPTION 'trace_skills_catalog_guard must be NOSUPERUSER NOBYPASSRLS';
    END IF;
END $$;
ALTER ROLE trace_skills_catalog_guard NOLOGIN;
GRANT USAGE ON SCHEMA public TO trace_skills_catalog_guard;
GRANT trace_skills_catalog_guard TO CURRENT_USER;

CREATE TABLE IF NOT EXISTS trace_skills_catalog_repositories (
    repository TEXT PRIMARY KEY
        CHECK (repository ~ '^[a-z0-9_.-]+/[a-z0-9_.-]+$' AND octet_length(repository) <= 140),
    source_kind TEXT NOT NULL
        CHECK (source_kind IN ('seed', 'list', 'marketplace', 'suggested')),
    publisher_id TEXT NOT NULL
        CHECK (publisher_id ~ '^[a-z0-9][a-z0-9._-]*$' AND octet_length(publisher_id) <= 160),
    publisher_name TEXT NOT NULL
        CHECK (octet_length(publisher_name) BETWEEN 1 AND 128),
    instructions_url TEXT NOT NULL
        CHECK (instructions_url ~ '^https://' AND octet_length(instructions_url) <= 512),
    skills_root TEXT CHECK (octet_length(skills_root) <= 512),
    head_revision TEXT CHECK (head_revision ~ '^[0-9a-f]{40}$'),
    stars BIGINT CHECK (stars >= 0),
    pushed_at TIMESTAMPTZ,
    created_at_github TIMESTAMPTZ,
    archived BOOLEAN,
    commits_7d INTEGER CHECK (commits_7d >= 0),
    commits_28d INTEGER CHECK (commits_28d >= 0),
    signals_checked_at TIMESTAMPTZ,
    crawl_outcome TEXT NOT NULL CHECK (crawl_outcome ~ '^[a-z0-9_]{1,64}$'),
    crawled_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE IF NOT EXISTS trace_skills_catalog_skills (
    skill_id TEXT PRIMARY KEY CHECK (octet_length(skill_id) <= 700),
    repository TEXT NOT NULL
        REFERENCES trace_skills_catalog_repositories (repository) ON DELETE CASCADE,
    path TEXT NOT NULL
        CHECK (path ~ '^([A-Za-z0-9_.-]+/)*SKILL\.md$'
               AND path !~ '(^|/)\.\.?(/|$)'
               AND octet_length(path) <= 512),
    publisher_id TEXT NOT NULL,
    revision TEXT NOT NULL CHECK (revision ~ '^[0-9a-f]{40}$'),
    sha256 TEXT NOT NULL CHECK (sha256 ~ '^[0-9a-f]{64}$'),
    previous_sha256 TEXT[] NOT NULL DEFAULT '{}'
        CHECK (cardinality(previous_sha256) <= 8),
    name TEXT NOT NULL CHECK (octet_length(name) <= 128),
    description TEXT NOT NULL CHECK (octet_length(description) <= 1024),
    license JSONB NOT NULL,
    checks JSONB NOT NULL,
    blocking TEXT[] NOT NULL DEFAULT '{}',
    present BOOLEAN NOT NULL DEFAULT TRUE,
    first_seen_at TIMESTAMPTZ NOT NULL,
    last_changed_at TIMESTAMPTZ NOT NULL,
    last_seen_at TIMESTAMPTZ NOT NULL,
    UNIQUE (repository, path)
);

CREATE TABLE IF NOT EXISTS trace_skills_catalog_suggestions (
    repository TEXT NOT NULL
        CHECK (repository ~ '^[a-z0-9_.-]+/[a-z0-9_.-]+$' AND octet_length(repository) <= 140),
    path TEXT NOT NULL
        CHECK (path ~ '^([A-Za-z0-9_.-]+/)*SKILL\.md$'
               AND path !~ '(^|/)\.\.?(/|$)'
               AND octet_length(path) <= 512),
    revision TEXT NOT NULL CHECK (revision ~ '^[0-9a-f]{40}$'),
    first_received_day DATE NOT NULL
        DEFAULT ((clock_timestamp() AT TIME ZONE 'UTC')::date),
    last_received_day DATE NOT NULL
        DEFAULT ((clock_timestamp() AT TIME ZONE 'UTC')::date),
    report_count BIGINT NOT NULL DEFAULT 1 CHECK (report_count BETWEEN 1 AND 1000000),
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'crawled', 'refused')),
    PRIMARY KEY (repository, path)
);

CREATE INDEX IF NOT EXISTS trace_skills_catalog_suggestions_pending
    ON trace_skills_catalog_suggestions (status, last_received_day);

ALTER TABLE trace_skills_catalog_repositories ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_skills_catalog_repositories FORCE ROW LEVEL SECURITY;
ALTER TABLE trace_skills_catalog_skills ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_skills_catalog_skills FORCE ROW LEVEL SECURITY;
ALTER TABLE trace_skills_catalog_suggestions ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_skills_catalog_suggestions FORCE ROW LEVEL SECURITY;

REVOKE ALL ON trace_skills_catalog_repositories FROM PUBLIC;
REVOKE ALL ON trace_skills_catalog_skills FROM PUBLIC;
REVOKE ALL ON trace_skills_catalog_suggestions FROM PUBLIC;
GRANT SELECT, INSERT, UPDATE, DELETE ON trace_skills_catalog_repositories TO trace_skills_catalog_guard;
GRANT SELECT, INSERT, UPDATE, DELETE ON trace_skills_catalog_skills TO trace_skills_catalog_guard;
GRANT SELECT, INSERT, UPDATE, DELETE ON trace_skills_catalog_suggestions TO trace_skills_catalog_guard;

DROP POLICY IF EXISTS trace_skills_catalog_guard_repositories ON trace_skills_catalog_repositories;
CREATE POLICY trace_skills_catalog_guard_repositories ON trace_skills_catalog_repositories
    TO trace_skills_catalog_guard USING (TRUE) WITH CHECK (TRUE);
DROP POLICY IF EXISTS trace_skills_catalog_guard_skills ON trace_skills_catalog_skills;
CREATE POLICY trace_skills_catalog_guard_skills ON trace_skills_catalog_skills
    TO trace_skills_catalog_guard USING (TRUE) WITH CHECK (TRUE);
DROP POLICY IF EXISTS trace_skills_catalog_guard_suggestions ON trace_skills_catalog_suggestions;
CREATE POLICY trace_skills_catalog_guard_suggestions ON trace_skills_catalog_suggestions
    TO trace_skills_catalog_guard USING (TRUE) WITH CHECK (TRUE);

-- One suggestion. Returns 'stored' (a new pending row), 'counted' (a row
-- for this repository and path already existed), or 'full' (10,000 rows are
-- pending; nothing written). The service validates the suggestion against the
-- protocol before calling; these checks are the backstop.
CREATE OR REPLACE FUNCTION trace_skills_catalog_suggest(
    p_repository TEXT,
    p_path       TEXT,
    p_revision   TEXT
) RETURNS TEXT
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE
    v_pending BIGINT;
    v_today DATE := (pg_catalog.clock_timestamp() AT TIME ZONE 'UTC')::date;
BEGIN
    IF p_repository IS NULL OR p_path IS NULL OR p_revision IS NULL
       OR p_repository !~ '^[a-z0-9_.-]+/[a-z0-9_.-]+$'
       OR pg_catalog.octet_length(p_repository) > 140
       OR p_path !~ '^([A-Za-z0-9_.-]+/)*SKILL\.md$'
       OR p_path ~ '(^|/)\.\.?(/|$)'
       OR pg_catalog.octet_length(p_path) > 512
       OR p_revision !~ '^[0-9a-f]{40}$' THEN
        RAISE EXCEPTION 'skills_suggestion_invalid' USING ERRCODE = '22023';
    END IF;
    UPDATE public.trace_skills_catalog_suggestions
       SET report_count = LEAST(report_count + 1, 1000000),
           last_received_day = v_today,
           revision = p_revision
     WHERE repository = p_repository AND path = p_path;
    IF FOUND THEN
        RETURN 'counted';
    END IF;
    SELECT pg_catalog.count(*) INTO v_pending
      FROM public.trace_skills_catalog_suggestions
     WHERE status = 'pending';
    IF v_pending >= 10000 THEN
        RETURN 'full';
    END IF;
    INSERT INTO public.trace_skills_catalog_suggestions (repository, path, revision)
    VALUES (p_repository, p_path, p_revision)
    ON CONFLICT (repository, path) DO UPDATE
       SET report_count = LEAST(public.trace_skills_catalog_suggestions.report_count + 1, 1000000),
           last_received_day = v_today,
           revision = EXCLUDED.revision;
    RETURN 'stored';
END $$;

GRANT CREATE ON SCHEMA public TO trace_skills_catalog_guard;
ALTER FUNCTION trace_skills_catalog_suggest(TEXT, TEXT, TEXT)
    OWNER TO trace_skills_catalog_guard;
REVOKE CREATE ON SCHEMA public FROM trace_skills_catalog_guard;
REVOKE ALL ON FUNCTION trace_skills_catalog_suggest(TEXT, TEXT, TEXT) FROM PUBLIC;
-- The runtime, and the migrator for a deployment whose ingest login is the
-- migrator (V149's reason).
GRANT EXECUTE ON FUNCTION trace_skills_catalog_suggest(TEXT, TEXT, TEXT)
    TO trace_ingest_runtime;
GRANT EXECUTE ON FUNCTION trace_skills_catalog_suggest(TEXT, TEXT, TEXT)
    TO CURRENT_USER;

REVOKE trace_skills_catalog_guard FROM CURRENT_USER;
