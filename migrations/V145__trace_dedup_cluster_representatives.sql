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
-- trace_gate_decisions stays the source of truth. Statement-level triggers
-- read what each statement changed (its transition tables), adjust each
-- touched cluster's member_count by the difference, and look the
-- representative up again from trace_gate_decisions only when the old one
-- left the cluster. trace_dedup_cluster_rebuild() regenerates the whole table
-- and is what backfills it below.
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

-- One cluster's members, in representative order. Finding a cluster's
-- earliest member again is one probe of this index.
CREATE INDEX IF NOT EXISTS trace_gate_decisions_dedup_cluster_members
    ON trace_gate_decisions (dedup_cluster_id, decided_at, decision_id)
    WHERE dedup_cluster_id IS NOT NULL AND dedup_simhash IS NOT NULL;

DROP TRIGGER IF EXISTS trace_dedup_cluster_track_insert ON trace_gate_decisions;
DROP TRIGGER IF EXISTS trace_dedup_cluster_track_update ON trace_gate_decisions;
DROP TRIGGER IF EXISTS trace_dedup_cluster_track_delete ON trace_gate_decisions;
DROP TRIGGER IF EXISTS trace_dedup_cluster_track_truncate ON trace_gate_decisions;
DROP FUNCTION IF EXISTS trace_dedup_cluster_track();
DROP FUNCTION IF EXISTS trace_dedup_cluster_truncate();
DROP FUNCTION IF EXISTS trace_dedup_cluster_apply(UUID, BIGINT, UUID[], UUID, TIMESTAMPTZ, BIGINT, TEXT);
DROP FUNCTION IF EXISTS trace_dedup_cluster_rebuild();

-- Apply one statement's change to one cluster: p_delta members joined (net),
-- p_removed are the decision ids that left or changed, and p_add_* is the
-- earliest member the statement added or changed, if any.
--
-- Locking. SELECT ... FOR UPDATE takes the cluster row's lock without writing
-- a row version. When there is no row, the insert creates it, and a concurrent
-- writer creating the same cluster waits on that insert, then finds the row
-- and loops round to lock it. Either way writers to one cluster, from any
-- tenant, apply their deltas one at a time. A row lock lives on the tuple,
-- not in the shared lock table, so a statement touching many clusters cannot
-- run out of lock slots.
--
-- Isolation. Every call that finds the row writes it. So under REPEATABLE
-- READ or SERIALIZABLE, where this transaction's snapshot can predate another
-- writer's commit to the same cluster, the FOR UPDATE (or the insert's
-- conflict check) raises a serialization failure instead of building on a
-- stale count or a stale representative. Under READ COMMITTED it sees the
-- committed row and is correct.
--
-- Cost. The representative only changes when the old one left or an earlier
-- member arrived, so the common case reads and writes one row. Only when the
-- representative left is it looked up again, by one probe of the members
-- index; the members are recounted only if the count reached zero while
-- members remain, which the triggers never produce.
CREATE FUNCTION trace_dedup_cluster_apply(
    p_cluster     UUID,
    p_delta       BIGINT,
    p_removed     UUID[],
    p_add_id      UUID,
    p_add_at      TIMESTAMPTZ,
    p_add_simhash BIGINT,
    p_add_version TEXT
) RETURNS VOID
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE
    r     public.trace_dedup_cluster_representatives%ROWTYPE;
    v_count BIGINT;
    v_id  UUID;
    v_at  TIMESTAMPTZ;
    v_sh  BIGINT;
    v_ver TEXT;
BEGIN
    LOOP
        SELECT * INTO r FROM public.trace_dedup_cluster_representatives
         WHERE dedup_cluster_id = p_cluster
         FOR UPDATE;
        IF FOUND THEN
            v_count := r.member_count + p_delta;
            IF r.representative_decision_id = ANY (p_removed) OR v_count <= 0 THEN
                SELECT g.decision_id, g.decided_at, g.dedup_simhash, g.dedup_signal_version
                  INTO v_id, v_at, v_sh, v_ver
                  FROM public.trace_gate_decisions g
                 WHERE g.dedup_cluster_id = p_cluster AND g.dedup_simhash IS NOT NULL
                 ORDER BY g.decided_at, g.decision_id
                 LIMIT 1;
                IF NOT FOUND THEN
                    DELETE FROM public.trace_dedup_cluster_representatives
                     WHERE dedup_cluster_id = p_cluster;
                    RETURN;
                END IF;
                IF v_count <= 0 THEN
                    SELECT pg_catalog.count(*) INTO v_count
                      FROM public.trace_gate_decisions g
                     WHERE g.dedup_cluster_id = p_cluster AND g.dedup_simhash IS NOT NULL;
                END IF;
            ELSIF p_add_id IS NOT NULL
                  AND (p_add_at, p_add_id)
                      < (r.representative_decided_at, r.representative_decision_id) THEN
                v_id := p_add_id; v_at := p_add_at;
                v_sh := p_add_simhash; v_ver := p_add_version;
            ELSE
                v_id := r.representative_decision_id; v_at := r.representative_decided_at;
                v_sh := r.representative_simhash; v_ver := r.representative_signal_version;
            END IF;
            UPDATE public.trace_dedup_cluster_representatives
               SET representative_decision_id    = v_id,
                   representative_decided_at     = v_at,
                   representative_simhash        = v_sh,
                   representative_signal_version = v_ver,
                   member_count                  = v_count
             WHERE dedup_cluster_id = p_cluster;
            RETURN;
        END IF;
        -- No row: the cluster had no members before this statement.
        IF p_delta <= 0 OR p_add_id IS NULL THEN
            RETURN;
        END IF;
        INSERT INTO public.trace_dedup_cluster_representatives
            (dedup_cluster_id, representative_decision_id, representative_decided_at,
             representative_simhash, representative_signal_version, member_count)
        VALUES (p_cluster, p_add_id, p_add_at, p_add_simhash, p_add_version, p_delta)
        ON CONFLICT (dedup_cluster_id) DO NOTHING;
        IF FOUND THEN
            RETURN;
        END IF;
        -- Another writer created the row and committed first: lock it and
        -- apply this statement on top.
    END LOOP;
END $$;

-- One call per statement, not per row. A member is a row with both
-- dedup_cluster_id and dedup_simhash set; an updated row counts as leaving
-- its old cluster and joining its new one unless none of the columns that
-- define membership or the representative changed. Clusters are visited in
-- ascending order, so two statements touching the same clusters lock them in
-- the same order and cannot deadlock on each other. (Two statements in one
-- transaction are still locked statement by statement.)
CREATE FUNCTION trace_dedup_cluster_track() RETURNS TRIGGER
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE
    c RECORD;
BEGIN
    IF TG_OP = 'INSERT' THEN
        FOR c IN
            WITH ch AS (
                SELECT n.dedup_cluster_id AS cluster, 1 AS s, n.decision_id, n.decided_at,
                       n.dedup_simhash AS simhash, n.dedup_signal_version AS ver
                  FROM dedup_new n
                 WHERE n.dedup_cluster_id IS NOT NULL AND n.dedup_simhash IS NOT NULL
            )
            SELECT a.cluster, a.delta, a.removed, b.decision_id, b.decided_at, b.simhash, b.ver
              FROM (SELECT ch.cluster, pg_catalog.sum(ch.s)::BIGINT AS delta,
                           '{}'::UUID[] AS removed
                      FROM ch GROUP BY ch.cluster) a
              JOIN (SELECT DISTINCT ON (ch.cluster) ch.cluster, ch.decision_id, ch.decided_at,
                           ch.simhash, ch.ver
                      FROM ch ORDER BY ch.cluster, ch.decided_at, ch.decision_id) b
                ON b.cluster = a.cluster
             ORDER BY a.cluster
        LOOP
            PERFORM public.trace_dedup_cluster_apply(c.cluster, c.delta, c.removed,
                c.decision_id, c.decided_at, c.simhash, c.ver);
        END LOOP;
    ELSIF TG_OP = 'DELETE' THEN
        FOR c IN
            SELECT o.dedup_cluster_id AS cluster, -pg_catalog.count(*) AS delta,
                   pg_catalog.array_agg(o.decision_id) AS removed
              FROM dedup_old o
             WHERE o.dedup_cluster_id IS NOT NULL AND o.dedup_simhash IS NOT NULL
             GROUP BY o.dedup_cluster_id
             ORDER BY o.dedup_cluster_id
        LOOP
            PERFORM public.trace_dedup_cluster_apply(c.cluster, c.delta, c.removed,
                NULL, NULL, NULL, NULL);
        END LOOP;
    ELSE
        FOR c IN
            WITH unchanged AS (
                SELECT o.tenant_id, o.decision_id
                  FROM dedup_old o
                  JOIN dedup_new n
                    ON n.tenant_id = o.tenant_id AND n.decision_id = o.decision_id
                 WHERE (o.decided_at, o.dedup_cluster_id, o.dedup_simhash, o.dedup_signal_version)
                       IS NOT DISTINCT FROM
                       (n.decided_at, n.dedup_cluster_id, n.dedup_simhash, n.dedup_signal_version)
            ),
            ch AS (
                SELECT o.dedup_cluster_id AS cluster, -1 AS s, o.decision_id, o.decided_at,
                       o.dedup_simhash AS simhash, o.dedup_signal_version AS ver
                  FROM dedup_old o
                 WHERE o.dedup_cluster_id IS NOT NULL AND o.dedup_simhash IS NOT NULL
                   AND NOT EXISTS (SELECT 1 FROM unchanged u
                                    WHERE u.tenant_id = o.tenant_id
                                      AND u.decision_id = o.decision_id)
                UNION ALL
                SELECT n.dedup_cluster_id, 1, n.decision_id, n.decided_at,
                       n.dedup_simhash, n.dedup_signal_version
                  FROM dedup_new n
                 WHERE n.dedup_cluster_id IS NOT NULL AND n.dedup_simhash IS NOT NULL
                   AND NOT EXISTS (SELECT 1 FROM unchanged u
                                    WHERE u.tenant_id = n.tenant_id
                                      AND u.decision_id = n.decision_id)
            )
            SELECT a.cluster, a.delta, a.removed, b.decision_id, b.decided_at, b.simhash, b.ver
              FROM (SELECT ch.cluster, pg_catalog.sum(ch.s)::BIGINT AS delta,
                           COALESCE(pg_catalog.array_agg(ch.decision_id) FILTER (WHERE ch.s < 0),
                                    '{}'::UUID[]) AS removed
                      FROM ch GROUP BY ch.cluster) a
              LEFT JOIN (SELECT DISTINCT ON (ch.cluster) ch.cluster, ch.decision_id,
                                ch.decided_at, ch.simhash, ch.ver
                           FROM ch WHERE ch.s > 0
                          ORDER BY ch.cluster, ch.decided_at, ch.decision_id) b
                ON b.cluster = a.cluster
             ORDER BY a.cluster
        LOOP
            PERFORM public.trace_dedup_cluster_apply(c.cluster, c.delta, c.removed,
                c.decision_id, c.decided_at, c.simhash, c.ver);
        END LOOP;
    END IF;
    RETURN NULL;
END $$;

-- Row triggers do not fire on TRUNCATE, so it gets its own: an emptied
-- trace_gate_decisions leaves no clusters behind. (The runtime roles hold no
-- TRUNCATE on it; this is for an owner's reset or a restore script.)
CREATE FUNCTION trace_dedup_cluster_truncate() RETURNS TRIGGER
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
BEGIN
    DELETE FROM public.trace_dedup_cluster_representatives;
    RETURN NULL;
END $$;

-- Regenerate every row from trace_gate_decisions. The backfill below, and the
-- repair if the table is ever suspected of drifting. Only the guard (and a
-- superuser) can run it: a repair is a new migration that, as this one does,
-- grants itself the guard, calls this, and gives the guard back. Returns the
-- row count.
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
ALTER FUNCTION trace_dedup_cluster_apply(UUID, BIGINT, UUID[], UUID, TIMESTAMPTZ, BIGINT, TEXT)
    OWNER TO trace_dedup_cluster_guard;
ALTER FUNCTION trace_dedup_cluster_track() OWNER TO trace_dedup_cluster_guard;
ALTER FUNCTION trace_dedup_cluster_truncate() OWNER TO trace_dedup_cluster_guard;
ALTER FUNCTION trace_dedup_cluster_rebuild() OWNER TO trace_dedup_cluster_guard;
REVOKE CREATE ON SCHEMA public FROM trace_dedup_cluster_guard;
REVOKE ALL ON FUNCTION trace_dedup_cluster_apply(UUID, BIGINT, UUID[], UUID, TIMESTAMPTZ, BIGINT, TEXT)
    FROM PUBLIC;
REVOKE ALL ON FUNCTION trace_dedup_cluster_track() FROM PUBLIC;
REVOKE ALL ON FUNCTION trace_dedup_cluster_truncate() FROM PUBLIC;
REVOKE ALL ON FUNCTION trace_dedup_cluster_rebuild() FROM PUBLIC;

-- Transition tables need one trigger per event.
CREATE TRIGGER trace_dedup_cluster_track_insert
    AFTER INSERT ON trace_gate_decisions
    REFERENCING NEW TABLE AS dedup_new
    FOR EACH STATEMENT EXECUTE FUNCTION trace_dedup_cluster_track();
CREATE TRIGGER trace_dedup_cluster_track_update
    AFTER UPDATE ON trace_gate_decisions
    REFERENCING OLD TABLE AS dedup_old NEW TABLE AS dedup_new
    FOR EACH STATEMENT EXECUTE FUNCTION trace_dedup_cluster_track();
CREATE TRIGGER trace_dedup_cluster_track_delete
    AFTER DELETE ON trace_gate_decisions
    REFERENCING OLD TABLE AS dedup_old
    FOR EACH STATEMENT EXECUTE FUNCTION trace_dedup_cluster_track();
CREATE TRIGGER trace_dedup_cluster_track_truncate
    AFTER TRUNCATE ON trace_gate_decisions
    FOR EACH STATEMENT EXECUTE FUNCTION trace_dedup_cluster_truncate();

-- Backfill under the guard, whose read policy sees every tenant's rows.
SELECT trace_dedup_cluster_rebuild();

REVOKE trace_dedup_cluster_guard FROM CURRENT_USER;
