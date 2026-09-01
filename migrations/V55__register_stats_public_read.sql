-- Aggregate register facts, and the least-privileged way to read them.
--
-- The public endpoint has no tenant, so `trace_current_tenant_id()` matches
-- nothing and the ordinary predicate returns an empty set. The answer is a
-- role that may read one aggregate and nothing else -- NOT `BYPASSRLS`, NOT a
-- superuser pool, and NOT dropping FORCE on a table, each of which trades a
-- narrow read for a broad hole.

CREATE TABLE trace_register_stats (
    singleton          BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    traces_accepted    BIGINT      NOT NULL DEFAULT 0,
    contributors       BIGINT      NOT NULL DEFAULT 0,
    points_issued      BIGINT      NOT NULL DEFAULT 0,
    withheld           BOOLEAN     NOT NULL DEFAULT TRUE,
    as_of              TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    -- NULL until a refresh has actually computed this row. The endpoint
    -- publishes nothing while it is NULL: zeros would be a claim about the
    -- register that nobody made.
    refreshed_at       TIMESTAMPTZ
);

INSERT INTO trace_register_stats (singleton) VALUES (TRUE)
    ON CONFLICT DO NOTHING;

ALTER TABLE trace_register_stats ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_register_stats FORCE ROW LEVEL SECURITY;

CREATE ROLE trace_commons_public_read NOLOGIN NOBYPASSRLS;

GRANT SELECT (traces_accepted, contributors, points_issued, withheld, as_of, refreshed_at)
    ON trace_register_stats TO trace_commons_public_read;

-- Role-scoped rather than blanket: this row carries no tenant, so there is no
-- tenant predicate to write, and the grant above is what bounds the columns.
CREATE POLICY trace_register_stats_public_read
    ON trace_register_stats
    FOR SELECT
    TO trace_commons_public_read
    USING (TRUE);

-- FORCE ROW LEVEL SECURITY applies to the table owner too, so without a
-- write policy the refresh worker (running as the ordinary NOBYPASSRLS
-- runtime role every other write path uses) could never update this row --
-- not even once. No `TO` clause, matching the convention the rest of this
-- schema uses for its runtime-role policies (e.g. trace_corpus_tenant_
-- isolation): scoped by predicate, not by role, so it is visible to PUBLIC
-- but only usable by a role that already holds the underlying UPDATE grant.
-- trace_commons_public_read has no such grant (only the column-scoped SELECT
-- above), so this does not widen what that role can reach.
CREATE POLICY trace_register_stats_runtime_write
    ON trace_register_stats
    FOR UPDATE
    USING (TRUE)
    WITH CHECK (TRUE);

-- Same reasoning for reads: the runtime role needs to read the row it just
-- wrote (e.g. to check refreshed_at before publishing), and the SELECT
-- policy above is scoped to trace_commons_public_read only. This is a
-- second, equally narrow permissive policy -- it does not touch that role's
-- column-scoped grant, which is what actually bounds it.
CREATE POLICY trace_register_stats_runtime_read
    ON trace_register_stats
    FOR SELECT
    USING (TRUE);
