-- V156: a pipeline-accepted, credited submission is a passed gate.
--
-- Owner decision O3. Under the versioned pipeline's graded novelty rule
-- (SettledNoveltyUtility), the pipeline accepts and credits submissions whose
-- perplexity_passed AND novelty_passed conjunction is false. V137's evaluation
-- inputs and V155's recorder derive a gate fact's outcome from that
-- conjunction alone, so those submissions read evaluated_failed and earn no
-- trust unit. For a gate decision the pipeline's Settle wrote
-- (source = 'pipeline_settle'), this migration derives the outcome from the
-- pipeline's own credit verdict instead:
--
--   evaluated_passed  iff the submission is accepted (unchanged: otherwise
--                     evaluated_not_accepted), AND
--                     credit_withheld_reason IS NULL or is not a duplicate
--                     label ('skipped_duplicate', 'cached': versioned_pipeline
--                     PIPELINE_SKIPPED_DUPLICATE_LABEL and
--                     PIPELINE_CACHED_DUPLICATE_LABEL), AND
--                     a trace_credit_ledger row exists for the same tenant,
--                     submission and pipeline_run_id, on the 'trace_credit'
--                     instrument, whose points_delta is a positive decimal.
--   evaluated_failed  otherwise (a zero-credit run, a duplicate-withheld
--                     run, a NoveltyUtility-withheld run).
--
-- The instrument is pinned to trace_credit: other instruments (planned
-- multi-instrument settlement) are payouts, not the pipeline's verdict on the
-- trace. event_type is not read: PipelineScore writes 'accepted', the
-- NoveltyUtility legs 'novelty_utility'. points_delta is TEXT (V1); the
-- pipeline writes it with to_credit_decimal (non-negative, no sign), so
-- positivity is two regex tests and never a cast that could raise.
--
-- Legacy rows (source = 'legacy_gate') keep perplexity_passed AND
-- novelty_passed, and keep V137's rule that a stored non-passed outcome is
-- never promoted at read time. Pipeline rows are exempt from that rule: the
-- ledger row precedes the gate row (every settlement leg is terminal before
-- commit_settle writes the gate row in its transaction, and the gate row
-- reads the leg), so a stored evaluated_failed on a pipeline row is V137's
-- conjunction, not a failed verification; V155 re-reads status facts the same
-- way. A stored evaluated_not_accepted is kept. Facts are not rewritten.
--
-- One helper, trace_account_trust_pipeline_gate_credited, holds the
-- predicate; the recorder and the evaluation inputs both call it, so the
-- stored and the re-derived outcome cannot drift. It is owned by
-- trace_account_admission_guard (the owner of both callers), SECURITY
-- DEFINER, search_path pg_catalog, tenant-checked first, with no PUBLIC
-- EXECUTE. CREATE OR REPLACE keeps the callers' owners and ACLs.
--
-- Invalidation (O7, fail-closed). The outcome now depends on
-- credit_withheld_reason (writable after the fact through
-- update_trace_gate_decision_credit_withheld_reason) and on source and
-- pipeline_run_id, so trace_account_trust_advance_gate_frontiers adds those
-- three columns to its unchanged-row test. The ledger needs no trigger:
-- pipeline rows exist before the gate row, no writer changes their
-- points_delta, pipeline_run_id or instrument_id, and they go only with
-- their submission (ON DELETE CASCADE), whose deletion already bumps. A function
-- replacement fires no trigger, and some pipeline facts read lower than
-- before (zero-credit rows that passed the conjunction), so applied
-- evaluations are invalidated once, as V137 did.
--
-- PostgreSQL 15 compatible: no PG16-only syntax.

-- The helper and its callers read these columns.
GRANT trace_account_admission_guard TO CURRENT_USER;
GRANT SELECT (credit_withheld_reason,pipeline_run_id)
    ON trace_gate_decisions TO trace_account_admission_guard;
GRANT SELECT (points_delta,pipeline_run_id,instrument_id)
    ON trace_credit_ledger TO trace_account_admission_guard;

CREATE FUNCTION trace_account_trust_pipeline_gate_credited(p_tenant TEXT,p_decision UUID)
RETURNS BOOLEAN LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
    SELECT p_tenant IS NOT DISTINCT FROM public.trace_current_tenant_id()
       AND p_decision IS NOT NULL
       AND EXISTS (
        SELECT 1 FROM public.trace_gate_decisions g
         WHERE g.tenant_id=p_tenant AND g.decision_id=p_decision
           AND g.source='pipeline_settle'
           AND (g.credit_withheld_reason IS NULL
                OR g.credit_withheld_reason NOT IN ('skipped_duplicate','cached'))
           AND EXISTS (
               SELECT 1 FROM public.trace_credit_ledger c
                WHERE c.tenant_id=g.tenant_id AND c.submission_id=g.submission_id
                  AND c.pipeline_run_id=g.pipeline_run_id
                  AND c.instrument_id='trace_credit'
                  AND c.points_delta ~ '^[0-9]+([.][0-9]+)?$'
                  AND c.points_delta ~ '[1-9]'));
$$;
GRANT CREATE ON SCHEMA public TO trace_account_admission_guard;
ALTER FUNCTION trace_account_trust_pipeline_gate_credited(TEXT,UUID) OWNER TO trace_account_admission_guard;
REVOKE CREATE ON SCHEMA public FROM trace_account_admission_guard;
REVOKE ALL ON FUNCTION trace_account_trust_pipeline_gate_credited(TEXT,UUID) FROM PUBLIC;

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
                    -- V156: the pipeline's own Settle row is read from its
                    -- credit verdict, which is fixed before the row exists.
                    -- A stored evaluated_failed there is V137's conjunction,
                    -- not a failed verification, so it is re-derived; a
                    -- stored evaluated_not_accepted is kept.
                    WHEN g.source='pipeline_settle' THEN
                        CASE WHEN f.outcome='evaluated_not_accepted' THEN f.outcome
                             WHEN public.trace_account_trust_pipeline_gate_credited(p_tenant,f.source_id)
                                 THEN 'evaluated_passed'
                             ELSE 'evaluated_failed' END
                    -- Retention does not retract a verified contribution.
                    -- Neither a retained row nor a later gate correction may
                    -- promote a legacy gate fact that was never verified as
                    -- passed.
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

CREATE OR REPLACE FUNCTION trace_record_account_trust_fact(p_tenant TEXT,p_account UUID,
    p_kind TEXT,p_source UUID) RETURNS TEXT
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE v_submission UUID; v_status TEXT; v_version TEXT; v_gate_source TEXT;
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
               g.novelty_passed,s.status,g.decided_at,g.source
          INTO v_submission,v_version,v_perplexity,v_novelty,v_status,v_occurred,v_gate_source
          FROM public.trace_gate_decisions g
          JOIN public.trace_submissions s ON s.tenant_id=g.tenant_id
               AND s.submission_id=g.submission_id
         WHERE g.tenant_id=p_tenant AND g.decision_id=p_source
           AND public.trace_account_trust_live_owned_principal(p_tenant,p_account,s.auth_principal_ref);
        IF v_submission IS NULL OR v_version IS NULL OR v_version='' THEN RETURN NULL; END IF;
        -- V156: the pipeline's own Settle row passes when the pipeline
        -- credited the submission, whatever its perplexity/novelty booleans.
        v_outcome := CASE WHEN v_status<>'accepted' THEN 'evaluated_not_accepted'
                          WHEN v_gate_source='pipeline_settle' THEN
                              CASE WHEN public.trace_account_trust_pipeline_gate_credited(p_tenant,p_source)
                                   THEN 'evaluated_passed' ELSE 'evaluated_failed' END
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

-- The outcome of a pipeline gate fact reads credit_withheld_reason, source
-- and pipeline_run_id: a change to any of them invalidates, like a change to
-- the pass booleans. Owner and trigger are unchanged.
GRANT trace_account_trust_input_guard TO CURRENT_USER;
CREATE OR REPLACE FUNCTION trace_account_trust_advance_gate_frontiers() RETURNS TRIGGER
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE v_old_tenant TEXT; v_new_tenant TEXT; v_old_submission UUID; v_new_submission UUID;
    v_old_decision UUID; v_new_decision UUID; v_old_cluster UUID; v_new_cluster UUID;
    v_clusters UUID[]; v_keys TEXT[]; v_account RECORD;
BEGIN
    IF TG_OP='UPDATE' AND
        (OLD.tenant_id,OLD.submission_id,OLD.decision_id,OLD.gate_policy_version,OLD.perplexity_passed,OLD.novelty_passed,OLD.credit_quality_micros,
         OLD.credit_quality_calibration_version,OLD.dedup_signal_version,OLD.dedup_cluster_id,OLD.decided_at,
         OLD.source,OLD.pipeline_run_id,OLD.credit_withheld_reason)
        IS NOT DISTINCT FROM
        (NEW.tenant_id,NEW.submission_id,NEW.decision_id,NEW.gate_policy_version,NEW.perplexity_passed,NEW.novelty_passed,NEW.credit_quality_micros,
         NEW.credit_quality_calibration_version,NEW.dedup_signal_version,NEW.dedup_cluster_id,NEW.decided_at,
         NEW.source,NEW.pipeline_run_id,NEW.credit_withheld_reason)
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
REVOKE trace_account_trust_input_guard FROM CURRENT_USER;

-- Applied evaluations were made under V137's pipeline outcome; invalidate
-- their generations once, without rewriting evaluations, facts or credit.
-- A later first-enable still performs V114's own bump when the switch is off.
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
