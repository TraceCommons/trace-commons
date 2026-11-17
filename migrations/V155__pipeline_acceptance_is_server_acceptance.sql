-- V155: the versioned pipeline's acceptance is server acceptance.
--
-- A PipelineScore Settle writes its `accepted` credit event with actor_role
-- 'pipeline_worker' (versioned_pipeline_credit.rs PIPELINE_CREDIT_ACTOR_ROLE)
-- and no review_decision audit row. V137's two reviewer classifiers read any
-- non-'system' accepted credit event without an approval row as a human
-- acceptance that cannot name its approver ('accepted_approver_unknown'),
-- which vetoes the submission. This migration replaces both classifiers so
-- that such an event is server acceptance, like ingest's 'system' event, but
-- only when the pipeline itself accepted that submission:
--
--   * a pipeline_runs row for the submission whose Review approved it
--     (approved_revision_id IS NOT NULL). Review sets it in the same
--     transaction that moves the submission to 'accepted', before any credit
--     event exists, and nothing clears it, so the stored outcome of an
--     accepted_submission fact (recorded once) cannot race it; or
--   * the pipeline's own Settle gate decision (source 'pipeline_settle').
--
-- The role string alone is never enough: a 'pipeline_worker' event on a
-- submission with neither row, and any other non-'system' role without an
-- approval row, still fails closed as 'accepted_approver_unknown'. A
-- self-review still classifies first. Facts already stored are not
-- rewritten; status-kind outcomes are reclassified at read time by
-- trace_account_trust_evaluation_inputs (V137), which calls the replaced
-- status classifier. Owners, SECURITY DEFINER, search_path and the absent
-- PUBLIC EXECUTE are unchanged; CREATE OR REPLACE keeps the owner and ACL.

GRANT trace_account_admission_guard TO CURRENT_USER;
GRANT SELECT (tenant_id,submission_id,approved_revision_id)
    ON pipeline_runs TO trace_account_admission_guard;
GRANT SELECT (source) ON trace_gate_decisions TO trace_account_admission_guard;

CREATE OR REPLACE FUNCTION trace_account_trust_status_acceptance_outcome(p_tenant TEXT,p_account UUID,p_submission UUID)
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
              AND NOT (c.actor_role='pipeline_worker' AND (
                  EXISTS (SELECT 1 FROM public.pipeline_runs r
                           WHERE r.tenant_id=p_tenant AND r.submission_id=p_submission
                             AND r.approved_revision_id IS NOT NULL)
                  OR EXISTS (SELECT 1 FROM public.trace_gate_decisions g
                           WHERE g.tenant_id=p_tenant AND g.submission_id=p_submission
                             AND g.source='pipeline_settle')))
        ) THEN 'accepted_approver_unknown'
        ELSE 'accepted' END;
$$;
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
            -- approver, so it fails closed. V155: the pipeline's credit event
            -- ('pipeline_worker') is server acceptance too, but only for a
            -- submission the pipeline's own Review approved.
            SELECT 1 FROM public.trace_credit_ledger c
             WHERE c.tenant_id=p_tenant AND c.submission_id=v_submission
               AND c.event_type='accepted' AND c.actor_role<>'system'
               AND NOT (c.actor_role='pipeline_worker' AND (
                   EXISTS (SELECT 1 FROM public.pipeline_runs r
                            WHERE r.tenant_id=p_tenant AND r.submission_id=c.submission_id
                              AND r.approved_revision_id IS NOT NULL)
                   OR EXISTS (SELECT 1 FROM public.trace_gate_decisions g
                            WHERE g.tenant_id=p_tenant AND g.submission_id=c.submission_id
                              AND g.source='pipeline_settle')))
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
REVOKE ALL ON FUNCTION trace_record_account_trust_fact(TEXT,UUID,TEXT,UUID) FROM PUBLIC;
REVOKE trace_account_admission_guard FROM CURRENT_USER;
