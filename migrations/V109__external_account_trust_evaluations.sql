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
END $$;
ALTER ROLE trace_account_trust_evaluator NOLOGIN NOSUPERUSER NOBYPASSRLS;
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
