-- One row per trace dedup cluster: its representative's simhash and stamp,
-- and how many decisions it holds.
--
-- Why: the inline gate path clustered each new submission by reading EVERY
-- row of trace_gate_decisions, across all tenants, ordered by decided_at
-- (an order no index covers), and folding them into a representative per
-- cluster in Rust. That is a full scan and sort per submission, so the cost
-- grows with the corpus, and on the trace_gate_driver role it runs under a
-- 5s statement_timeout (V36). Past that the read fails, the caller swallows
-- the error, and every submission silently becomes a singleton.
--
-- This table holds exactly what that fold produced, kept in step by trigger,
-- so the inline path asks only for the clusters within its Hamming radius.
--
-- Definitions, unchanged from the fold it replaces:
--   * a member is a decision row with dedup_cluster_id AND dedup_simhash set;
--   * the representative is the earliest-decided member (decided_at, then
--     decision_id to make ties deterministic);
--   * member_count is the number of members, across every tenant.
-- representative_signal_version is stored raw: NULL and '' still decode to
-- the legacy stamp in exactly one place, effective_dedup_signal_version().
--
-- trace_gate_decisions stays the source of truth. The trigger recomputes an
-- affected cluster from it rather than adjusting counts, so a row that is
-- cleared, moved by the recluster pass, or deleted cannot leave the table
-- wrong. trace_dedup_cluster_rebuild() regenerates the whole table and is
-- what backfills it below.
--
-- Applied by a non-superuser CREATEROLE migrator, as V101 and V114 are: it
-- holds the owning guard just long enough to own the functions and run the
-- backfill, then relinquishes it.

DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_dedup_cluster_guard') THEN
        CREATE ROLE trace_dedup_cluster_guard NOLOGIN NOSUPERUSER NOBYPASSRLS;
    END IF;
    -- CREATEROLE cannot restate NOSUPERUSER or NOBYPASSRLS in ALTER ROLE.
    IF EXISTS (SELECT 1 FROM pg_roles
               WHERE rolname = 'trace_dedup_cluster_guard' AND (rolsuper OR rolbypassrls)) THEN
        RAISE EXCEPTION 'trace_dedup_cluster_guard must be NOSUPERUSER NOBYPASSRLS';
    END IF;
END $$;
ALTER ROLE trace_dedup_cluster_guard NOLOGIN;
GRANT USAGE ON SCHEMA public TO trace_dedup_cluster_guard;
GRANT trace_dedup_cluster_guard TO CURRENT_USER;

CREATE TABLE IF NOT EXISTS trace_dedup_cluster_representatives (
    dedup_cluster_id              UUID PRIMARY KEY,
    representative_decision_id    UUID NOT NULL,
    representative_decided_at     TIMESTAMPTZ NOT NULL,
    representative_simhash        BIGINT NOT NULL,
    representative_signal_version TEXT,
    member_count                  BIGINT NOT NULL CHECK (member_count > 0)
);

-- No tenant column and no tenant policy: a cluster spans tenants, as the
-- simhash enumeration it replaces always did. Only the guard writes it and
-- only the gate driver reads it; every other role sees no rows.
ALTER TABLE trace_dedup_cluster_representatives ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_dedup_cluster_representatives FORCE ROW LEVEL SECURITY;
REVOKE ALL ON trace_dedup_cluster_representatives FROM PUBLIC;

GRANT SELECT, INSERT, UPDATE, DELETE ON trace_dedup_cluster_representatives
    TO trace_dedup_cluster_guard;
DROP POLICY IF EXISTS trace_dedup_cluster_guard_write ON trace_dedup_cluster_representatives;
CREATE POLICY trace_dedup_cluster_guard_write ON trace_dedup_cluster_representatives
    TO trace_dedup_cluster_guard USING (TRUE) WITH CHECK (TRUE);

GRANT SELECT (dedup_cluster_id, representative_simhash, representative_signal_version, member_count)
    ON trace_dedup_cluster_representatives TO trace_gate_driver;
DROP POLICY IF EXISTS trace_gate_driver_cross_tenant_read ON trace_dedup_cluster_representatives;
CREATE POLICY trace_gate_driver_cross_tenant_read ON trace_dedup_cluster_representatives
    FOR SELECT TO trace_gate_driver USING (TRUE);

-- The guard reads only the columns that define membership and the
-- representative, across tenants, through its own permissive policy; the
-- shared tenant-isolation policy stays for everyone else.
GRANT SELECT (decision_id, decided_at, dedup_cluster_id, dedup_simhash, dedup_signal_version)
    ON trace_gate_decisions TO trace_dedup_cluster_guard;
DROP POLICY IF EXISTS trace_dedup_cluster_guard_read ON trace_gate_decisions;
CREATE POLICY trace_dedup_cluster_guard_read ON trace_gate_decisions
    FOR SELECT TO trace_dedup_cluster_guard USING (TRUE);

-- One cluster's members, in representative order. Each recompute is one
-- range scan of this index, sized by the cluster rather than the corpus.
CREATE INDEX IF NOT EXISTS trace_gate_decisions_dedup_cluster_members
    ON trace_gate_decisions (dedup_cluster_id, decided_at, decision_id)
    WHERE dedup_cluster_id IS NOT NULL AND dedup_simhash IS NOT NULL;

DROP TRIGGER IF EXISTS trace_dedup_cluster_track ON trace_gate_decisions;
DROP FUNCTION IF EXISTS trace_dedup_cluster_track();
DROP FUNCTION IF EXISTS trace_dedup_cluster_refresh(UUID);
DROP FUNCTION IF EXISTS trace_dedup_cluster_rebuild();

-- Recompute one cluster's row from its members.
--
-- The first statement is the lock. ON CONFLICT DO UPDATE takes the row lock on
-- an existing row, waiting for any other writer holding it; when there is no
-- row it inserts a placeholder that any concurrent writer for the same
-- cluster then waits on. Either way writers to one cluster, from any tenant,
-- run one at a time, and the recompute statements that follow start after
-- the lock is held, so under READ COMMITTED they see whatever the writer
-- before committed. The placeholder never outlives this function: it is
-- overwritten or deleted below, in the same transaction.
--
-- A row lock is recorded on the tuple, not in the shared lock table, so a
-- single statement that touches many clusters (deleting a large tenant, a
-- bulk backfill) holds no more lock-table slots than one that touches one.
-- A transaction-scoped advisory lock per cluster would hold one slot per
-- cluster until commit and fail past max_locks_per_transaction.
CREATE FUNCTION trace_dedup_cluster_refresh(p_cluster UUID) RETURNS VOID
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE
    v_count BIGINT;
    v_rep   RECORD;
BEGIN
    INSERT INTO public.trace_dedup_cluster_representatives AS r
        (dedup_cluster_id, representative_decision_id, representative_decided_at,
         representative_simhash, representative_signal_version, member_count)
    VALUES (p_cluster, '00000000-0000-0000-0000-000000000000', '-infinity', 0, NULL, 1)
    ON CONFLICT (dedup_cluster_id) DO UPDATE SET member_count = r.member_count;
    SELECT pg_catalog.count(*) INTO v_count
      FROM public.trace_gate_decisions g
     WHERE g.dedup_cluster_id = p_cluster AND g.dedup_simhash IS NOT NULL;
    IF v_count = 0 THEN
        DELETE FROM public.trace_dedup_cluster_representatives
         WHERE dedup_cluster_id = p_cluster;
        RETURN;
    END IF;
    SELECT g.decision_id, g.decided_at, g.dedup_simhash, g.dedup_signal_version INTO v_rep
      FROM public.trace_gate_decisions g
     WHERE g.dedup_cluster_id = p_cluster AND g.dedup_simhash IS NOT NULL
     ORDER BY g.decided_at, g.decision_id
     LIMIT 1;
    UPDATE public.trace_dedup_cluster_representatives
       SET representative_decision_id    = v_rep.decision_id,
           representative_decided_at     = v_rep.decided_at,
           representative_simhash        = v_rep.dedup_simhash,
           representative_signal_version = v_rep.dedup_signal_version,
           member_count                  = v_count
     WHERE dedup_cluster_id = p_cluster;
END $$;

CREATE FUNCTION trace_dedup_cluster_track() RETURNS TRIGGER
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE
    v_old UUID;
    v_new UUID;
BEGIN
    -- Most updates to a decision row (credit, size snapshots, status) do not
    -- touch membership or the representative; they cost one comparison.
    IF TG_OP = 'UPDATE' AND
        (OLD.decision_id, OLD.decided_at, OLD.dedup_cluster_id, OLD.dedup_simhash,
         OLD.dedup_signal_version)
        IS NOT DISTINCT FROM
        (NEW.decision_id, NEW.decided_at, NEW.dedup_cluster_id, NEW.dedup_simhash,
         NEW.dedup_signal_version)
    THEN
        RETURN NULL;
    END IF;
    IF TG_OP <> 'INSERT' THEN v_old := OLD.dedup_cluster_id; END IF;
    IF TG_OP <> 'DELETE' THEN v_new := NEW.dedup_cluster_id; END IF;
    -- Ascending cluster order, so two writers moving rows between the same
    -- pair of clusters in opposite directions cannot deadlock.
    IF v_old IS NOT NULL AND v_new IS NOT NULL AND v_old <> v_new THEN
        PERFORM public.trace_dedup_cluster_refresh(LEAST(v_old, v_new));
        PERFORM public.trace_dedup_cluster_refresh(GREATEST(v_old, v_new));
    ELSIF COALESCE(v_old, v_new) IS NOT NULL THEN
        PERFORM public.trace_dedup_cluster_refresh(COALESCE(v_old, v_new));
    END IF;
    RETURN NULL;
END $$;

-- Regenerate every row from trace_gate_decisions. The backfill below, and a
-- repair if the table is ever suspected of drifting. Returns the row count.
CREATE FUNCTION trace_dedup_cluster_rebuild() RETURNS BIGINT
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE
    v_rows BIGINT;
BEGIN
    LOCK TABLE public.trace_dedup_cluster_representatives IN EXCLUSIVE MODE;
    DELETE FROM public.trace_dedup_cluster_representatives;
    INSERT INTO public.trace_dedup_cluster_representatives
        (dedup_cluster_id, representative_decision_id, representative_decided_at,
         representative_simhash, representative_signal_version, member_count)
    SELECT DISTINCT ON (g.dedup_cluster_id)
           g.dedup_cluster_id, g.decision_id, g.decided_at, g.dedup_simhash,
           g.dedup_signal_version,
           pg_catalog.count(*) OVER (PARTITION BY g.dedup_cluster_id)
      FROM public.trace_gate_decisions g
     WHERE g.dedup_cluster_id IS NOT NULL AND g.dedup_simhash IS NOT NULL
     ORDER BY g.dedup_cluster_id, g.decided_at, g.decision_id;
    GET DIAGNOSTICS v_rows = ROW_COUNT;
    RETURN v_rows;
END $$;

GRANT CREATE ON SCHEMA public TO trace_dedup_cluster_guard;
ALTER FUNCTION trace_dedup_cluster_refresh(UUID) OWNER TO trace_dedup_cluster_guard;
ALTER FUNCTION trace_dedup_cluster_track() OWNER TO trace_dedup_cluster_guard;
ALTER FUNCTION trace_dedup_cluster_rebuild() OWNER TO trace_dedup_cluster_guard;
REVOKE CREATE ON SCHEMA public FROM trace_dedup_cluster_guard;
REVOKE ALL ON FUNCTION trace_dedup_cluster_refresh(UUID) FROM PUBLIC;
REVOKE ALL ON FUNCTION trace_dedup_cluster_track() FROM PUBLIC;
REVOKE ALL ON FUNCTION trace_dedup_cluster_rebuild() FROM PUBLIC;

CREATE TRIGGER trace_dedup_cluster_track
    AFTER INSERT OR UPDATE OR DELETE ON trace_gate_decisions
    FOR EACH ROW EXECUTE FUNCTION trace_dedup_cluster_track();

-- Backfill under the guard, whose read policy sees every tenant's rows.
SELECT trace_dedup_cluster_rebuild();

REVOKE trace_dedup_cluster_guard FROM CURRENT_USER;
