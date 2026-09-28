-- Earned account trust, M3 (docs/superpowers/specs/2026-09-26-earned-account-trust-design.md).
--
-- Shadow only. This migration stores evaluations; nothing reads them to set
-- an allowance, and the write function below refuses every mode except
-- 'shadow'. Moving to 'applied' is a later, reviewed migration (switch-on
-- conditions in the spec, "Rollout").
--
-- 1. trace_account_trust_evaluations: one row per evaluation. Counts, a tier,
--    a digest and a policy version; no submission IDs, no principal refs, no
--    content. Forced tenant RLS. trace_account_admission_runtime gets SELECT
--    only (the spec's admission read, not wired in this build);
--    trace_account_trust_worker gets SELECT for the explain route. Writes go
--    only through trace_record_account_trust_evaluation, owned by a new
--    NOLOGIN NOBYPASSRLS guard.
-- 2. trace_account_trust_first_in_cluster: the one cross-tenant question,
--    answered as a boolean. "Is this submission the earliest member of its
--    dedup cluster, across every tenant?" It returns no other tenant's
--    identifiers, counts or times. Owned by its own NOLOGIN NOBYPASSRLS guard
--    with a SELECT-only permissive policy on trace_gate_decisions, the shape
--    of V77's readiness guard. Only the facts guard may call it.
--    The spec preferred a boolean column written by the dedup passes; this is
--    its other option, chosen so the dedup passes are not touched.
-- 3. trace_account_trust_evaluation_inputs: every fact of one account in the
--    caller's tenant, with the current gate-decision fields each gate fact
--    reads and the first-in-cluster boolean. Owned by the facts guard
--    (trace_account_admission_guard); callable by trace_account_trust_worker.

CREATE TABLE trace_account_trust_evaluations (
    tenant_id TEXT NOT NULL,
    evaluation_id UUID NOT NULL,
    account_id UUID NOT NULL,
    growth_policy_version TEXT NOT NULL CHECK (growth_policy_version ~ '^[A-Za-z0-9._-]{1,64}$'),
    mode TEXT NOT NULL CHECK (mode IN ('shadow','applied')),
    as_of TIMESTAMPTZ NOT NULL,
    tier INTEGER NOT NULL CHECK (tier >= 0),
    previous_tier INTEGER CHECK (previous_tier >= 0),
    effective_allowance BIGINT NOT NULL CHECK (effective_allowance > 0),
    units INTEGER NOT NULL CHECK (units >= 0),
    units_capped INTEGER NOT NULL CHECK (units_capped >= 0),
    active_weeks INTEGER NOT NULL CHECK (active_weeks >= 0),
    age_weeks INTEGER NOT NULL CHECK (age_weeks >= 0),
    netted_withdrawn INTEGER NOT NULL CHECK (netted_withdrawn >= 0),
    netted_revoked INTEGER NOT NULL CHECK (netted_revoked >= 0),
    netted_quarantined INTEGER NOT NULL CHECK (netted_quarantined >= 0),
    penalty_active BOOLEAN NOT NULL,
    facts_digest TEXT NOT NULL CHECK (facts_digest ~ '^sha256:[0-9a-f]{64}$'),
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (tenant_id, evaluation_id),
    FOREIGN KEY (tenant_id, account_id) REFERENCES trace_accounts(tenant_id, account_id)
);
CREATE INDEX trace_account_trust_evaluations_latest
    ON trace_account_trust_evaluations(tenant_id, account_id, growth_policy_version, mode, as_of DESC);
ALTER TABLE trace_account_trust_evaluations ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_account_trust_evaluations FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_account_trust_evaluations;
CREATE POLICY trace_corpus_tenant_isolation ON trace_account_trust_evaluations
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());
GRANT SELECT ON trace_account_trust_evaluations TO trace_account_admission_runtime;
GRANT SELECT ON trace_account_trust_evaluations TO trace_account_trust_worker;

-- 2. The cross-tenant first-in-cluster boolean.
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname='trace_account_trust_cluster_guard') THEN
        CREATE ROLE trace_account_trust_cluster_guard NOLOGIN NOBYPASSRLS;
    END IF;
END $$;
ALTER ROLE trace_account_trust_cluster_guard NOLOGIN;
GRANT trace_account_trust_cluster_guard TO CURRENT_USER;
GRANT USAGE ON SCHEMA public TO trace_account_trust_cluster_guard;
GRANT SELECT (tenant_id, submission_id, decision_id, dedup_cluster_id, decided_at)
    ON trace_gate_decisions TO trace_account_trust_cluster_guard;
CREATE POLICY account_trust_cluster_decisions ON trace_gate_decisions
    FOR SELECT TO trace_account_trust_cluster_guard USING (TRUE);
-- The earliest member is the submission with the smallest
-- (first decided_at, tenant_id, submission_id) among every decision stamped
-- with the cluster, in any tenant. The tie-break is deterministic. A
-- submission with no cluster yet is not first: a missing dedup assignment
-- earns nothing rather than everything, so a broken or late signal
-- under-counts.
CREATE FUNCTION trace_account_trust_first_in_cluster(p_tenant TEXT, p_submission UUID)
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
     ORDER BY g.decided_at DESC, g.decision_id DESC
     LIMIT 1;
    IF v_cluster IS NULL THEN RETURN FALSE; END IF;
    SELECT m.tenant_id, m.submission_id INTO v_first_tenant, v_first_submission
      FROM (SELECT g.tenant_id, g.submission_id, min(g.decided_at) AS first_at
              FROM public.trace_gate_decisions g
             WHERE g.dedup_cluster_id=v_cluster
             GROUP BY g.tenant_id, g.submission_id) m
     ORDER BY m.first_at, m.tenant_id COLLATE "C", m.submission_id
     LIMIT 1;
    RETURN v_first_tenant=p_tenant AND v_first_submission=p_submission;
END $$;
GRANT CREATE ON SCHEMA public TO trace_account_trust_cluster_guard;
ALTER FUNCTION trace_account_trust_first_in_cluster(TEXT,UUID)
    OWNER TO trace_account_trust_cluster_guard;
REVOKE CREATE ON SCHEMA public FROM trace_account_trust_cluster_guard;
REVOKE ALL ON FUNCTION trace_account_trust_first_in_cluster(TEXT,UUID) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION trace_account_trust_first_in_cluster(TEXT,UUID)
    TO trace_account_admission_guard;
REVOKE trace_account_trust_cluster_guard FROM CURRENT_USER;

-- 3. The evaluator's read of one account's facts.
GRANT trace_account_admission_guard TO CURRENT_USER;
GRANT SELECT (credit_quality_micros, credit_quality_calibration_version, dedup_signal_version)
    ON trace_gate_decisions TO trace_account_admission_guard;
CREATE FUNCTION trace_account_trust_evaluation_inputs(p_tenant TEXT, p_account UUID)
RETURNS TABLE(source_kind TEXT, source_id UUID, submission_id UUID, outcome TEXT,
              evaluator_version TEXT, occurred_at TIMESTAMPTZ,
              credit_quality_micros BIGINT, credit_quality_calibration_version INTEGER,
              dedup_signal_version TEXT, first_in_cluster BOOLEAN)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
    SELECT f.source_kind, f.source_id, f.submission_id, f.outcome, f.evaluator_version,
           f.occurred_at, g.credit_quality_micros, g.credit_quality_calibration_version,
           g.dedup_signal_version,
           CASE WHEN f.source_kind='gate_evaluation'
                THEN public.trace_account_trust_first_in_cluster(f.tenant_id, f.submission_id)
           END
      FROM public.trace_account_trust_facts f
      LEFT JOIN public.trace_gate_decisions g ON f.source_kind='gate_evaluation'
           AND g.tenant_id=f.tenant_id AND g.decision_id=f.source_id
     WHERE f.tenant_id=p_tenant AND f.account_id=p_account
       AND p_tenant=public.trace_current_tenant_id()
     ORDER BY f.source_kind, f.source_id;
$$;
GRANT CREATE ON SCHEMA public TO trace_account_admission_guard;
ALTER FUNCTION trace_account_trust_evaluation_inputs(TEXT,UUID)
    OWNER TO trace_account_admission_guard;
REVOKE CREATE ON SCHEMA public FROM trace_account_admission_guard;
REVOKE ALL ON FUNCTION trace_account_trust_evaluation_inputs(TEXT,UUID) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION trace_account_trust_evaluation_inputs(TEXT,UUID)
    TO trace_account_trust_worker;
REVOKE trace_account_admission_guard FROM CURRENT_USER;

-- 1 (cont.). The only write path for evaluations. Shadow only in this
-- migration. Appends a hash-only account_trust_tier_changed row to
-- trace_account_audit when the tier differs from the previous evaluation of
-- the same account, policy version and mode (or, for the first evaluation,
-- from tier 0). Returns whether it did.
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname='trace_account_trust_evaluation_guard') THEN
        CREATE ROLE trace_account_trust_evaluation_guard NOLOGIN NOBYPASSRLS;
    END IF;
END $$;
ALTER ROLE trace_account_trust_evaluation_guard NOLOGIN;
GRANT trace_account_trust_evaluation_guard TO CURRENT_USER;
GRANT USAGE ON SCHEMA public TO trace_account_trust_evaluation_guard;
GRANT SELECT, INSERT ON trace_account_trust_evaluations TO trace_account_trust_evaluation_guard;
GRANT INSERT ON trace_account_audit TO trace_account_trust_evaluation_guard;
GRANT USAGE ON SEQUENCE trace_account_audit_audit_sequence_seq TO trace_account_trust_evaluation_guard;
CREATE FUNCTION trace_record_account_trust_evaluation(
    p_tenant TEXT, p_evaluation UUID, p_account UUID, p_policy_version TEXT, p_mode TEXT,
    p_as_of TIMESTAMPTZ, p_tier INTEGER, p_effective_allowance BIGINT, p_units INTEGER,
    p_units_capped INTEGER, p_active_weeks INTEGER, p_age_weeks INTEGER,
    p_netted_withdrawn INTEGER, p_netted_revoked INTEGER, p_netted_quarantined INTEGER,
    p_penalty_active BOOLEAN, p_digest TEXT)
RETURNS BOOLEAN
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE v_previous INTEGER;
BEGIN
    IF p_tenant IS NULL OR p_tenant IS DISTINCT FROM public.trace_current_tenant_id() THEN
        RAISE EXCEPTION USING MESSAGE = 'account_trust_evaluation_unauthorized';
    END IF;
    IF p_mode IS DISTINCT FROM 'shadow' THEN
        RAISE EXCEPTION USING MESSAGE = 'account_trust_evaluation_shadow_only';
    END IF;
    -- Serialize evaluations of one account so previous_tier is exact.
    PERFORM pg_catalog.pg_advisory_xact_lock(
        pg_catalog.hashtextextended('trace-account-trust-evaluation:' || p_tenant || ':' || p_account::text, 0));
    SELECT e.tier INTO v_previous FROM public.trace_account_trust_evaluations e
     WHERE e.tenant_id=p_tenant AND e.account_id=p_account
       AND e.growth_policy_version=p_policy_version AND e.mode=p_mode
     ORDER BY e.as_of DESC, e.recorded_at DESC
     LIMIT 1;
    INSERT INTO public.trace_account_trust_evaluations(tenant_id, evaluation_id, account_id,
        growth_policy_version, mode, as_of, tier, previous_tier, effective_allowance, units,
        units_capped, active_weeks, age_weeks, netted_withdrawn, netted_revoked,
        netted_quarantined, penalty_active, facts_digest)
    VALUES (p_tenant, p_evaluation, p_account, p_policy_version, p_mode, p_as_of, p_tier,
        v_previous, p_effective_allowance, p_units, p_units_capped, p_active_weeks, p_age_weeks,
        p_netted_withdrawn, p_netted_revoked, p_netted_quarantined, p_penalty_active, p_digest);
    IF p_tier IS DISTINCT FROM COALESCE(v_previous, 0) THEN
        INSERT INTO public.trace_account_audit(tenant_id, action, actor_ref, outcome, safe_metadata)
        VALUES (p_tenant, 'account_trust_tier_changed', 'account-actor:' || p_account::text, p_mode,
            pg_catalog.jsonb_build_object(
                'from_tier', COALESCE(v_previous, 0),
                'to_tier', p_tier,
                'growth_policy_version', p_policy_version,
                'facts_digest', p_digest,
                'mode', p_mode));
        RETURN TRUE;
    END IF;
    RETURN FALSE;
END $$;
GRANT CREATE ON SCHEMA public TO trace_account_trust_evaluation_guard;
ALTER FUNCTION trace_record_account_trust_evaluation(TEXT,UUID,UUID,TEXT,TEXT,TIMESTAMPTZ,
    INTEGER,BIGINT,INTEGER,INTEGER,INTEGER,INTEGER,INTEGER,INTEGER,INTEGER,BOOLEAN,TEXT)
    OWNER TO trace_account_trust_evaluation_guard;
REVOKE CREATE ON SCHEMA public FROM trace_account_trust_evaluation_guard;
REVOKE ALL ON FUNCTION trace_record_account_trust_evaluation(TEXT,UUID,UUID,TEXT,TEXT,
    TIMESTAMPTZ,INTEGER,BIGINT,INTEGER,INTEGER,INTEGER,INTEGER,INTEGER,INTEGER,INTEGER,
    BOOLEAN,TEXT) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION trace_record_account_trust_evaluation(TEXT,UUID,UUID,TEXT,TEXT,
    TIMESTAMPTZ,INTEGER,BIGINT,INTEGER,INTEGER,INTEGER,INTEGER,INTEGER,INTEGER,INTEGER,
    BOOLEAN,TEXT) TO trace_account_trust_worker;
REVOKE trace_account_trust_evaluation_guard FROM CURRENT_USER;
