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
    -- The OPERATOR's lever, distinct from `withheld` above. `withheld` is the
    -- computed/never-computed marker and every refresh clears it, so it can
    -- not double as an off switch: an operator who set it during an incident
    -- would have it silently cleared by the next scheduled refresh. The
    -- refresh NEVER writes this column. Set it TRUE and the public endpoint
    -- publishes no figure until a human sets it back.
    suppressed         BOOLEAN     NOT NULL DEFAULT FALSE,
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

-- Roles are cluster-wide, not database-wide: on a cluster where this role
-- already exists (a second database, a recreated one) a bare CREATE ROLE
-- aborts the whole batch_execute, and since run_migrations records the
-- version only after the batch succeeds, V55 would never record itself and
-- would retry -- and fail -- on every boot. Wrapped exactly as
-- trace_login_resolver (V30) and trace_invite_registry (V42) are.
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_commons_public_read') THEN
        CREATE ROLE trace_commons_public_read NOLOGIN NOBYPASSRLS;
    END IF;
END $$;

-- `singleton` is in this list because PostgreSQL column privileges cover every
-- column a query REFERENCES, not just the ones it projects, and the public
-- read filters on `WHERE singleton = TRUE`. Omitting it denied the whole
-- table ("permission denied for table trace_register_stats") even though
-- every projected column was granted. Granting it widens nothing: it is a
-- constant TRUE on a one-row table by its own PRIMARY KEY and CHECK, and
-- carries no information a role that can read the row does not already have.
-- The alternative -- dropping the filter -- would have bought the privilege
-- at the cost of the query's correctness guard, which is the wrong trade: the
-- filter is what keeps the read right if the CHECK is ever relaxed.
GRANT SELECT (singleton, traces_accepted, contributors, points_issued, withheld, suppressed, as_of, refreshed_at)
    ON trace_register_stats TO trace_commons_public_read;

-- Nothing else grants membership in this role, so without this, Task 4's
-- `SET ROLE trace_commons_public_read` fails in production with "permission
-- denied to set role" -- the migration would have created a role nobody
-- could ever assume. GRANT ... TO CURRENT_USER makes whoever applies this
-- migration (the app's own runtime role, in every deployment) a member, and
-- is deployment-agnostic: it does not bake in a runtime-role name that
-- would vary per environment. It widens nothing: CURRENT_USER already owns
-- trace_register_stats, and the role it is joining holds only the
-- six-column SELECT above.
--
-- REQUIREMENT this leaves on the deployment: the role that actually SERVES
-- requests (runs register_stats_refresh_handler / the public read) must be
-- a member of trace_commons_public_read. On a simple deployment that is
-- automatic, because the serving role is also whatever applied this
-- migration. On managed Postgres where a separate migration-runner
-- credential applies schema changes, it is NOT automatic -- the applier and
-- the server are different roles, and `SET ROLE` fails at the first
-- request, not at migration time. See docs/operator/register-stats-role.md.
GRANT trace_commons_public_read TO CURRENT_USER;

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
