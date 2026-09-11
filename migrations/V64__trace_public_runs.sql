-- Explicitly reviewed public excerpts derived from owned, accepted traces.
--
-- A public run is independent from contributor enrollment and profile
-- attribution. The account session owns the publication; the approved digest
-- covers the exact public fields; evidence keeps its source event id only for
-- server-side provenance checks. Public reads expose neither account,
-- submission, tenant, principal, nor event identifiers.

CREATE TABLE trace_public_runs (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    publication_id UUID NOT NULL,
    account_id UUID NOT NULL,
    submission_id UUID NOT NULL,
    slug TEXT NOT NULL CHECK (
        slug ~ '^[a-z0-9]([a-z0-9-]{0,62}[a-z0-9])?$'
    ),
    title TEXT NOT NULL CHECK (char_length(title) BETWEEN 1 AND 100),
    outcome_summary TEXT NOT NULL CHECK (char_length(outcome_summary) BETWEEN 1 AND 600),
    correction_excerpt TEXT CHECK (char_length(correction_excerpt) BETWEEN 1 AND 1000),
    workflow TEXT NOT NULL CHECK (char_length(workflow) BETWEEN 1 AND 4000),
    reuse_permission TEXT NOT NULL CHECK (reuse_permission IN ('cc_by_4_0', 'cc0_1_0')),
    evidence_jsonb JSONB NOT NULL CHECK (
        jsonb_typeof(evidence_jsonb) = 'array'
        AND jsonb_array_length(evidence_jsonb) BETWEEN 1 AND 4
    ),
    task_success TEXT NOT NULL CHECK (task_success IN ('success', 'partial', 'failure', 'unknown')),
    contributed_version TEXT NOT NULL CHECK (char_length(contributed_version) BETWEEN 1 AND 128),
    approval_sha256 TEXT NOT NULL CHECK (approval_sha256 ~ '^sha256:[0-9a-f]{64}$'),
    source_publication_id UUID,
    version INTEGER NOT NULL DEFAULT 1 CHECK (version > 0),
    published_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    unpublished_at TIMESTAMPTZ,
    PRIMARY KEY (tenant_id, publication_id),
    UNIQUE (publication_id),
    UNIQUE (slug),
    UNIQUE (tenant_id, submission_id),
    FOREIGN KEY (tenant_id, account_id)
        REFERENCES trace_accounts(tenant_id, account_id) ON DELETE CASCADE,
    FOREIGN KEY (tenant_id, submission_id)
        REFERENCES trace_submissions(tenant_id, submission_id) ON DELETE CASCADE,
    FOREIGN KEY (source_publication_id)
        REFERENCES trace_public_runs(publication_id) ON DELETE SET NULL
);

CREATE INDEX idx_trace_public_runs_source
    ON trace_public_runs (source_publication_id, published_at DESC)
    WHERE unpublished_at IS NULL;

CREATE INDEX idx_trace_public_runs_account
    ON trace_public_runs (tenant_id, account_id);

ALTER TABLE trace_public_runs ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_public_runs FORCE ROW LEVEL SECURITY;

DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_public_runs;
CREATE POLICY trace_corpus_tenant_isolation ON trace_public_runs
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

-- Cross-tenant reads run only as this unassumable function-owner role. The
-- application role receives EXECUTE on two bounded functions and no role
-- membership, so it cannot issue its own raw cross-tenant SELECT.
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_public_run_reader') THEN
        CREATE ROLE trace_public_run_reader NOLOGIN NOBYPASSRLS;
    END IF;
END $$;

GRANT USAGE ON SCHEMA public TO trace_public_run_reader;
GRANT SELECT (
    publication_id, slug, title, outcome_summary, correction_excerpt,
    workflow, reuse_permission, evidence_jsonb, task_success,
    contributed_version, version, published_at, source_publication_id,
    unpublished_at
) ON trace_public_runs TO trace_public_run_reader;

DROP POLICY IF EXISTS trace_public_run_public_read ON trace_public_runs;
CREATE POLICY trace_public_run_public_read ON trace_public_runs
    FOR SELECT
    TO trace_public_run_reader
    USING (unpublished_at IS NULL);

-- Provenance validation needs identifier-only visibility across tenants and
-- across inactive ancestors. The runtime cannot assume this role or query the
-- table through it; it can execute only the boolean graph check below.
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_public_run_graph_guard') THEN
        CREATE ROLE trace_public_run_graph_guard NOLOGIN NOBYPASSRLS;
    END IF;
END $$;

GRANT USAGE ON SCHEMA public TO trace_public_run_graph_guard;
GRANT SELECT (tenant_id, account_id, submission_id, publication_id,
    source_publication_id, slug)
    ON trace_public_runs TO trace_public_run_graph_guard;

CREATE POLICY trace_public_run_graph_read ON trace_public_runs
    FOR SELECT
    TO trace_public_run_graph_guard
    USING (TRUE);

CREATE OR REPLACE FUNCTION trace_public_run_would_cycle(
    candidate_publication_id UUID,
    requested_source_id UUID
)
RETURNS BOOLEAN
LANGUAGE SQL
STABLE
SECURITY DEFINER
SET search_path = pg_catalog, public
AS $$
    WITH RECURSIVE ancestry(publication_id, source_publication_id) AS (
        SELECT run.publication_id, run.source_publication_id
        FROM trace_public_runs AS run
        WHERE run.publication_id = requested_source_id
        UNION
        SELECT parent.publication_id, parent.source_publication_id
        FROM trace_public_runs AS parent
        JOIN ancestry AS child
          ON parent.publication_id = child.source_publication_id
    )
    SELECT EXISTS (
        SELECT 1 FROM ancestry
        WHERE publication_id = candidate_publication_id
    );
$$;

CREATE OR REPLACE FUNCTION trace_public_run_retained_source(
    owner_tenant_id TEXT,
    owner_account_id UUID,
    owner_submission_id UUID
)
RETURNS TABLE (retained_source_slug TEXT)
LANGUAGE SQL
STABLE
SECURITY DEFINER
SET search_path = pg_catalog, public
SET trace_commons.trace_tenant_id = ''
AS $$
    SELECT source_run.slug
    FROM trace_public_runs AS owner_run
    JOIN trace_public_runs AS source_run
      ON source_run.publication_id = owner_run.source_publication_id
    WHERE owner_run.tenant_id = owner_tenant_id
      AND owner_run.account_id = owner_account_id
      AND owner_run.submission_id = owner_submission_id;
$$;

CREATE OR REPLACE FUNCTION trace_public_run_page(
    requested_slug TEXT,
    variation_limit INTEGER
)
RETURNS TABLE (
    slug TEXT,
    title TEXT,
    outcome_summary TEXT,
    correction_excerpt TEXT,
    workflow TEXT,
    reuse_permission TEXT,
    evidence JSONB,
    task_success TEXT,
    contributed_version TEXT,
    version INTEGER,
    published_at TIMESTAMPTZ,
    source JSONB,
    source_unavailable BOOLEAN,
    variations JSONB
)
LANGUAGE SQL
STABLE
SECURITY DEFINER
SET search_path = pg_catalog, public
SET trace_commons.trace_tenant_id = ''
AS $$
    SELECT
        run.slug,
        run.title,
        run.outcome_summary,
        run.correction_excerpt,
        run.workflow,
        run.reuse_permission,
        (
            SELECT COALESCE(
                jsonb_agg(
                    jsonb_build_object('excerpt', item.value ->> 'excerpt')
                    ORDER BY item.ordinality
                ),
                '[]'::jsonb
            )
            FROM jsonb_array_elements(run.evidence_jsonb)
                WITH ORDINALITY AS item(value, ordinality)
        ),
        run.task_success,
        run.contributed_version,
        run.version,
        run.published_at,
        CASE WHEN source_run.publication_id IS NULL THEN NULL ELSE
            jsonb_build_object('slug', source_run.slug, 'title', source_run.title)
        END,
        run.source_publication_id IS NOT NULL AND source_run.publication_id IS NULL,
        (
            SELECT COALESCE(
                jsonb_agg(
                    jsonb_build_object('slug', variation.slug, 'title', variation.title)
                    ORDER BY variation.published_at DESC
                ),
                '[]'::jsonb
            )
            FROM (
                SELECT child.slug, child.title, child.published_at
                FROM trace_public_runs AS child
                WHERE child.source_publication_id = run.publication_id
                ORDER BY child.published_at DESC
                LIMIT LEAST(GREATEST(variation_limit, 1), 20)
            ) AS variation
        )
    FROM trace_public_runs AS run
    LEFT JOIN trace_public_runs AS source_run
        ON source_run.publication_id = run.source_publication_id
    WHERE run.slug = requested_slug;
$$;

CREATE OR REPLACE FUNCTION trace_resolve_public_run_source(requested_slug TEXT)
RETURNS TABLE (publication_id UUID, slug TEXT, title TEXT)
LANGUAGE SQL
STABLE
SECURITY DEFINER
SET search_path = pg_catalog, public
SET trace_commons.trace_tenant_id = ''
AS $$
    SELECT run.publication_id, run.slug, run.title
    FROM trace_public_runs AS run
    WHERE run.slug = requested_slug;
$$;

GRANT trace_public_run_reader TO CURRENT_USER;
ALTER FUNCTION trace_public_run_page(TEXT, INTEGER) OWNER TO trace_public_run_reader;
ALTER FUNCTION trace_resolve_public_run_source(TEXT) OWNER TO trace_public_run_reader;
REVOKE trace_public_run_reader FROM CURRENT_USER;

GRANT trace_public_run_graph_guard TO CURRENT_USER;
ALTER FUNCTION trace_public_run_would_cycle(UUID, UUID)
    OWNER TO trace_public_run_graph_guard;
ALTER FUNCTION trace_public_run_retained_source(TEXT, UUID, UUID)
    OWNER TO trace_public_run_graph_guard;
REVOKE trace_public_run_graph_guard FROM CURRENT_USER;

REVOKE ALL ON FUNCTION trace_public_run_page(TEXT, INTEGER) FROM PUBLIC;
REVOKE ALL ON FUNCTION trace_resolve_public_run_source(TEXT) FROM PUBLIC;
REVOKE ALL ON FUNCTION trace_public_run_would_cycle(UUID, UUID) FROM PUBLIC;
REVOKE ALL ON FUNCTION trace_public_run_retained_source(TEXT, UUID, UUID) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION trace_public_run_page(TEXT, INTEGER) TO CURRENT_USER;
GRANT EXECUTE ON FUNCTION trace_resolve_public_run_source(TEXT) TO CURRENT_USER;
GRANT EXECUTE ON FUNCTION trace_public_run_would_cycle(UUID, UUID) TO CURRENT_USER;
GRANT EXECUTE ON FUNCTION trace_public_run_retained_source(TEXT, UUID, UUID) TO CURRENT_USER;

-- Every transition away from accepted revokes the derived public page in the
-- same transaction, regardless of which application path changed the status.
CREATE OR REPLACE FUNCTION trace_unpublish_run_when_submission_leaves_accepted()
RETURNS TRIGGER
LANGUAGE plpgsql
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

DROP TRIGGER IF EXISTS trace_submission_status_unpublishes_run ON trace_submissions;
CREATE TRIGGER trace_submission_status_unpublishes_run
AFTER UPDATE OF status ON trace_submissions
FOR EACH ROW
WHEN (OLD.status IS DISTINCT FROM NEW.status)
EXECUTE FUNCTION trace_unpublish_run_when_submission_leaves_accepted();
