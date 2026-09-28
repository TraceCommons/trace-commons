-- Earned account trust, M2 (docs/superpowers/specs/2026-09-26-earned-account-trust-design.md),
-- plus the seams the batch fact recorder runs through.
--
-- 1. trace_record_account_trust_fact is replaced in place: same signature,
--    same owner (trace_account_admission_guard, NOLOGIN NOBYPASSRLS), same
--    tenant check first. It now verifies all six kinds from their server
--    rows, stamps occurred_at from the source row, and records acceptance
--    through self-review, or through an approver it cannot name, as a
--    non-qualifying outcome instead of 'accepted'.
-- 2. trace_account_trust_fact_candidates lists, for one account in the
--    caller's tenant, the sources that have no fact yet. Same owner, same
--    tenant RLS.
-- 3. trace_account_trust_worker_accounts is the single cross-tenant read: a
--    page of (tenant_id, account_id) keys of open accounts in anchored
--    tenants. Owned by its own NOLOGIN NOBYPASSRLS guard with a SELECT-only
--    permissive policy on trace_accounts, as V77's readiness guard is. It
--    returns the two keys and nothing else.
-- 4. trace_account_trust_worker (NOLOGIN, NOBYPASSRLS) holds EXECUTE on the
--    three and nothing else. The deployer grants it to the login that runs
--    the recorder. trace_account_admission_runtime is not widened: it keeps
--    only its V77 EXECUTE on the recording function and gains no read of
--    facts or of the account enumeration.
--
-- Account mapping follows V77: a submission's auth_principal_ref joined to
-- the account's trace_account_principals row. A positive fact (acceptance, a
-- gate evaluation) needs a live link, as V77 required. An adverse or netting
-- fact (withdrawal, revocation, quarantine, penalty) does not: unlinking a
-- device must not be a way to shed one. A legacy `tenant-...` submission maps
-- to no account and earns nothing.

-- The recorder's login gets this role and nothing else from this migration.
-- EXECUTE is granted beside each function below, while the migrating role
-- still holds the owning guard: a GRANT on a function needs its owner's
-- authority, and a non-superuser migrator has it only through membership.
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname='trace_account_trust_worker') THEN
        CREATE ROLE trace_account_trust_worker NOLOGIN NOBYPASSRLS;
    END IF;
END $$;
ALTER ROLE trace_account_trust_worker NOLOGIN;
GRANT USAGE ON SCHEMA public TO trace_account_trust_worker;

GRANT trace_account_admission_guard TO CURRENT_USER;
GRANT SELECT (withdrawn_at, revoked_at, reviewed_at, received_at)
    ON trace_submissions TO trace_account_admission_guard;
GRANT SELECT (credit_event_id, occurred_at, actor_role)
    ON trace_credit_ledger TO trace_account_admission_guard;
GRANT SELECT (decided_at) ON trace_gate_decisions TO trace_account_admission_guard;
-- The approving reviewer is found in the review audit trail: the
-- review_decision row whose resulting_status is 'accepted'.
-- review_assigned_to_principal_ref names the assignee, who need not be the
-- approver, so it is not read.
GRANT SELECT (tenant_id, submission_id, action, actor_principal_ref, metadata_json)
    ON trace_audit_events TO trace_account_admission_guard;

CREATE OR REPLACE FUNCTION trace_record_account_trust_fact(p_tenant TEXT,p_account UUID,
    p_kind TEXT,p_source UUID) RETURNS TEXT
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE v_submission UUID; v_status TEXT; v_version TEXT;
        v_perplexity BOOLEAN; v_novelty BOOLEAN; v_outcome TEXT;
        v_occurred TIMESTAMPTZ; v_withdrawn TIMESTAMPTZ; v_revoked TIMESTAMPTZ;
        v_reviewed TIMESTAMPTZ; v_received TIMESTAMPTZ;
BEGIN
    IF p_tenant IS DISTINCT FROM public.trace_current_tenant_id()
       OR p_account IS NULL OR p_source IS NULL THEN RETURN NULL; END IF;
    IF p_kind='accepted_submission' THEN
        SELECT s.submission_id INTO v_submission FROM public.trace_submissions s
          JOIN public.trace_account_principals p ON p.tenant_id=s.tenant_id
             AND p.principal_ref=s.auth_principal_ref AND p.account_id=p_account
         WHERE s.tenant_id=p_tenant AND s.submission_id=p_source
           AND s.status='accepted' AND p.unlinked_at IS NULL;
        IF v_submission IS NULL THEN RETURN NULL; END IF;
        SELECT min(c.occurred_at) INTO v_occurred FROM public.trace_credit_ledger c
         WHERE c.tenant_id=p_tenant AND c.submission_id=v_submission
           AND c.event_type='accepted';
        IF v_occurred IS NULL THEN RETURN NULL; END IF;
        IF EXISTS (
            SELECT 1 FROM public.trace_audit_events e
              JOIN public.trace_account_principals p ON p.tenant_id=e.tenant_id
                 AND p.principal_ref=e.actor_principal_ref AND p.account_id=p_account
             WHERE e.tenant_id=p_tenant AND e.submission_id=v_submission
               AND e.action='review'
               AND e.metadata_json->>'kind'='review_decision'
               AND e.metadata_json->>'resulting_status'='accepted'
        ) THEN
            v_outcome := 'accepted_self_reviewed';
        ELSIF NOT EXISTS (
            SELECT 1 FROM public.trace_audit_events e
             WHERE e.tenant_id=p_tenant AND e.submission_id=v_submission
               AND e.action='review'
               AND e.metadata_json->>'kind'='review_decision'
               AND e.metadata_json->>'resulting_status'='accepted'
        ) AND EXISTS (
            -- Ingest's own acceptance writes its credit event as 'system'. A
            -- human-issued acceptance with no approval row cannot name its
            -- approver, so it fails closed.
            SELECT 1 FROM public.trace_credit_ledger c
             WHERE c.tenant_id=p_tenant AND c.submission_id=v_submission
               AND c.event_type='accepted' AND c.actor_role<>'system'
        ) THEN
            v_outcome := 'accepted_approver_unknown';
        ELSE
            v_outcome := 'accepted';
        END IF;
    ELSIF p_kind='gate_evaluation' THEN
        SELECT g.submission_id,g.gate_policy_version,g.perplexity_passed,
               g.novelty_passed,s.status,g.decided_at
          INTO v_submission,v_version,v_perplexity,v_novelty,v_status,v_occurred
          FROM public.trace_gate_decisions g
          JOIN public.trace_submissions s ON s.tenant_id=g.tenant_id
               AND s.submission_id=g.submission_id
          JOIN public.trace_account_principals p ON p.tenant_id=s.tenant_id
               AND p.principal_ref=s.auth_principal_ref AND p.account_id=p_account
         WHERE g.tenant_id=p_tenant AND g.decision_id=p_source
           AND p.unlinked_at IS NULL;
        IF v_submission IS NULL OR v_version IS NULL OR v_version='' THEN RETURN NULL; END IF;
        v_outcome := CASE WHEN v_status<>'accepted' THEN 'evaluated_not_accepted'
                          WHEN v_perplexity AND v_novelty THEN 'evaluated_passed'
                          ELSE 'evaluated_failed' END;
    ELSIF p_kind IN ('submission_withdrawn','submission_revoked','submission_quarantined') THEN
        SELECT s.submission_id,s.status,s.withdrawn_at,s.revoked_at,s.reviewed_at,s.received_at
          INTO v_submission,v_status,v_withdrawn,v_revoked,v_reviewed,v_received
          FROM public.trace_submissions s
          JOIN public.trace_account_principals p ON p.tenant_id=s.tenant_id
               AND p.principal_ref=s.auth_principal_ref AND p.account_id=p_account
         WHERE s.tenant_id=p_tenant AND s.submission_id=p_source;
        IF v_submission IS NULL THEN RETURN NULL; END IF;
        IF p_kind='submission_withdrawn' AND v_withdrawn IS NOT NULL THEN
            v_outcome := 'withdrawn'; v_occurred := v_withdrawn;
        ELSIF p_kind='submission_revoked' AND v_status='revoked' AND v_withdrawn IS NULL THEN
            -- V43 is what separates the two: withdrawn_at marks a withdrawal.
            v_outcome := 'revoked'; v_occurred := COALESCE(v_revoked, v_received);
        ELSIF p_kind='submission_quarantined' AND v_status='quarantined' THEN
            -- No quarantined_at column exists. A status change through the
            -- store stamps reviewed_at; a quarantine at submit time has only
            -- received_at. Both are fixed once the fact is recorded.
            v_outcome := 'quarantined'; v_occurred := COALESCE(v_reviewed, v_received);
        ELSE
            RETURN NULL;
        END IF;
        v_version := NULL;
    ELSIF p_kind='abuse_penalty' THEN
        SELECT c.submission_id,c.occurred_at INTO v_submission,v_occurred
          FROM public.trace_credit_ledger c
          JOIN public.trace_submissions s ON s.tenant_id=c.tenant_id
               AND s.submission_id=c.submission_id
          JOIN public.trace_account_principals p ON p.tenant_id=s.tenant_id
               AND p.principal_ref=s.auth_principal_ref AND p.account_id=p_account
         WHERE c.tenant_id=p_tenant AND c.credit_event_id=p_source
           AND c.event_type='abuse_penalty';
        IF v_submission IS NULL THEN RETURN NULL; END IF;
        v_outcome := 'penalized';
    ELSE
        RETURN NULL;
    END IF;
    INSERT INTO public.trace_account_trust_facts(tenant_id,account_id,source_kind,
        source_id,submission_id,outcome,evaluator_version,occurred_at)
      VALUES(p_tenant,p_account,p_kind,p_source,v_submission,v_outcome,v_version,v_occurred)
      ON CONFLICT DO NOTHING;
    RETURN (SELECT outcome FROM public.trace_account_trust_facts
             WHERE tenant_id=p_tenant AND account_id=p_account
               AND source_kind=p_kind AND source_id=p_source);
END $$;

CREATE FUNCTION trace_account_trust_fact_candidates(p_tenant TEXT, p_account UUID, p_limit BIGINT)
RETURNS TABLE(source_kind TEXT, source_id UUID)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
    WITH owned AS (
        SELECT s.submission_id, s.status, s.withdrawn_at, p.unlinked_at IS NULL AS live
          FROM public.trace_submissions s
          JOIN public.trace_account_principals p ON p.tenant_id=s.tenant_id
               AND p.principal_ref=s.auth_principal_ref AND p.account_id=p_account
         WHERE s.tenant_id=p_tenant AND p_tenant=public.trace_current_tenant_id()
    ), sources AS (
        SELECT 'accepted_submission'::text AS kind, o.submission_id AS id FROM owned o
         WHERE o.live AND o.status='accepted' AND EXISTS (
               SELECT 1 FROM public.trace_credit_ledger c
                WHERE c.tenant_id=p_tenant AND c.submission_id=o.submission_id
                  AND c.event_type='accepted')
        UNION ALL
        SELECT 'gate_evaluation', g.decision_id FROM owned o
          JOIN public.trace_gate_decisions g ON g.tenant_id=p_tenant
               AND g.submission_id=o.submission_id
         WHERE o.live AND g.gate_policy_version<>''
        UNION ALL
        SELECT 'submission_withdrawn', o.submission_id FROM owned o
         WHERE o.withdrawn_at IS NOT NULL
        UNION ALL
        SELECT 'submission_revoked', o.submission_id FROM owned o
         WHERE o.status='revoked' AND o.withdrawn_at IS NULL
        UNION ALL
        SELECT 'submission_quarantined', o.submission_id FROM owned o
         WHERE o.status='quarantined'
        UNION ALL
        SELECT 'abuse_penalty', c.credit_event_id FROM owned o
          JOIN public.trace_credit_ledger c ON c.tenant_id=p_tenant
               AND c.submission_id=o.submission_id
         WHERE c.event_type='abuse_penalty'
    )
    SELECT s.kind, s.id FROM sources s
     WHERE NOT EXISTS (
        SELECT 1 FROM public.trace_account_trust_facts f
         WHERE f.tenant_id=p_tenant AND f.account_id=p_account
           AND f.source_kind=s.kind AND f.source_id=s.id)
     ORDER BY s.kind, s.id
     LIMIT LEAST(GREATEST(COALESCE(p_limit,0),0),10000);
$$;

GRANT CREATE ON SCHEMA public TO trace_account_admission_guard;
ALTER FUNCTION trace_account_trust_fact_candidates(TEXT,UUID,BIGINT)
    OWNER TO trace_account_admission_guard;
REVOKE CREATE ON SCHEMA public FROM trace_account_admission_guard;
REVOKE ALL ON FUNCTION trace_account_trust_fact_candidates(TEXT,UUID,BIGINT) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION trace_account_trust_fact_candidates(TEXT,UUID,BIGINT),
    trace_record_account_trust_fact(TEXT,UUID,TEXT,UUID)
    TO trace_account_trust_worker;
REVOKE trace_account_admission_guard FROM CURRENT_USER;

-- The one cross-tenant read. Its owner cannot log in, bypass RLS, or write;
-- the permissive policy below is SELECT-only and names only this role.
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname='trace_account_trust_enumeration_guard') THEN
        CREATE ROLE trace_account_trust_enumeration_guard NOLOGIN NOBYPASSRLS;
    END IF;
END $$;
ALTER ROLE trace_account_trust_enumeration_guard NOLOGIN;
GRANT trace_account_trust_enumeration_guard TO CURRENT_USER;
GRANT USAGE ON SCHEMA public TO trace_account_trust_enumeration_guard;
GRANT SELECT (tenant_id, account_id, closed_at) ON trace_accounts
    TO trace_account_trust_enumeration_guard;
CREATE POLICY account_trust_enumeration_accounts ON trace_accounts
    FOR SELECT TO trace_account_trust_enumeration_guard USING (TRUE);
CREATE FUNCTION trace_account_trust_worker_accounts(
    p_after_tenant TEXT, p_after_account UUID, p_limit BIGINT)
RETURNS TABLE(tenant_id TEXT, account_id UUID)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
    SELECT a.tenant_id, a.account_id FROM public.trace_accounts a
     WHERE a.closed_at IS NULL
       AND a.tenant_id ~ '^(near-|nearai-)[0-9a-f]{64}$'
       AND (p_after_tenant IS NULL OR p_after_account IS NULL
            OR (a.tenant_id COLLATE "C", a.account_id)
               > (p_after_tenant COLLATE "C", p_after_account))
     ORDER BY a.tenant_id COLLATE "C", a.account_id
     LIMIT LEAST(GREATEST(COALESCE(p_limit,0),0),10000);
$$;
GRANT CREATE ON SCHEMA public TO trace_account_trust_enumeration_guard;
ALTER FUNCTION trace_account_trust_worker_accounts(TEXT,UUID,BIGINT)
    OWNER TO trace_account_trust_enumeration_guard;
REVOKE CREATE ON SCHEMA public FROM trace_account_trust_enumeration_guard;
REVOKE ALL ON FUNCTION trace_account_trust_worker_accounts(TEXT,UUID,BIGINT) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION trace_account_trust_worker_accounts(TEXT,UUID,BIGINT)
    TO trace_account_trust_worker;
REVOKE trace_account_trust_enumeration_guard FROM CURRENT_USER;

