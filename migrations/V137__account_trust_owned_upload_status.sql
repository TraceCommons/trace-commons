-- Status-qualified upload trust for verified device-upload ownership.
-- No historical attribution or monetary ledger rows are rewritten. The old
-- accepted_submission kind still requires its accepted credit event and
-- retains its reviewer classification. The new status kind is consumed only
-- by an explicitly gate-required growth policy.
ALTER TABLE trace_account_trust_facts
    DROP CONSTRAINT trace_account_trust_facts_source_kind_check,
    DROP CONSTRAINT trace_account_trust_facts_kind_outcome_check;
ALTER TABLE trace_account_trust_facts
    ADD CONSTRAINT trace_account_trust_facts_source_kind_check CHECK (source_kind IN (
        'accepted_submission','accepted_submission_status','gate_evaluation','submission_withdrawn',
        'submission_revoked','submission_quarantined','abuse_penalty','attested_submission')),
    ADD CONSTRAINT trace_account_trust_facts_kind_outcome_check CHECK (
        (source_kind='accepted_submission'
            AND outcome IN ('accepted','accepted_self_reviewed','accepted_approver_unknown')
            AND evaluator_version IS NULL AND source_id=submission_id)
     OR (source_kind='accepted_submission_status'
            AND outcome IN ('accepted','accepted_self_reviewed','accepted_approver_unknown')
            AND evaluator_version IS NULL AND source_id=submission_id)
     OR (source_kind='gate_evaluation'
            AND outcome IN ('evaluated_passed','evaluated_failed','evaluated_not_accepted')
            AND evaluator_version IS NOT NULL)
     OR (source_kind='submission_withdrawn' AND outcome='withdrawn'
            AND evaluator_version IS NULL AND source_id=submission_id)
     OR (source_kind='submission_revoked' AND outcome='revoked'
            AND evaluator_version IS NULL AND source_id=submission_id)
     OR (source_kind='submission_quarantined' AND outcome='quarantined'
            AND evaluator_version IS NULL AND source_id=submission_id)
     OR (source_kind='abuse_penalty' AND outcome='penalized' AND evaluator_version IS NULL)
     OR (source_kind='attested_submission' AND outcome='attested_provider_tee'
            AND evaluator_version IS NULL AND source_id=submission_id));

GRANT trace_account_admission_guard TO CURRENT_USER;
GRANT SELECT ON trace_account_owned_principals TO trace_account_admission_guard;
GRANT SELECT (tenant_id,upload_principal_ref,device_principal_ref,device_key_id)
    ON trace_device_upload_principals TO trace_account_admission_guard;
GRANT SELECT (tenant_id,account_id,closed_at) ON trace_accounts TO trace_account_admission_guard;

-- This boolean seam uses V135's authoritative view. An alias has no
-- independent membership and cannot outlive a revoked verified device.
CREATE FUNCTION trace_account_trust_live_owned_principal(p_tenant TEXT,p_account UUID,p_principal TEXT)
RETURNS BOOLEAN LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
    SELECT p_tenant=public.trace_current_tenant_id() AND EXISTS (
        SELECT 1 FROM public.trace_account_owned_principals p
          JOIN public.trace_accounts a ON a.tenant_id=p.tenant_id AND a.account_id=p.account_id
         WHERE p.tenant_id=p_tenant AND p.account_id=p_account AND p.principal_ref=p_principal
           AND a.closed_at IS NULL
           AND NOT EXISTS (
               SELECT 1 FROM public.trace_device_upload_principals u
                 JOIN public.device_keys d ON d.tenant_id=u.tenant_id AND d.device_key_id=u.device_key_id
                WHERE u.tenant_id=p_tenant AND u.upload_principal_ref=p_principal AND d.revoked_at IS NOT NULL)
           AND NOT EXISTS (
               SELECT 1 FROM public.trace_near_provisioned_devices n
                 JOIN public.device_keys d ON d.tenant_id=n.tenant_id AND d.device_key_id=n.device_key_id
                WHERE n.tenant_id=p_tenant AND n.account_id=p_account AND n.principal_ref=p_principal
                  AND d.revoked_at IS NOT NULL));
$$;
GRANT CREATE ON SCHEMA public TO trace_account_admission_guard;
ALTER FUNCTION trace_account_trust_live_owned_principal(TEXT,UUID,TEXT) OWNER TO trace_account_admission_guard;
REVOKE CREATE ON SCHEMA public FROM trace_account_admission_guard;
REVOKE ALL ON FUNCTION trace_account_trust_live_owned_principal(TEXT,UUID,TEXT) FROM PUBLIC;

-- Reviewer classification is read-only and independent of credit issuance.
-- Alias review ownership follows its verified parent, including a parent that
-- later unlinked; losing a device link cannot shed a self-review exclusion.
CREATE FUNCTION trace_account_trust_status_acceptance_outcome(p_tenant TEXT,p_account UUID,p_submission UUID)
RETURNS TEXT LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
    SELECT CASE WHEN p_tenant IS DISTINCT FROM public.trace_current_tenant_id()
                     OR p_account IS NULL OR p_submission IS NULL THEN NULL
        WHEN EXISTS (
            SELECT 1 FROM public.trace_audit_events e
             WHERE e.tenant_id=p_tenant AND e.submission_id=p_submission AND e.action='review'
               AND e.metadata_json->>'kind'='review_decision'
               AND e.metadata_json->>'resulting_status'='accepted'
               AND (EXISTS (SELECT 1 FROM public.trace_account_owned_principals p
                       WHERE p.tenant_id=p_tenant AND p.account_id=p_account AND p.principal_ref=e.actor_principal_ref)
                    OR EXISTS (SELECT 1 FROM public.trace_account_principals p
                       WHERE p.tenant_id=p_tenant AND p.account_id=p_account AND p.principal_ref=e.actor_principal_ref)
                    OR EXISTS (SELECT 1 FROM public.trace_device_upload_principals u
                       JOIN public.trace_account_principals p ON p.tenant_id=u.tenant_id
                            AND p.principal_ref=u.device_principal_ref AND p.account_id=p_account
                      WHERE u.tenant_id=p_tenant AND u.upload_principal_ref=e.actor_principal_ref))
        ) THEN 'accepted_self_reviewed'
        WHEN NOT EXISTS (
            SELECT 1 FROM public.trace_audit_events e
             WHERE e.tenant_id=p_tenant AND e.submission_id=p_submission AND e.action='review'
               AND e.metadata_json->>'kind'='review_decision'
               AND e.metadata_json->>'resulting_status'='accepted'
        ) AND EXISTS (
            SELECT 1 FROM public.trace_credit_ledger c WHERE c.tenant_id=p_tenant
              AND c.submission_id=p_submission AND c.event_type='accepted' AND c.actor_role<>'system'
        ) THEN 'accepted_approver_unknown'
        ELSE 'accepted' END;
$$;
GRANT CREATE ON SCHEMA public TO trace_account_admission_guard;
ALTER FUNCTION trace_account_trust_status_acceptance_outcome(TEXT,UUID,UUID) OWNER TO trace_account_admission_guard;
REVOKE CREATE ON SCHEMA public FROM trace_account_admission_guard;
REVOKE ALL ON FUNCTION trace_account_trust_status_acceptance_outcome(TEXT,UUID,UUID) FROM PUBLIC;

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
         WHERE s.tenant_id=p_tenant AND s.submission_id=p_source
           AND s.status='accepted'
           AND public.trace_account_trust_live_owned_principal(p_tenant,p_account,s.auth_principal_ref);
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
    ELSIF p_kind='accepted_submission_status' THEN
        -- This is server acceptance, not a promise of monetary credit. Only
        -- policies explicitly requiring the passed gate consume this kind.
        SELECT s.submission_id,COALESCE(s.reviewed_at,s.received_at)
          INTO v_submission,v_occurred FROM public.trace_submissions s
         WHERE s.tenant_id=p_tenant AND s.submission_id=p_source
           AND s.status='accepted' AND s.withdrawn_at IS NULL
           AND s.revoked_at IS NULL AND s.purged_at IS NULL
           AND (s.expires_at IS NULL OR s.expires_at > pg_catalog.now())
           AND public.trace_account_trust_live_owned_principal(p_tenant,p_account,s.auth_principal_ref);
        IF v_submission IS NULL THEN RETURN NULL; END IF;
        v_outcome := public.trace_account_trust_status_acceptance_outcome(p_tenant,p_account,v_submission);
        IF v_outcome IS NULL THEN RETURN NULL; END IF;
    ELSIF p_kind='gate_evaluation' THEN
        SELECT g.submission_id,g.gate_policy_version,g.perplexity_passed,
               g.novelty_passed,s.status,g.decided_at
          INTO v_submission,v_version,v_perplexity,v_novelty,v_status,v_occurred
          FROM public.trace_gate_decisions g
          JOIN public.trace_submissions s ON s.tenant_id=g.tenant_id
               AND s.submission_id=g.submission_id
         WHERE g.tenant_id=p_tenant AND g.decision_id=p_source
           AND public.trace_account_trust_live_owned_principal(p_tenant,p_account,s.auth_principal_ref);
        IF v_submission IS NULL OR v_version IS NULL OR v_version='' THEN RETURN NULL; END IF;
        v_outcome := CASE WHEN v_status<>'accepted' THEN 'evaluated_not_accepted'
                          WHEN v_perplexity AND v_novelty THEN 'evaluated_passed'
                          ELSE 'evaluated_failed' END;
    ELSIF p_kind IN ('submission_withdrawn','submission_revoked','submission_quarantined') THEN
        SELECT s.submission_id,s.status,s.withdrawn_at,s.revoked_at,s.reviewed_at,s.received_at
          INTO v_submission,v_status,v_withdrawn,v_revoked,v_reviewed,v_received
          FROM public.trace_submissions s
         WHERE s.tenant_id=p_tenant AND s.submission_id=p_source
           AND (public.trace_account_trust_live_owned_principal(p_tenant,p_account,s.auth_principal_ref)
                OR EXISTS (SELECT 1 FROM public.trace_account_trust_facts f
                    WHERE f.tenant_id=p_tenant AND f.account_id=p_account
                      AND f.submission_id=s.submission_id)
                OR EXISTS (SELECT 1 FROM public.trace_account_principals p
                    WHERE p.tenant_id=p_tenant AND p.account_id=p_account
                      AND p.principal_ref=s.auth_principal_ref));
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
         WHERE c.tenant_id=p_tenant AND c.credit_event_id=p_source
           AND c.event_type='abuse_penalty'
           AND (public.trace_account_trust_live_owned_principal(p_tenant,p_account,s.auth_principal_ref)
                OR EXISTS (SELECT 1 FROM public.trace_account_trust_facts f
                    WHERE f.tenant_id=p_tenant AND f.account_id=p_account
                      AND f.submission_id=s.submission_id)
                OR EXISTS (SELECT 1 FROM public.trace_account_principals p
                    WHERE p.tenant_id=p_tenant AND p.account_id=p_account
                      AND p.principal_ref=s.auth_principal_ref));
        IF v_submission IS NULL THEN RETURN NULL; END IF;
        v_outcome := 'penalized';
    ELSIF p_kind='attested_submission' THEN
        -- A positive fact: it needs a live link, as acceptance does.
        SELECT s.submission_id,e.issued_at INTO v_submission,v_occurred
          FROM public.trace_submissions s
          JOIN public.trace_witness_certificate_evidence e ON e.tenant_id=s.tenant_id
               AND e.submission_id=s.submission_id
         WHERE s.tenant_id=p_tenant AND s.submission_id=p_source
           AND public.trace_account_trust_live_owned_principal(p_tenant,p_account,s.auth_principal_ref)
           AND s.status='accepted' AND s.revoked_at IS NULL AND s.purged_at IS NULL
           AND (s.expires_at IS NULL OR s.expires_at > pg_catalog.now())
           AND e.certificate_version=2 AND e.inference_class='provider_tee_final_call'
           AND e.artifact_sha256=(
               SELECT o.content_sha256 FROM public.trace_object_refs o
                WHERE o.tenant_id=s.tenant_id AND o.submission_id=s.submission_id
                  AND o.artifact_kind='submitted_envelope'
                  AND o.invalidated_at IS NULL AND o.deleted_at IS NULL
                ORDER BY o.updated_at DESC, o.created_at DESC LIMIT 1);
        IF v_submission IS NULL THEN RETURN NULL; END IF;
        v_outcome := 'attested_provider_tee';
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

CREATE OR REPLACE FUNCTION trace_account_trust_fact_candidates(p_tenant TEXT,p_account UUID,p_limit BIGINT)
RETURNS TABLE(source_kind TEXT,source_id UUID)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
    WITH owned AS (
        SELECT s.submission_id,s.status,s.withdrawn_at,s.revoked_at,s.purged_at,s.expires_at,public.trace_account_trust_live_owned_principal(p_tenant,p_account,s.auth_principal_ref) AS live
          FROM public.trace_submissions s
         WHERE s.tenant_id=p_tenant AND p_tenant=public.trace_current_tenant_id()
           AND (public.trace_account_trust_live_owned_principal(p_tenant,p_account,s.auth_principal_ref)
                OR EXISTS(SELECT 1 FROM public.trace_account_trust_facts f
                    WHERE f.tenant_id=p_tenant AND f.account_id=p_account AND f.submission_id=s.submission_id)
                OR EXISTS(SELECT 1 FROM public.trace_account_principals p
                    WHERE p.tenant_id=p_tenant AND p.account_id=p_account AND p.principal_ref=s.auth_principal_ref))
    ), sources AS (
        SELECT 'accepted_submission'::text AS kind,o.submission_id AS id FROM owned o
         WHERE o.live AND o.status='accepted' AND EXISTS (
             SELECT 1 FROM public.trace_credit_ledger c WHERE c.tenant_id=p_tenant
               AND c.submission_id=o.submission_id AND c.event_type='accepted')
        UNION ALL
        SELECT 'accepted_submission_status',o.submission_id FROM owned o
         WHERE o.live AND o.status='accepted' AND o.withdrawn_at IS NULL AND o.revoked_at IS NULL
           AND o.purged_at IS NULL AND (o.expires_at IS NULL OR o.expires_at>pg_catalog.now())
        UNION ALL
        SELECT 'gate_evaluation',g.decision_id FROM owned o JOIN public.trace_gate_decisions g
          ON g.tenant_id=p_tenant AND g.submission_id=o.submission_id
         WHERE o.live AND g.gate_policy_version<>''
        UNION ALL
        SELECT 'submission_withdrawn',o.submission_id FROM owned o WHERE o.withdrawn_at IS NOT NULL
        UNION ALL
        SELECT 'submission_revoked',o.submission_id FROM owned o WHERE o.status='revoked' AND o.withdrawn_at IS NULL
        UNION ALL
        SELECT 'submission_quarantined',o.submission_id FROM owned o WHERE o.status='quarantined'
        UNION ALL
        SELECT 'abuse_penalty',c.credit_event_id FROM owned o JOIN public.trace_credit_ledger c
          ON c.tenant_id=p_tenant AND c.submission_id=o.submission_id WHERE c.event_type='abuse_penalty'
        UNION ALL
        SELECT 'attested_submission',o.submission_id FROM owned o
          JOIN public.trace_witness_certificate_evidence e ON e.tenant_id=p_tenant AND e.submission_id=o.submission_id
         WHERE o.live AND o.status='accepted' AND o.revoked_at IS NULL AND o.purged_at IS NULL
           AND (o.expires_at IS NULL OR o.expires_at>pg_catalog.now())
           AND e.certificate_version=2 AND e.inference_class='provider_tee_final_call'
           AND e.artifact_sha256=(SELECT r.content_sha256 FROM public.trace_object_refs r
               WHERE r.tenant_id=p_tenant AND r.submission_id=o.submission_id AND r.artifact_kind='submitted_envelope'
                 AND r.invalidated_at IS NULL AND r.deleted_at IS NULL
               ORDER BY r.updated_at DESC,r.created_at DESC LIMIT 1)
    )
    SELECT s.kind,s.id FROM sources s WHERE NOT EXISTS (
        SELECT 1 FROM public.trace_account_trust_facts f WHERE f.tenant_id=p_tenant AND f.account_id=p_account
          AND f.source_kind=s.kind AND f.source_id=s.id)
     ORDER BY s.kind,s.id LIMIT LEAST(GREATEST(COALESCE(p_limit,0),0),10000);
$$;

CREATE OR REPLACE FUNCTION trace_account_trust_evaluation_inputs(p_tenant TEXT,p_account UUID)
RETURNS TABLE(source_kind TEXT,source_id UUID,submission_id UUID,outcome TEXT,
              evaluator_version TEXT,occurred_at TIMESTAMPTZ,credit_quality_micros BIGINT,
              credit_quality_calibration_version INTEGER,dedup_signal_version TEXT,first_in_cluster BOOLEAN)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
    WITH observed AS (
    SELECT f.source_kind,f.source_id,f.submission_id,
           CASE WHEN f.source_kind='gate_evaluation' THEN
               CASE WHEN g.decision_id IS NULL OR s.status NOT IN ('accepted','expired','purged')
                        THEN 'evaluated_not_accepted'
                    -- Retention does not retract a verified contribution.
                    -- Neither a retained row nor a later gate correction may
                    -- promote a gate fact that was never verified as passed.
                    WHEN f.outcome IS DISTINCT FROM 'evaluated_passed' THEN f.outcome
                    WHEN g.perplexity_passed IS TRUE AND g.novelty_passed IS TRUE THEN 'evaluated_passed'
                    ELSE 'evaluated_failed' END
               WHEN f.source_kind='accepted_submission_status' AND NOT (
                   COALESCE(s.status IN ('accepted','expired','purged')
                     AND s.withdrawn_at IS NULL AND s.revoked_at IS NULL
                     AND public.trace_account_trust_live_owned_principal(p_tenant,p_account,s.auth_principal_ref),FALSE)
                   OR EXISTS (SELECT 1 FROM public.trace_account_trust_facts n
                       WHERE n.tenant_id=f.tenant_id AND n.account_id=f.account_id
                         AND n.submission_id=f.submission_id AND n.occurred_at>=f.occurred_at
                         AND n.source_kind IN ('submission_withdrawn','submission_revoked','submission_quarantined')))
                   -- Read-only lifecycle exclusion, never a stored fact. Its
                   -- UUID remains visible so legacy credit cannot override it
                   -- in a policy that opted into status-based qualification.
                   THEN 'accepted_status_inactive'
               WHEN f.source_kind='accepted_submission_status' AND f.outcome='accepted'
                   THEN public.trace_account_trust_status_acceptance_outcome(p_tenant,p_account,f.submission_id)
               ELSE f.outcome END,
           CASE WHEN f.source_kind='gate_evaluation' THEN g.gate_policy_version ELSE f.evaluator_version END,
           f.occurred_at,g.credit_quality_micros,g.credit_quality_calibration_version,
           g.dedup_signal_version,
           CASE WHEN f.source_kind IN ('gate_evaluation','attested_submission')
                THEN public.trace_account_trust_first_in_cluster(f.tenant_id,f.submission_id) END
      FROM public.trace_account_trust_facts f
      LEFT JOIN public.trace_submissions s ON s.tenant_id=f.tenant_id AND s.submission_id=f.submission_id
      LEFT JOIN public.trace_gate_decisions g ON f.source_kind='gate_evaluation'
           AND g.tenant_id=f.tenant_id AND g.decision_id=f.source_id
     WHERE f.tenant_id=p_tenant AND f.account_id=p_account AND p_tenant=public.trace_current_tenant_id()
    UNION ALL
    SELECT 'accepted_submission_status'::text,f.submission_id,f.submission_id,
           CASE WHEN NOT (
               COALESCE(s.status IN ('accepted','expired','purged')
                   AND s.withdrawn_at IS NULL AND s.revoked_at IS NULL
                   AND public.trace_account_trust_live_owned_principal(p_tenant,p_account,s.auth_principal_ref),FALSE)
               OR EXISTS (SELECT 1 FROM public.trace_account_trust_facts n
                   WHERE n.tenant_id=f.tenant_id AND n.account_id=f.account_id
                     AND n.submission_id=f.submission_id AND n.occurred_at>=f.occurred_at
                     AND n.source_kind IN ('submission_withdrawn','submission_revoked','submission_quarantined')))
               THEN 'accepted_status_inactive'
               WHEN f.outcome<>'accepted' THEN f.outcome
               ELSE public.trace_account_trust_status_acceptance_outcome(p_tenant,p_account,f.submission_id) END,
           NULL::text,f.occurred_at,NULL::bigint,NULL::integer,NULL::text,NULL::boolean
      FROM public.trace_account_trust_facts f
      JOIN public.trace_submissions s ON s.tenant_id=f.tenant_id AND s.submission_id=f.submission_id
     WHERE f.tenant_id=p_tenant AND f.account_id=p_account AND p_tenant=public.trace_current_tenant_id()
       AND f.source_kind='accepted_submission'
       AND NOT EXISTS (SELECT 1 FROM public.trace_account_trust_facts n
           WHERE n.tenant_id=f.tenant_id AND n.account_id=f.account_id
             AND n.submission_id=f.submission_id AND n.source_kind='accepted_submission_status')
    )
    SELECT * FROM observed ORDER BY source_kind,source_id;
$$;
REVOKE trace_account_admission_guard FROM CURRENT_USER;

-- Current gate pass/version changes are inputs too. Keep the existing
-- cross-tenant cluster/dependency locking protocol and owner intact.
GRANT trace_account_trust_input_guard TO CURRENT_USER;
GRANT SELECT (gate_policy_version,perplexity_passed,novelty_passed)
    ON trace_gate_decisions TO trace_account_trust_input_guard;
CREATE OR REPLACE FUNCTION trace_account_trust_advance_gate_frontiers() RETURNS TRIGGER
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE v_old_tenant TEXT; v_new_tenant TEXT; v_old_submission UUID; v_new_submission UUID;
    v_old_decision UUID; v_new_decision UUID; v_old_cluster UUID; v_new_cluster UUID;
    v_clusters UUID[]; v_keys TEXT[]; v_account RECORD;
BEGIN
    IF TG_OP='UPDATE' AND
        (OLD.tenant_id,OLD.submission_id,OLD.decision_id,OLD.gate_policy_version,OLD.perplexity_passed,OLD.novelty_passed,OLD.credit_quality_micros,
         OLD.credit_quality_calibration_version,OLD.dedup_signal_version,OLD.dedup_cluster_id,OLD.decided_at)
        IS NOT DISTINCT FROM
        (NEW.tenant_id,NEW.submission_id,NEW.decision_id,NEW.gate_policy_version,NEW.perplexity_passed,NEW.novelty_passed,NEW.credit_quality_micros,
         NEW.credit_quality_calibration_version,NEW.dedup_signal_version,NEW.dedup_cluster_id,NEW.decided_at)
    THEN RETURN NULL; END IF;
    -- Off until external growth is enabled: no locks, no fact scan.
    PERFORM 1 FROM public.trace_account_trust_external_growth WHERE enabled FOR SHARE;
    IF NOT FOUND THEN
        RETURN NULL;
    END IF;
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
    -- Three indexed probes, not one OR: by submission, by decision, and by
    -- every member of an affected cluster.
    FOR v_account IN
        SELECT DISTINCT a.tenant_id COLLATE "C" AS tenant_id,a.account_id FROM (
            SELECT f.tenant_id,f.account_id FROM public.trace_account_trust_facts f
             WHERE f.source_kind='gate_evaluation'
               AND (f.tenant_id,f.submission_id) IN ((v_old_tenant,v_old_submission),(v_new_tenant,v_new_submission))
            UNION ALL
            SELECT f.tenant_id,f.account_id FROM public.trace_account_trust_facts f
             WHERE f.source_kind='gate_evaluation'
               AND (f.tenant_id,f.source_id) IN ((v_old_tenant,v_old_decision),(v_new_tenant,v_new_decision))
            UNION ALL
            SELECT f.tenant_id,f.account_id FROM public.trace_gate_decisions g
              JOIN public.trace_account_trust_facts f
                ON f.tenant_id=g.tenant_id AND f.submission_id=g.submission_id
               AND f.source_kind='gate_evaluation'
             WHERE g.dedup_cluster_id=ANY(v_clusters)
        ) a
         ORDER BY 1,2
    LOOP
        UPDATE public.trace_account_trust_frontiers SET generation=generation+1
         WHERE tenant_id=v_account.tenant_id AND account_id=v_account.account_id;
    END LOOP;
    RETURN NULL;
END $$;

-- These triggers invalidate already applied evaluations when an alias,
-- verified device or submission changes underneath its recorded facts.
-- They return no data, have no login owner and read only dependency keys.
GRANT SELECT (tenant_id,submission_id,auth_principal_ref) ON trace_submissions TO trace_account_trust_input_guard;
GRANT SELECT (tenant_id,upload_principal_ref,device_principal_ref,device_key_id) ON trace_device_upload_principals TO trace_account_trust_input_guard;
GRANT SELECT (tenant_id,account_id,principal_ref,device_key_id) ON trace_near_provisioned_devices TO trace_account_trust_input_guard;
GRANT SELECT (tenant_id,account_id,principal_ref) ON trace_account_principals TO trace_account_trust_input_guard;
CREATE POLICY account_trust_input_principal_keys ON trace_account_principals FOR SELECT TO trace_account_trust_input_guard USING(TRUE);
CREATE POLICY account_trust_input_submission_keys ON trace_submissions FOR SELECT TO trace_account_trust_input_guard USING(TRUE);
CREATE POLICY account_trust_input_upload_keys ON trace_device_upload_principals FOR SELECT TO trace_account_trust_input_guard USING(TRUE);
CREATE POLICY account_trust_input_device_keys ON trace_near_provisioned_devices FOR SELECT TO trace_account_trust_input_guard USING(TRUE);
CREATE INDEX trace_account_trust_facts_submission ON trace_account_trust_facts(tenant_id,submission_id);

CREATE FUNCTION trace_account_trust_advance_upload_frontiers() RETURNS TRIGGER
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE v_old_tenant TEXT; v_new_tenant TEXT; v_old_submission UUID; v_new_submission UUID;
        v_old_principal TEXT; v_new_principal TEXT; v_old_device TEXT; v_new_device TEXT;
        v_old_parent TEXT; v_new_parent TEXT; v_account RECORD;
BEGIN
    IF TG_OP='UPDATE' AND OLD IS NOT DISTINCT FROM NEW THEN RETURN NULL; END IF;
    PERFORM 1 FROM public.trace_account_trust_external_growth WHERE enabled FOR SHARE;
    IF NOT FOUND THEN RETURN NULL; END IF;
    IF TG_OP<>'INSERT' THEN
        v_old_tenant:=OLD.tenant_id;
        IF TG_TABLE_NAME='trace_submissions' THEN v_old_submission:=OLD.submission_id;
        ELSIF TG_TABLE_NAME='trace_device_upload_principals' THEN
            v_old_principal:=OLD.upload_principal_ref; v_old_parent:=OLD.device_principal_ref;
        ELSE v_old_device:=OLD.device_key_id;
            v_old_parent:='principal_sha256:' || pg_catalog.encode(pg_catalog.sha256(pg_catalog.convert_to(
                'device:' || OLD.tenant_id || ':' || OLD.device_key_id,'UTF8')),'hex'); END IF;
    END IF;
    IF TG_OP<>'DELETE' THEN
        v_new_tenant:=NEW.tenant_id;
        IF TG_TABLE_NAME='trace_submissions' THEN v_new_submission:=NEW.submission_id;
        ELSIF TG_TABLE_NAME='trace_device_upload_principals' THEN
            v_new_principal:=NEW.upload_principal_ref; v_new_parent:=NEW.device_principal_ref;
        ELSE v_new_device:=NEW.device_key_id;
            v_new_parent:='principal_sha256:' || pg_catalog.encode(pg_catalog.sha256(pg_catalog.convert_to(
                'device:' || NEW.tenant_id || ':' || NEW.device_key_id,'UTF8')),'hex'); END IF;
    END IF;
    IF TG_TABLE_NAME='trace_submissions' THEN
        PERFORM public.trace_account_trust_lock_fact_dependencies(v_old_tenant,v_old_submission,v_new_tenant,v_new_submission);
    END IF;
    -- ONE ordered account set: discovering ownership only through existing
    -- facts misses the first-fact race. OLD alias parents survive AFTER DELETE;
    -- canonical device parents and provisioned mappings cover device cascades.
    -- The frontier update is shared with fact INSERT and the RR applied writer.
    -- Do not acquire dependency locks after it: existing order is dependencies
    -- then account frontiers, globally sorted, never the reverse.
    FOR v_account IN
        SELECT DISTINCT affected.tenant_id COLLATE "C" AS tenant_id,affected.account_id FROM (
        SELECT f.tenant_id,f.account_id
          FROM public.trace_account_trust_facts f
          LEFT JOIN public.trace_submissions s ON s.tenant_id=f.tenant_id AND s.submission_id=f.submission_id
         WHERE (f.tenant_id,f.submission_id) IN ((v_old_tenant,v_old_submission),(v_new_tenant,v_new_submission))
            OR (s.tenant_id,s.auth_principal_ref) IN ((v_old_tenant,v_old_principal),(v_new_tenant,v_new_principal))
            OR EXISTS (SELECT 1 FROM public.trace_device_upload_principals u
                 WHERE u.tenant_id=s.tenant_id AND u.upload_principal_ref=s.auth_principal_ref
                   AND (u.tenant_id,u.device_key_id) IN ((v_old_tenant,v_old_device),(v_new_tenant,v_new_device)))
            OR EXISTS (SELECT 1 FROM public.trace_near_provisioned_devices n
                 WHERE n.tenant_id=s.tenant_id AND n.principal_ref=s.auth_principal_ref
                   AND (n.tenant_id,n.device_key_id) IN ((v_old_tenant,v_old_device),(v_new_tenant,v_new_device)))
        UNION ALL
        SELECT p.tenant_id,p.account_id FROM public.trace_account_principals p
         WHERE (p.tenant_id,p.principal_ref) IN ((v_old_tenant,v_old_parent),(v_new_tenant,v_new_parent))
        UNION ALL
        SELECT n.tenant_id,n.account_id FROM public.trace_near_provisioned_devices n
         WHERE (n.tenant_id,n.device_key_id) IN ((v_old_tenant,v_old_device),(v_new_tenant,v_new_device))
        UNION ALL
        SELECT p.tenant_id,p.account_id FROM public.trace_device_upload_principals u
          JOIN public.trace_account_principals p ON p.tenant_id=u.tenant_id AND p.principal_ref=u.device_principal_ref
         WHERE (u.tenant_id,u.device_key_id) IN ((v_old_tenant,v_old_device),(v_new_tenant,v_new_device))
        ) affected ORDER BY 1,2
    LOOP
        UPDATE public.trace_account_trust_frontiers SET generation=generation+1
         WHERE tenant_id=v_account.tenant_id AND account_id=v_account.account_id;
    END LOOP;
    RETURN NULL;
END $$;
GRANT CREATE ON SCHEMA public TO trace_account_trust_input_guard;
ALTER FUNCTION trace_account_trust_advance_upload_frontiers() OWNER TO trace_account_trust_input_guard;
REVOKE CREATE ON SCHEMA public FROM trace_account_trust_input_guard;
REVOKE ALL ON FUNCTION trace_account_trust_advance_upload_frontiers() FROM PUBLIC;
CREATE TRIGGER account_trust_upload_alias_frontier AFTER INSERT OR UPDATE OR DELETE ON trace_device_upload_principals
    FOR EACH ROW EXECUTE FUNCTION trace_account_trust_advance_upload_frontiers();
CREATE TRIGGER account_trust_upload_device_frontier AFTER UPDATE OF revoked_at OR DELETE ON device_keys
    FOR EACH ROW EXECUTE FUNCTION trace_account_trust_advance_upload_frontiers();
CREATE TRIGGER account_trust_upload_submission_frontier AFTER UPDATE OF status,auth_principal_ref,withdrawn_at,revoked_at,purged_at,expires_at OR DELETE ON trace_submissions
    FOR EACH ROW EXECUTE FUNCTION trace_account_trust_advance_upload_frontiers();
REVOKE trace_account_trust_input_guard FROM CURRENT_USER;

-- A database may already have external growth enabled. Its previously applied
-- rows were evaluated under the old ownership/gate semantics; invalidate those
-- generations once, without rewriting evaluations, facts or monetary credit.
-- A later first-enable still performs V114's own bump when this switch is off.
GRANT trace_account_trust_evaluation_guard TO CURRENT_USER;
SET LOCAL ROLE trace_account_trust_evaluation_guard;
DO $$ DECLARE observed RECORD;
BEGIN
    PERFORM 1 FROM public.trace_account_trust_external_growth WHERE enabled FOR SHARE;
    IF FOUND THEN
        FOR observed IN SELECT tenant_id,account_id FROM public.trace_account_trust_frontiers
            ORDER BY tenant_id COLLATE "C",account_id
        LOOP
            UPDATE public.trace_account_trust_frontiers SET generation=generation+1
             WHERE tenant_id=observed.tenant_id AND account_id=observed.account_id;
        END LOOP;
    END IF;
END $$;
RESET ROLE;
REVOKE trace_account_trust_evaluation_guard FROM CURRENT_USER;
