-- Account-authority reservations are distinct from legacy evidence reservations.
-- The policy is supplied explicitly at activation; this migration sets no quota.
ALTER TABLE trace_account_trust DROP CONSTRAINT trace_account_trust_authority_check;
ALTER TABLE trace_account_trust ADD CONSTRAINT trace_account_trust_authority_check
    CHECK (authority IN ('bounded', 'invited'));
ALTER TABLE trace_account_invite_grants ADD COLUMN revoked_at TIMESTAMPTZ;

CREATE TABLE trace_account_admission_budget (
    tenant_id TEXT NOT NULL,
    account_id UUID NOT NULL,
    period_id TEXT NOT NULL,
    policy_version TEXT NOT NULL,
    cost_limit BIGINT NOT NULL CHECK (cost_limit > 0),
    cost_bound BIGINT NOT NULL CHECK (cost_bound > 0),
    cost_used BIGINT NOT NULL DEFAULT 0 CHECK (cost_used >= 0),
    PRIMARY KEY (tenant_id, account_id, period_id),
    FOREIGN KEY (tenant_id, account_id) REFERENCES trace_accounts(tenant_id, account_id)
);
CREATE TABLE trace_account_admission_submissions (
    tenant_id TEXT NOT NULL,
    submission_id UUID NOT NULL,
    account_id UUID NOT NULL,
    body_hash TEXT NOT NULL CHECK (body_hash ~ '^[0-9a-f]{64}$'),
    authority_kind TEXT NOT NULL CHECK (authority_kind = 'account'),
    trust_version BIGINT NOT NULL CHECK (trust_version > 0),
    policy_version TEXT NOT NULL,
    period_id TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('reserved','processing','completed','released')),
    lease_id UUID NOT NULL,
    lease_expires_at TIMESTAMPTZ NOT NULL,
    last_cost_bound BIGINT NOT NULL CHECK (last_cost_bound > 0),
    last_charged BOOLEAN NOT NULL,
    ever_processed BOOLEAN NOT NULL DEFAULT FALSE,
    PRIMARY KEY (tenant_id, submission_id),
    FOREIGN KEY (tenant_id, account_id) REFERENCES trace_accounts(tenant_id, account_id)
);
CREATE INDEX trace_account_admission_submissions_account
    ON trace_account_admission_submissions(tenant_id, account_id);

-- Historical server-observed facts for a future reviewed growth policy. This
-- table is not consulted by admission and never changes the allowance.
CREATE TABLE trace_account_trust_facts (
    tenant_id TEXT NOT NULL,
    account_id UUID NOT NULL,
    source_kind TEXT NOT NULL CHECK (source_kind IN ('accepted_submission','gate_evaluation')),
    source_id UUID NOT NULL,
    submission_id UUID NOT NULL,
    outcome TEXT NOT NULL CHECK (outcome IN ('accepted','evaluated_passed','evaluated_failed','evaluated_not_accepted')),
    evaluator_version TEXT,
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (tenant_id, account_id, source_kind, source_id),
    FOREIGN KEY (tenant_id, account_id) REFERENCES trace_accounts(tenant_id, account_id),
    FOREIGN KEY (tenant_id, submission_id) REFERENCES trace_submissions(tenant_id, submission_id),
    CHECK ((source_kind='accepted_submission' AND outcome='accepted' AND evaluator_version IS NULL)
       OR (source_kind='gate_evaluation' AND outcome<>'accepted' AND evaluator_version IS NOT NULL))
);
ALTER TABLE trace_account_trust_facts ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_account_trust_facts FORCE ROW LEVEL SECURITY;
CREATE POLICY trace_corpus_tenant_isolation ON trace_account_trust_facts
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

ALTER TABLE trace_account_admission_budget ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_account_admission_budget FORCE ROW LEVEL SECURITY;
CREATE POLICY trace_corpus_tenant_isolation ON trace_account_admission_budget
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());
ALTER TABLE trace_account_admission_submissions ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_account_admission_submissions FORCE ROW LEVEL SECURITY;
CREATE POLICY trace_corpus_tenant_isolation ON trace_account_admission_submissions
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

-- Ingest must be granted this NOLOGIN role by the deployer. It cannot mint an
-- invite, enumerate the registry, or bypass tenant RLS.
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_account_admission_runtime') THEN
        CREATE ROLE trace_account_admission_runtime NOLOGIN NOBYPASSRLS;
    END IF;
END $$;
GRANT USAGE ON SCHEMA public TO trace_account_admission_runtime;
GRANT SELECT (tenant_id, account_id, closed_at), UPDATE (account_id)
    ON trace_accounts TO trace_account_admission_runtime;
GRANT SELECT (tenant_id, account_id, principal_ref, unlinked_at)
    ON trace_account_principals TO trace_account_admission_runtime;
GRANT SELECT (tenant_id, account_id, principal_ref, device_key_id)
    ON trace_near_provisioned_devices TO trace_account_admission_runtime;
GRANT SELECT (tenant_id, device_key_id, revoked_at, onboarding_origin)
    ON device_keys TO trace_account_admission_runtime;
GRANT SELECT (tenant_id, account_id)
    ON trace_near_account_anchors TO trace_account_admission_runtime;
GRANT SELECT, INSERT, UPDATE ON trace_account_trust TO trace_account_admission_runtime;
GRANT SELECT (tenant_id, account_id, revoked_at)
    ON trace_account_invite_grants TO trace_account_admission_runtime;
GRANT SELECT, INSERT, UPDATE ON trace_account_admission_budget,
    trace_account_admission_submissions TO trace_account_admission_runtime;
GRANT SELECT (tenant_id,submission_id,anchor_hash,body_hash,status,receipt_hash,challenge_hash)
    ON trace_admission_submissions TO trace_account_admission_runtime;

-- PostgreSQL row locks require UPDATE privilege. Keep it out of the ingest
-- login: the only privileged operations exposed are two boolean liveness
-- checks, each constrained by the caller's tenant RLS context.
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_account_admission_guard') THEN
        CREATE ROLE trace_account_admission_guard NOLOGIN NOBYPASSRLS;
    END IF;
END $$;
GRANT trace_account_admission_guard TO CURRENT_USER;
GRANT USAGE ON SCHEMA public TO trace_account_admission_guard;
GRANT SELECT (tenant_id, account_id, principal_ref, device_key_id)
    ON trace_near_provisioned_devices TO trace_account_admission_guard;
GRANT SELECT (tenant_id, device_key_id, revoked_at, onboarding_origin), UPDATE (revoked_at)
    ON device_keys TO trace_account_admission_guard;
GRANT SELECT (tenant_id, account_id, principal_ref, unlinked_at), UPDATE (unlinked_at)
    ON trace_account_principals TO trace_account_admission_guard;
GRANT SELECT (tenant_id, account_id, revoked_at), UPDATE (revoked_at)
    ON trace_account_invite_grants TO trace_account_admission_guard;
GRANT SELECT (tenant_id, account_id, principal_ref, unlinked_at)
    ON trace_account_principals TO trace_account_admission_guard;
GRANT SELECT (tenant_id, submission_id, auth_principal_ref, status)
    ON trace_submissions TO trace_account_admission_guard;
GRANT SELECT (tenant_id, submission_id, event_type)
    ON trace_credit_ledger TO trace_account_admission_guard;
GRANT SELECT (tenant_id, decision_id, submission_id, gate_policy_version,
    perplexity_passed, novelty_passed)
    ON trace_gate_decisions TO trace_account_admission_guard;
GRANT SELECT, INSERT ON trace_account_trust_facts TO trace_account_admission_guard;

CREATE FUNCTION trace_account_admission_live_device(p_tenant TEXT,p_account UUID,p_principal TEXT)
RETURNS BOOLEAN LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE observed INTEGER;
BEGIN
    IF p_tenant IS DISTINCT FROM public.trace_current_tenant_id() THEN RETURN FALSE; END IF;
    SELECT 1 INTO observed FROM public.trace_near_provisioned_devices n
      JOIN public.device_keys d ON d.tenant_id=n.tenant_id AND d.device_key_id=n.device_key_id
      JOIN public.trace_account_principals p ON p.tenant_id=n.tenant_id
          AND p.account_id=n.account_id AND p.principal_ref=n.principal_ref
     WHERE n.tenant_id=p_tenant AND n.account_id=p_account AND n.principal_ref=p_principal
       AND d.revoked_at IS NULL AND d.onboarding_origin IN ('near','near_ai')
       AND p.unlinked_at IS NULL
     LIMIT 1 FOR SHARE OF d,p;
    RETURN FOUND;
END $$;

CREATE FUNCTION trace_account_admission_active_grant(p_tenant TEXT,p_account UUID)
RETURNS BOOLEAN LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE observed INTEGER;
BEGIN
    IF p_tenant IS DISTINCT FROM public.trace_current_tenant_id() THEN RETURN FALSE; END IF;
    SELECT 1 INTO observed FROM public.trace_account_invite_grants
     WHERE tenant_id=p_tenant AND account_id=p_account AND revoked_at IS NULL
     LIMIT 1 FOR SHARE;
    RETURN FOUND;
END $$;

CREATE FUNCTION trace_record_account_trust_fact(p_tenant TEXT,p_account UUID,
    p_kind TEXT,p_source UUID) RETURNS TEXT
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE v_submission UUID; v_status TEXT; v_version TEXT;
        v_perplexity BOOLEAN; v_novelty BOOLEAN; v_outcome TEXT;
BEGIN
    IF p_tenant IS DISTINCT FROM public.trace_current_tenant_id() THEN RETURN NULL; END IF;
    IF p_kind='accepted_submission' THEN
        SELECT s.submission_id INTO v_submission FROM public.trace_submissions s
          JOIN public.trace_account_principals p ON p.tenant_id=s.tenant_id
             AND p.principal_ref=s.auth_principal_ref AND p.account_id=p_account
         WHERE s.tenant_id=p_tenant AND s.submission_id=p_source
           AND s.status='accepted' AND p.unlinked_at IS NULL
           AND EXISTS (SELECT 1 FROM public.trace_credit_ledger c
                WHERE c.tenant_id=s.tenant_id AND c.submission_id=s.submission_id
                  AND c.event_type='accepted');
        IF v_submission IS NULL THEN RETURN NULL; END IF;
        v_outcome := 'accepted';
    ELSIF p_kind='gate_evaluation' THEN
        SELECT g.submission_id,g.gate_policy_version,g.perplexity_passed,
               g.novelty_passed,s.status
          INTO v_submission,v_version,v_perplexity,v_novelty,v_status
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
    ELSE
        RETURN NULL;
    END IF;
    INSERT INTO public.trace_account_trust_facts(tenant_id,account_id,source_kind,
        source_id,submission_id,outcome,evaluator_version)
      VALUES(p_tenant,p_account,p_kind,p_source,v_submission,v_outcome,v_version)
      ON CONFLICT DO NOTHING;
    RETURN (SELECT outcome FROM public.trace_account_trust_facts
             WHERE tenant_id=p_tenant AND account_id=p_account
               AND source_kind=p_kind AND source_id=p_source);
END $$;
GRANT CREATE ON SCHEMA public TO trace_account_admission_guard;
ALTER FUNCTION trace_account_admission_live_device(TEXT,UUID,TEXT) OWNER TO trace_account_admission_guard;
ALTER FUNCTION trace_account_admission_active_grant(TEXT,UUID) OWNER TO trace_account_admission_guard;
ALTER FUNCTION trace_record_account_trust_fact(TEXT,UUID,TEXT,UUID) OWNER TO trace_account_admission_guard;
REVOKE CREATE ON SCHEMA public FROM trace_account_admission_guard;
REVOKE ALL ON FUNCTION trace_account_admission_live_device(TEXT,UUID,TEXT) FROM PUBLIC;
REVOKE ALL ON FUNCTION trace_account_admission_active_grant(TEXT,UUID) FROM PUBLIC;
REVOKE ALL ON FUNCTION trace_record_account_trust_fact(TEXT,UUID,TEXT,UUID) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION trace_account_admission_live_device(TEXT,UUID,TEXT),
    trace_account_admission_active_grant(TEXT,UUID),
    trace_record_account_trust_fact(TEXT,UUID,TEXT,UUID) TO trace_account_admission_runtime;
REVOKE trace_account_admission_guard FROM CURRENT_USER;

-- A cutover must not strand an already authorized V59 submission after its
-- lease is released or expires. Reuse that ledger's original authority and
-- charge rules; never create a second account-ledger row for the same UUID.
-- The guard has tenant RLS and only the legacy table privileges from V59.
GRANT trace_admission_guard TO CURRENT_USER;
GRANT CREATE ON SCHEMA public TO trace_admission_guard;
CREATE OR REPLACE FUNCTION trace_resume_legacy_admission(
    p_tenant TEXT, p_anchor TEXT, p_submission UUID, p_body TEXT,
    p_lease UUID, p_lease_seconds BIGINT
) RETURNS TEXT LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE prior public.trace_admission_submissions%ROWTYPE;
    attempt_limit BIGINT; account_limit BIGINT; global_limit BIGINT;
BEGIN
    IF p_tenant IS DISTINCT FROM public.trace_current_tenant_id()
       OR p_anchor !~ '^[0-9a-f]{64}$' OR p_body !~ '^[0-9a-f]{64}$'
       OR p_lease_seconds < 1 OR p_lease_seconds > 86400 THEN RETURN 'refused'; END IF;
    SELECT * INTO prior FROM public.trace_admission_submissions
      WHERE tenant_id=p_tenant AND submission_id=p_submission;
    IF NOT FOUND THEN RETURN 'refused'; END IF;
    IF prior.anchor_hash<>p_anchor OR prior.body_hash<>p_body THEN RETURN 'conflict'; END IF;
    SELECT a.attempt_limit,a.cost_limit INTO attempt_limit,account_limit
      FROM public.trace_admission_accounts a
      WHERE a.tenant_id=p_tenant AND a.anchor_hash=p_anchor;
    IF NOT FOUND THEN RETURN 'refused'; END IF;
    SELECT g.cost_limit INTO global_limit FROM public.trace_admission_global_budget g
      WHERE g.singleton=TRUE;
    IF NOT FOUND THEN RETURN 'refused'; END IF;
    RETURN public.trace_reserve_admission(p_tenant,p_anchor,p_submission,p_body,
      prior.receipt_hash,prior.challenge_hash,attempt_limit,account_limit,
      global_limit,prior.last_cost_bound,p_lease,p_lease_seconds);
END $$;
ALTER FUNCTION trace_resume_legacy_admission(TEXT,TEXT,UUID,TEXT,UUID,BIGINT)
    OWNER TO trace_admission_guard;
REVOKE CREATE ON SCHEMA public FROM trace_admission_guard;
REVOKE ALL ON FUNCTION trace_resume_legacy_admission(TEXT,TEXT,UUID,TEXT,UUID,BIGINT) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION trace_resume_legacy_admission(TEXT,TEXT,UUID,TEXT,UUID,BIGINT)
    TO trace_account_admission_runtime;
REVOKE trace_admission_guard FROM CURRENT_USER;
