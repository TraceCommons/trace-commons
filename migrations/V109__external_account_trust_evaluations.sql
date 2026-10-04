-- Compact external evaluation contract. Existing shadow recording remains available.
ALTER TABLE trace_account_trust_evaluations
    ALTER COLUMN units DROP NOT NULL,
    ALTER COLUMN units_capped DROP NOT NULL,
    ALTER COLUMN active_weeks DROP NOT NULL,
    ALTER COLUMN age_weeks DROP NOT NULL,
    ALTER COLUMN netted_withdrawn DROP NOT NULL,
    ALTER COLUMN netted_revoked DROP NOT NULL,
    ALTER COLUMN netted_quarantined DROP NOT NULL,
    ALTER COLUMN penalty_active DROP NOT NULL;

DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname='trace_account_trust_evaluator') THEN
        CREATE ROLE trace_account_trust_evaluator NOLOGIN NOSUPERUSER NOBYPASSRLS;
    END IF;
    -- CREATEROLE cannot restate NOSUPERUSER or NOBYPASSRLS in ALTER ROLE.
    -- Refuse an elevated existing role instead of relying on a superuser to
    -- normalize it: these roles must never bypass the tenant policies.
    IF EXISTS (SELECT 1 FROM pg_roles
               WHERE rolname = 'trace_account_trust_evaluator' AND (rolsuper OR rolbypassrls)) THEN
        RAISE EXCEPTION 'trace_account_trust_evaluator must be NOSUPERUSER NOBYPASSRLS';
    END IF;
END $$;
ALTER ROLE trace_account_trust_evaluator NOLOGIN;
GRANT USAGE ON SCHEMA public TO trace_account_trust_evaluator;
-- Read seams only: this role is deliberately not a trust worker or fact recorder.
GRANT EXECUTE ON FUNCTION trace_account_trust_worker_accounts(TEXT,UUID,BIGINT),
    trace_account_trust_evaluation_inputs(TEXT,UUID) TO trace_account_trust_evaluator;

GRANT trace_account_trust_evaluation_guard TO CURRENT_USER;

-- A coherent frontier invalidates evaluations when recorded inputs or identity change.
-- No login gets table write privileges. Trigger rows come from authoritative tables.
CREATE TABLE trace_account_trust_frontiers (
    tenant_id TEXT NOT NULL,
    account_id UUID NOT NULL,
    generation BIGINT NOT NULL DEFAULT 0 CHECK (generation >= 0),
    PRIMARY KEY (tenant_id,account_id)
);
INSERT INTO trace_account_trust_frontiers(tenant_id,account_id)
    SELECT tenant_id,account_id FROM trace_accounts;
ALTER TABLE trace_account_trust_frontiers ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_account_trust_frontiers FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_account_trust_frontiers;
CREATE POLICY trace_corpus_tenant_isolation ON trace_account_trust_frontiers
    USING (tenant_id=trace_current_tenant_id()) WITH CHECK (tenant_id=trace_current_tenant_id());
-- Definer triggers must also work for operator writes and retention without a tenant GUC.
CREATE POLICY account_trust_frontier_guard ON trace_account_trust_frontiers
    TO trace_account_trust_evaluation_guard USING (TRUE) WITH CHECK (TRUE);
GRANT SELECT,INSERT,UPDATE ON trace_account_trust_frontiers TO trace_account_trust_evaluation_guard;
GRANT SELECT(tenant_id,account_id,closed_at),UPDATE(created_at) ON trace_accounts TO trace_account_trust_evaluation_guard;
ALTER TABLE trace_account_trust_evaluations ADD COLUMN input_generation BIGINT CHECK (input_generation >= 0);

CREATE FUNCTION trace_account_trust_advance_frontier() RETURNS TRIGGER
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
BEGIN
    IF TG_TABLE_NAME='trace_account_trust_facts' THEN
        PERFORM public.trace_account_trust_lock_fact_dependencies(
            CASE WHEN TG_OP<>'INSERT' THEN OLD.tenant_id END,
            CASE WHEN TG_OP<>'INSERT' THEN OLD.submission_id END,
            CASE WHEN TG_OP<>'DELETE' THEN NEW.tenant_id END,
            CASE WHEN TG_OP<>'DELETE' THEN NEW.submission_id END);
    END IF;
        IF TG_OP <> 'INSERT' THEN
            INSERT INTO public.trace_account_trust_frontiers(tenant_id,account_id,generation)
            VALUES(OLD.tenant_id,OLD.account_id,1)
            ON CONFLICT(tenant_id,account_id) DO UPDATE SET generation=trace_account_trust_frontiers.generation+1;
        END IF;
        IF TG_OP='INSERT' OR (TG_OP='UPDATE' AND (OLD.tenant_id,OLD.account_id) IS DISTINCT FROM (NEW.tenant_id,NEW.account_id)) THEN
            INSERT INTO public.trace_account_trust_frontiers(tenant_id,account_id,generation)
            VALUES(NEW.tenant_id,NEW.account_id,CASE WHEN TG_TABLE_NAME='trace_accounts' THEN 0 ELSE 1 END)
            ON CONFLICT(tenant_id,account_id) DO UPDATE SET generation=trace_account_trust_frontiers.generation+1;
        END IF;
    RETURN NULL;
END $$;
CREATE TRIGGER account_trust_fact_frontier AFTER INSERT OR UPDATE OR DELETE ON trace_account_trust_facts
    FOR EACH ROW EXECUTE FUNCTION trace_account_trust_advance_frontier();
CREATE TRIGGER account_trust_principal_frontier AFTER INSERT OR UPDATE OR DELETE ON trace_account_principals
    FOR EACH ROW EXECUTE FUNCTION trace_account_trust_advance_frontier();
CREATE TRIGGER account_trust_account_frontier AFTER INSERT OR UPDATE OR DELETE ON trace_accounts
    FOR EACH ROW EXECUTE FUNCTION trace_account_trust_advance_frontier();

CREATE FUNCTION trace_account_trust_input_generation(p_tenant TEXT,p_account UUID) RETURNS BIGINT
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
    SELECT generation FROM public.trace_account_trust_frontiers
    WHERE tenant_id=p_tenant AND account_id=p_account AND p_tenant=public.trace_current_tenant_id();
$$;
CREATE FUNCTION trace_account_trust_lock_input_generation(p_tenant TEXT,p_account UUID) RETURNS BIGINT
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE v_generation BIGINT;
BEGIN
    IF p_tenant IS DISTINCT FROM public.trace_current_tenant_id() THEN
        RAISE EXCEPTION USING MESSAGE='account_trust_evaluation_unauthorized';
    END IF;
    SELECT generation INTO v_generation FROM public.trace_account_trust_frontiers
    WHERE tenant_id=p_tenant AND account_id=p_account FOR SHARE;
    RETURN v_generation;
END $$;
CREATE FUNCTION trace_record_external_account_trust_evaluation(
    p_tenant TEXT, p_evaluation UUID, p_account UUID, p_policy_version TEXT, p_mode TEXT,
    p_as_of TIMESTAMPTZ, p_tier INTEGER, p_effective_allowance BIGINT, p_digest TEXT,
    p_expected_generation BIGINT)
RETURNS BOOLEAN
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE v_previous INTEGER; v_generation BIGINT;
BEGIN
    IF p_tenant IS NULL OR p_tenant IS DISTINCT FROM public.trace_current_tenant_id() THEN
        RAISE EXCEPTION USING MESSAGE='account_trust_evaluation_unauthorized';
    END IF;
    IF p_mode IS NULL OR p_mode NOT IN ('shadow','applied') OR p_as_of IS NULL
       OR NOT pg_catalog.isfinite(p_as_of) OR p_as_of > pg_catalog.transaction_timestamp() THEN
        RAISE EXCEPTION USING MESSAGE='account_trust_evaluation_invalid';
    END IF;
    -- Match admission's account-before-frontier order; also refuse closed accounts.
    PERFORM 1 FROM public.trace_accounts
     WHERE tenant_id=p_tenant AND account_id=p_account AND closed_at IS NULL FOR SHARE;
    IF NOT FOUND THEN
        RAISE EXCEPTION USING MESSAGE='account_trust_evaluation_unauthorized';
    END IF;
    SELECT generation INTO v_generation FROM public.trace_account_trust_frontiers
     WHERE tenant_id=p_tenant AND account_id=p_account FOR UPDATE;
    IF v_generation IS NULL OR p_expected_generation IS DISTINCT FROM v_generation THEN
        RAISE EXCEPTION USING MESSAGE='account_trust_evaluation_inputs_changed';
    END IF;
    PERFORM pg_catalog.pg_advisory_xact_lock(
        pg_catalog.hashtextextended('trace-account-trust-evaluation:' || p_tenant || ':' || p_account::text, 0));
    SELECT e.tier INTO v_previous FROM public.trace_account_trust_evaluations e
     WHERE e.tenant_id=p_tenant AND e.account_id=p_account
       AND e.growth_policy_version=p_policy_version AND e.mode=p_mode
     ORDER BY e.as_of DESC, e.recorded_at DESC, e.evaluation_id DESC LIMIT 1;
    INSERT INTO public.trace_account_trust_evaluations(tenant_id,evaluation_id,account_id,
        growth_policy_version,mode,as_of,tier,previous_tier,effective_allowance,facts_digest,input_generation)
    VALUES(p_tenant,p_evaluation,p_account,p_policy_version,p_mode,p_as_of,p_tier,
        v_previous,p_effective_allowance,p_digest,v_generation);
    IF p_tier IS DISTINCT FROM COALESCE(v_previous,0) THEN
        INSERT INTO public.trace_account_audit(tenant_id,action,actor_ref,outcome,safe_metadata)
        VALUES(p_tenant,'account_trust_tier_changed','account-actor:' || p_account::text,p_mode,
            pg_catalog.jsonb_build_object('from_tier',COALESCE(v_previous,0),'to_tier',p_tier,
                'growth_policy_version',p_policy_version,'facts_digest',p_digest,'mode',p_mode));
        RETURN TRUE;
    END IF;
    RETURN FALSE;
END $$;
GRANT CREATE ON SCHEMA public TO trace_account_trust_evaluation_guard;
ALTER FUNCTION trace_account_trust_advance_frontier() OWNER TO trace_account_trust_evaluation_guard;
ALTER FUNCTION trace_account_trust_input_generation(TEXT,UUID) OWNER TO trace_account_trust_evaluation_guard;
ALTER FUNCTION trace_account_trust_lock_input_generation(TEXT,UUID) OWNER TO trace_account_trust_evaluation_guard;
REVOKE ALL ON FUNCTION trace_account_trust_advance_frontier(),trace_account_trust_input_generation(TEXT,UUID),trace_account_trust_lock_input_generation(TEXT,UUID) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION trace_account_trust_input_generation(TEXT,UUID) TO trace_account_trust_evaluator;
GRANT EXECUTE ON FUNCTION trace_account_trust_lock_input_generation(TEXT,UUID) TO trace_account_admission_runtime;
ALTER FUNCTION trace_record_external_account_trust_evaluation(TEXT,UUID,UUID,TEXT,TEXT,TIMESTAMPTZ,INTEGER,BIGINT,TEXT,BIGINT)
    OWNER TO trace_account_trust_evaluation_guard;
REVOKE CREATE ON SCHEMA public FROM trace_account_trust_evaluation_guard;
REVOKE ALL ON FUNCTION trace_record_external_account_trust_evaluation(TEXT,UUID,UUID,TEXT,TEXT,TIMESTAMPTZ,INTEGER,BIGINT,TEXT,BIGINT) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION trace_record_external_account_trust_evaluation(TEXT,UUID,UUID,TEXT,TEXT,TIMESTAMPTZ,INTEGER,BIGINT,TEXT,BIGINT)
    TO trace_account_trust_evaluator;
REVOKE trace_account_trust_evaluation_guard FROM CURRENT_USER;

-- Consume-proposal invalidation must run after the existing merge locks, not
-- from a proposal trigger: execute_merge consumes before acquiring anchor/account locks.
ALTER FUNCTION trace_account_trust_merge(TEXT,UUID,UUID,UUID) RENAME TO trace_account_trust_merge_before_external;
REVOKE ALL ON FUNCTION trace_account_trust_merge_before_external(TEXT,UUID,UUID,UUID) FROM PUBLIC;
GRANT trace_account_trust_merge_guard TO CURRENT_USER;
GRANT SELECT,UPDATE ON trace_account_trust_frontiers TO trace_account_trust_merge_guard;
CREATE FUNCTION trace_account_trust_merge(p_tenant TEXT,p_surviving_account UUID,p_absorbed_account UUID,p_proposal UUID)
RETURNS BIGINT LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE v_carried BIGINT;
BEGIN
    -- The preserved hook validates the consumed-proposal proof and tenant,
    -- and acquires the existing trust locks before this wrapper touches frontiers.
    v_carried := public.trace_account_trust_merge_before_external(p_tenant,p_surviving_account,p_absorbed_account,p_proposal);
    UPDATE public.trace_account_trust_frontiers SET generation=generation+1
     WHERE tenant_id=p_tenant AND account_id IN (p_surviving_account,p_absorbed_account);
    RETURN v_carried;
END $$;
GRANT CREATE ON SCHEMA public TO trace_account_trust_merge_guard;
ALTER FUNCTION trace_account_trust_merge(TEXT,UUID,UUID,UUID) OWNER TO trace_account_trust_merge_guard;
REVOKE CREATE ON SCHEMA public FROM trace_account_trust_merge_guard;
GRANT EXECUTE ON FUNCTION trace_account_trust_merge(TEXT,UUID,UUID,UUID) TO PUBLIC;
REVOKE trace_account_trust_merge_guard FROM CURRENT_USER;

-- Current gate projections and shared cluster membership are evaluation inputs.
-- Dependency keys serialize gate/fact mutations before cross-tenant enumeration.
-- They are internal lock state, with no login read or write surface.
DO $$ BEGIN
    IF NOT EXISTS(SELECT 1 FROM pg_roles WHERE rolname='trace_account_trust_input_guard') THEN
        CREATE ROLE trace_account_trust_input_guard NOLOGIN NOSUPERUSER NOBYPASSRLS;
    END IF;
    -- CREATEROLE cannot restate NOSUPERUSER or NOBYPASSRLS in ALTER ROLE.
    -- Refuse an elevated existing role instead of relying on a superuser to
    -- normalize it: these roles must never bypass the tenant policies.
    IF EXISTS (SELECT 1 FROM pg_roles
               WHERE rolname = 'trace_account_trust_input_guard' AND (rolsuper OR rolbypassrls)) THEN
        RAISE EXCEPTION 'trace_account_trust_input_guard must be NOSUPERUSER NOBYPASSRLS';
    END IF;
END $$;
ALTER ROLE trace_account_trust_input_guard NOLOGIN;
GRANT trace_account_trust_input_guard,trace_account_trust_evaluation_guard TO CURRENT_USER;
GRANT USAGE ON SCHEMA public TO trace_account_trust_input_guard;
CREATE TABLE trace_account_trust_dependency_locks (
    dependency_key TEXT PRIMARY KEY,
    revision BIGINT NOT NULL DEFAULT 0 CHECK(revision >= 0)
);
ALTER TABLE trace_account_trust_dependency_locks ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_account_trust_dependency_locks FORCE ROW LEVEL SECURITY;
CREATE POLICY account_trust_dependency_guard ON trace_account_trust_dependency_locks
    TO trace_account_trust_input_guard USING(TRUE) WITH CHECK(TRUE);
GRANT SELECT,INSERT,UPDATE ON trace_account_trust_dependency_locks TO trace_account_trust_input_guard;
GRANT SELECT(tenant_id,submission_id,decision_id,dedup_cluster_id) ON trace_gate_decisions TO trace_account_trust_input_guard;
CREATE POLICY account_trust_input_gate_keys ON trace_gate_decisions
    FOR SELECT TO trace_account_trust_input_guard USING(TRUE);
GRANT SELECT(tenant_id,account_id,source_kind,source_id,submission_id) ON trace_account_trust_facts TO trace_account_trust_input_guard;
CREATE POLICY account_trust_input_fact_keys ON trace_account_trust_facts
    FOR SELECT TO trace_account_trust_input_guard USING(TRUE);
GRANT SELECT,UPDATE ON trace_account_trust_frontiers TO trace_account_trust_input_guard;
CREATE POLICY account_trust_input_frontiers ON trace_account_trust_frontiers
    TO trace_account_trust_input_guard USING(TRUE) WITH CHECK(TRUE);

CREATE FUNCTION trace_account_trust_lock_dependencies(p_keys TEXT[]) RETURNS VOID
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE v_key TEXT;
BEGIN
    FOR v_key IN SELECT DISTINCT k COLLATE "C" AS k FROM pg_catalog.unnest(p_keys) k WHERE k IS NOT NULL ORDER BY k COLLATE "C" LOOP
        -- Unlike an advisory lock, updating a versioned row also aborts a
        -- REPEATABLE READ mutation whose snapshot predates a competing writer.
        INSERT INTO public.trace_account_trust_dependency_locks(dependency_key,revision)
        VALUES(v_key,1) ON CONFLICT(dependency_key) DO UPDATE
        SET revision=trace_account_trust_dependency_locks.revision+1;
    END LOOP;
END $$;
CREATE FUNCTION trace_account_trust_lock_fact_dependencies(
    p_old_tenant TEXT,p_old_submission UUID,p_new_tenant TEXT,p_new_submission UUID)
RETURNS VOID LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE v_keys TEXT[];
BEGIN
    SELECT pg_catalog.array_agg(k) INTO v_keys FROM (
        SELECT 'submission:' || p_old_tenant || ':' || p_old_submission::text AS k
        UNION SELECT 'submission:' || p_new_tenant || ':' || p_new_submission::text
        UNION SELECT 'cluster:' || g.dedup_cluster_id::text FROM public.trace_gate_decisions g
         WHERE (g.tenant_id,g.submission_id) IN ((p_old_tenant,p_old_submission),(p_new_tenant,p_new_submission))
    ) dependencies;
    PERFORM public.trace_account_trust_lock_dependencies(v_keys);
END $$;
CREATE FUNCTION trace_account_trust_advance_gate_frontiers() RETURNS TRIGGER
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE v_old_tenant TEXT; v_new_tenant TEXT; v_old_submission UUID; v_new_submission UUID;
    v_old_decision UUID; v_new_decision UUID; v_old_cluster UUID; v_new_cluster UUID;
    v_clusters UUID[]; v_keys TEXT[]; v_account RECORD;
BEGIN
    IF TG_OP='UPDATE' AND
        (OLD.tenant_id,OLD.submission_id,OLD.decision_id,OLD.credit_quality_micros,
         OLD.credit_quality_calibration_version,OLD.dedup_signal_version,OLD.dedup_cluster_id,OLD.decided_at)
        IS NOT DISTINCT FROM
        (NEW.tenant_id,NEW.submission_id,NEW.decision_id,NEW.credit_quality_micros,
         NEW.credit_quality_calibration_version,NEW.dedup_signal_version,NEW.dedup_cluster_id,NEW.decided_at)
    THEN RETURN NULL; END IF;
    IF TG_OP<>'INSERT' THEN
        v_old_tenant:=OLD.tenant_id; v_old_submission:=OLD.submission_id;
        v_old_decision:=OLD.decision_id; v_old_cluster:=OLD.dedup_cluster_id;
    END IF;
    IF TG_OP<>'DELETE' THEN
        v_new_tenant:=NEW.tenant_id; v_new_submission:=NEW.submission_id;
        v_new_decision:=NEW.decision_id; v_new_cluster:=NEW.dedup_cluster_id;
    END IF;
    SELECT pg_catalog.array_agg(DISTINCT cluster_id) INTO v_clusters FROM (
        SELECT v_old_cluster AS cluster_id UNION SELECT v_new_cluster
        UNION SELECT g.dedup_cluster_id FROM public.trace_gate_decisions g
         WHERE (g.tenant_id,g.submission_id) IN ((v_old_tenant,v_old_submission),(v_new_tenant,v_new_submission))
    ) clusters WHERE cluster_id IS NOT NULL;
    SELECT pg_catalog.array_agg(k) INTO v_keys FROM (
        SELECT 'submission:' || v_old_tenant || ':' || v_old_submission::text AS k
        UNION SELECT 'submission:' || v_new_tenant || ':' || v_new_submission::text
        UNION SELECT 'cluster:' || c::text FROM pg_catalog.unnest(v_clusters) c
    ) dependencies;
    PERFORM public.trace_account_trust_lock_dependencies(v_keys);
    -- No account or gate-row locks after dependency/frontier locks. Read only
    -- dependency keys and advance every affected account in deterministic order.
    FOR v_account IN
        SELECT DISTINCT f.tenant_id COLLATE "C" AS tenant_id,f.account_id FROM public.trace_account_trust_facts f
         WHERE f.source_kind='gate_evaluation' AND (
             (f.tenant_id,f.submission_id) IN ((v_old_tenant,v_old_submission),(v_new_tenant,v_new_submission))
             OR (f.tenant_id,f.source_id) IN ((v_old_tenant,v_old_decision),(v_new_tenant,v_new_decision))
             OR EXISTS(SELECT 1 FROM public.trace_gate_decisions g
                  WHERE g.tenant_id=f.tenant_id AND g.submission_id=f.submission_id
                    AND g.dedup_cluster_id=ANY(v_clusters)))
         ORDER BY f.tenant_id COLLATE "C",f.account_id
    LOOP
        UPDATE public.trace_account_trust_frontiers SET generation=generation+1
         WHERE tenant_id=v_account.tenant_id AND account_id=v_account.account_id;
    END LOOP;
    RETURN NULL;
END $$;
CREATE TRIGGER account_trust_gate_frontier AFTER INSERT OR UPDATE OR DELETE ON trace_gate_decisions
    FOR EACH ROW EXECUTE FUNCTION trace_account_trust_advance_gate_frontiers();
GRANT CREATE ON SCHEMA public TO trace_account_trust_input_guard;
ALTER FUNCTION trace_account_trust_lock_dependencies(TEXT[]) OWNER TO trace_account_trust_input_guard;
ALTER FUNCTION trace_account_trust_lock_fact_dependencies(TEXT,UUID,TEXT,UUID) OWNER TO trace_account_trust_input_guard;
ALTER FUNCTION trace_account_trust_advance_gate_frontiers() OWNER TO trace_account_trust_input_guard;
REVOKE CREATE ON SCHEMA public FROM trace_account_trust_input_guard;
REVOKE ALL ON FUNCTION trace_account_trust_lock_dependencies(TEXT[]),
    trace_account_trust_lock_fact_dependencies(TEXT,UUID,TEXT,UUID),
    trace_account_trust_advance_gate_frontiers() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION trace_account_trust_lock_fact_dependencies(TEXT,UUID,TEXT,UUID)
    TO trace_account_trust_evaluation_guard;
REVOKE trace_account_trust_input_guard,trace_account_trust_evaluation_guard FROM CURRENT_USER;

-- Current cluster projection is bounded by the same transaction clock used
-- by external batch snapshots. This is not a historical replay interface.
GRANT trace_account_trust_cluster_guard TO CURRENT_USER;
GRANT CREATE ON SCHEMA public TO trace_account_trust_cluster_guard;
CREATE OR REPLACE FUNCTION trace_account_trust_first_in_cluster(p_tenant TEXT, p_submission UUID)
RETURNS BOOLEAN
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE v_cluster UUID; v_first_tenant TEXT; v_first_submission UUID;
BEGIN
    IF p_tenant IS NULL OR p_submission IS NULL
       OR p_tenant IS DISTINCT FROM public.trace_current_tenant_id() THEN
        RETURN FALSE;
    END IF;
    SELECT g.dedup_cluster_id INTO v_cluster FROM public.trace_gate_decisions g
     WHERE g.tenant_id=p_tenant AND g.submission_id=p_submission
       AND g.dedup_cluster_id IS NOT NULL
       AND pg_catalog.isfinite(g.decided_at)
       AND g.decided_at <= pg_catalog.transaction_timestamp()
     ORDER BY g.decided_at DESC, g.decision_id DESC
     LIMIT 1;
    IF v_cluster IS NULL THEN RETURN FALSE; END IF;
    SELECT m.tenant_id, m.submission_id INTO v_first_tenant, v_first_submission
      FROM (SELECT g.tenant_id, g.submission_id, min(g.decided_at) AS first_at
              FROM public.trace_gate_decisions g
             WHERE g.dedup_cluster_id=v_cluster
               AND pg_catalog.isfinite(g.decided_at)
               AND g.decided_at <= pg_catalog.transaction_timestamp()
             GROUP BY g.tenant_id, g.submission_id) m
     ORDER BY m.first_at, m.tenant_id COLLATE "C", m.submission_id
     LIMIT 1;
    RETURN v_first_tenant=p_tenant AND v_first_submission=p_submission;
END $$;
REVOKE CREATE ON SCHEMA public FROM trace_account_trust_cluster_guard;
REVOKE trace_account_trust_cluster_guard FROM CURRENT_USER;
