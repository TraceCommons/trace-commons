-- Earned account trust goes live, and attested inference earns trust.
-- Decided by Zaki on 2026-10-11 ("yes I approve live growth"; on attestation,
-- "I think a b and c"), reversing decision 5 of
-- docs/superpowers/specs/2026-09-26-earned-account-trust-design.md.
--
-- 1. A seventh fact kind, attested_submission -> attested_provider_tee. Its
--    source is the submission. The definer function verifies it from V76's
--    witness evidence with the reader's VerifiedV2 rule
--    (db/trace_corpus_pg.rs, witness_claim_from_row): a version 2
--    certificate whose final call ran in a provider TEE, on a submission that
--    is accepted, not revoked, purged or expired, whose stored artifact_sha256
--    matches the current submitted_envelope object. A gateway receipt does not
--    show which model ran and is never this outcome. V89's ledger label is not
--    read: its contract is that nothing which gates or prices a trace reads it.
--    occurred_at is the certificate's issued_at.
-- 2. trace_record_account_trust_fact and trace_account_trust_fact_candidates
--    are replaced in place: V85's bodies, same signatures and owner, plus the
--    new branch. trace_account_admission_guard gains column SELECTs on V76, on
--    trace_object_refs, and on trace_submissions.purged_at/expires_at.
-- 3. trace_account_trust_evaluation_inputs answers first_in_cluster for an
--    attested fact as for a gate fact: an attested unit must still be the
--    earliest member of its dedup cluster.
-- 4. trace_account_trust_evaluations.attested_units, nullable: rows written
--    before this migration had no attested facts to count.
-- 5. trace_record_account_trust_evaluation_v2: the one writer the in-process
--    evaluator uses, for `shadow` and `applied` rows. An applied row carries
--    the account's input generation and is written under V114's rules
--    (external growth switched on, a REPEATABLE READ snapshot, an open
--    account, the generation unchanged), so admission's V114 read consumes it
--    and any later fact, gate or identity change invalidates it. Owned by
--    trace_account_trust_evaluation_guard; EXECUTE goes to
--    trace_account_trust_worker, which also gains EXECUTE on
--    trace_account_trust_input_generation. V86's shadow-only writer and V114's
--    external writer are unchanged.

-- 1. The fact kind.
ALTER TABLE trace_account_trust_facts
    DROP CONSTRAINT trace_account_trust_facts_source_kind_check,
    DROP CONSTRAINT trace_account_trust_facts_outcome_check,
    DROP CONSTRAINT trace_account_trust_facts_kind_outcome_check;
ALTER TABLE trace_account_trust_facts
    ADD CONSTRAINT trace_account_trust_facts_source_kind_check CHECK (source_kind IN (
        'accepted_submission','gate_evaluation','submission_withdrawn',
        'submission_revoked','submission_quarantined','abuse_penalty',
        'attested_submission')),
    ADD CONSTRAINT trace_account_trust_facts_outcome_check CHECK (outcome IN (
        'accepted','accepted_self_reviewed','accepted_approver_unknown',
        'evaluated_passed','evaluated_failed','evaluated_not_accepted',
        'withdrawn','revoked','quarantined','penalized','attested_provider_tee')),
    ADD CONSTRAINT trace_account_trust_facts_kind_outcome_check CHECK (
        (source_kind='accepted_submission'
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
     OR (source_kind='abuse_penalty' AND outcome='penalized'
            AND evaluator_version IS NULL)
     OR (source_kind='attested_submission' AND outcome='attested_provider_tee'
            AND evaluator_version IS NULL AND source_id=submission_id));

-- 2. The recorder and the candidate list.
GRANT trace_account_admission_guard TO CURRENT_USER;
GRANT SELECT (purged_at, expires_at) ON trace_submissions TO trace_account_admission_guard;
GRANT SELECT (tenant_id, submission_id, certificate_version, inference_class,
              artifact_sha256, issued_at)
    ON trace_witness_certificate_evidence TO trace_account_admission_guard;
GRANT SELECT (tenant_id, submission_id, artifact_kind, content_sha256, invalidated_at,
              deleted_at, updated_at, created_at)
    ON trace_object_refs TO trace_account_admission_guard;

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
    ELSIF p_kind='attested_submission' THEN
        -- A positive fact: it needs a live link, as acceptance does.
        SELECT s.submission_id,e.issued_at INTO v_submission,v_occurred
          FROM public.trace_submissions s
          JOIN public.trace_account_principals p ON p.tenant_id=s.tenant_id
               AND p.principal_ref=s.auth_principal_ref AND p.account_id=p_account
          JOIN public.trace_witness_certificate_evidence e ON e.tenant_id=s.tenant_id
               AND e.submission_id=s.submission_id
         WHERE s.tenant_id=p_tenant AND s.submission_id=p_source
           AND p.unlinked_at IS NULL
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

CREATE OR REPLACE FUNCTION trace_account_trust_fact_candidates(p_tenant TEXT, p_account UUID, p_limit BIGINT)
RETURNS TABLE(source_kind TEXT, source_id UUID)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
    WITH owned AS (
        SELECT s.submission_id, s.status, s.withdrawn_at, p.unlinked_at IS NULL AS live,
               s.revoked_at, s.purged_at, s.expires_at
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
        UNION ALL
        -- The recorder's full check, so a mismatched artifact is not listed
        -- (and declined) on every run.
        SELECT 'attested_submission', o.submission_id FROM owned o
          JOIN public.trace_witness_certificate_evidence e ON e.tenant_id=p_tenant
               AND e.submission_id=o.submission_id
         WHERE o.live AND o.status='accepted' AND o.revoked_at IS NULL
           AND o.purged_at IS NULL
           AND (o.expires_at IS NULL OR o.expires_at > pg_catalog.now())
           AND e.certificate_version=2 AND e.inference_class='provider_tee_final_call'
           AND e.artifact_sha256=(
               SELECT r.content_sha256 FROM public.trace_object_refs r
                WHERE r.tenant_id=p_tenant AND r.submission_id=o.submission_id
                  AND r.artifact_kind='submitted_envelope'
                  AND r.invalidated_at IS NULL AND r.deleted_at IS NULL
                ORDER BY r.updated_at DESC, r.created_at DESC LIMIT 1)
    )
    SELECT s.kind, s.id FROM sources s
     WHERE NOT EXISTS (
        SELECT 1 FROM public.trace_account_trust_facts f
         WHERE f.tenant_id=p_tenant AND f.account_id=p_account
           AND f.source_kind=s.kind AND f.source_id=s.id)
     ORDER BY s.kind, s.id
     LIMIT LEAST(GREATEST(COALESCE(p_limit,0),0),10000);
$$;

-- 3. First-in-cluster for attested facts.
CREATE OR REPLACE FUNCTION trace_account_trust_evaluation_inputs(p_tenant TEXT, p_account UUID)
RETURNS TABLE(source_kind TEXT, source_id UUID, submission_id UUID, outcome TEXT,
              evaluator_version TEXT, occurred_at TIMESTAMPTZ,
              credit_quality_micros BIGINT, credit_quality_calibration_version INTEGER,
              dedup_signal_version TEXT, first_in_cluster BOOLEAN)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
    SELECT f.source_kind, f.source_id, f.submission_id, f.outcome, f.evaluator_version,
           f.occurred_at, g.credit_quality_micros, g.credit_quality_calibration_version,
           g.dedup_signal_version,
           CASE WHEN f.source_kind IN ('gate_evaluation','attested_submission')
                THEN public.trace_account_trust_first_in_cluster(f.tenant_id, f.submission_id)
           END
      FROM public.trace_account_trust_facts f
      LEFT JOIN public.trace_gate_decisions g ON f.source_kind='gate_evaluation'
           AND g.tenant_id=f.tenant_id AND g.decision_id=f.source_id
     WHERE f.tenant_id=p_tenant AND f.account_id=p_account
       AND p_tenant=public.trace_current_tenant_id()
     ORDER BY f.source_kind, f.source_id;
$$;
REVOKE trace_account_admission_guard FROM CURRENT_USER;

-- 4. The attested count on an evaluation.
ALTER TABLE trace_account_trust_evaluations
    ADD COLUMN attested_units INTEGER CHECK (attested_units >= 0);

-- 5. The writer for shadow and applied rows.
GRANT trace_account_trust_evaluation_guard TO CURRENT_USER;
CREATE FUNCTION trace_record_account_trust_evaluation_v2(
    p_tenant TEXT, p_evaluation UUID, p_account UUID, p_policy_version TEXT, p_mode TEXT,
    p_as_of TIMESTAMPTZ, p_tier INTEGER, p_effective_allowance BIGINT, p_units INTEGER,
    p_units_capped INTEGER, p_attested_units INTEGER, p_active_weeks INTEGER,
    p_age_weeks INTEGER, p_netted_withdrawn INTEGER, p_netted_revoked INTEGER,
    p_netted_quarantined INTEGER, p_penalty_active BOOLEAN, p_digest TEXT,
    p_expected_generation BIGINT)
RETURNS BOOLEAN
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE v_previous INTEGER; v_generation BIGINT;
BEGIN
    IF p_tenant IS NULL OR p_tenant IS DISTINCT FROM public.trace_current_tenant_id() THEN
        RAISE EXCEPTION USING MESSAGE='account_trust_evaluation_unauthorized';
    END IF;
    IF p_mode IS NULL OR p_mode NOT IN ('shadow','applied') OR p_as_of IS NULL
       OR NOT pg_catalog.isfinite(p_as_of) THEN
        RAISE EXCEPTION USING MESSAGE='account_trust_evaluation_invalid';
    END IF;
    IF p_mode='applied' THEN
        -- V114's rules for a row admission consumes. The generation binds the
        -- evaluation to its inputs only if every read shared one snapshot.
        IF pg_catalog.current_setting('transaction_isolation') NOT IN ('repeatable read','serializable') THEN
            RAISE EXCEPTION USING MESSAGE='account_trust_evaluation_snapshot_required';
        END IF;
        PERFORM 1 FROM public.trace_account_trust_external_growth WHERE enabled FOR SHARE;
        IF NOT FOUND THEN
            RAISE EXCEPTION USING MESSAGE='account_trust_external_growth_disabled';
        END IF;
        IF p_as_of > pg_catalog.transaction_timestamp() THEN
            RAISE EXCEPTION USING MESSAGE='account_trust_evaluation_invalid';
        END IF;
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
    END IF;
    PERFORM pg_catalog.pg_advisory_xact_lock(
        pg_catalog.hashtextextended('trace-account-trust-evaluation:' || p_tenant || ':' || p_account::text, 0));
    SELECT e.tier INTO v_previous FROM public.trace_account_trust_evaluations e
     WHERE e.tenant_id=p_tenant AND e.account_id=p_account
       AND e.growth_policy_version=p_policy_version AND e.mode=p_mode
     ORDER BY e.as_of DESC, e.recorded_at DESC, e.evaluation_id DESC
     LIMIT 1;
    INSERT INTO public.trace_account_trust_evaluations(tenant_id, evaluation_id, account_id,
        growth_policy_version, mode, as_of, tier, previous_tier, effective_allowance, units,
        units_capped, attested_units, active_weeks, age_weeks, netted_withdrawn,
        netted_revoked, netted_quarantined, penalty_active, facts_digest, input_generation)
    VALUES (p_tenant, p_evaluation, p_account, p_policy_version, p_mode, p_as_of, p_tier,
        v_previous, p_effective_allowance, p_units, p_units_capped, p_attested_units,
        p_active_weeks, p_age_weeks, p_netted_withdrawn, p_netted_revoked,
        p_netted_quarantined, p_penalty_active, p_digest, v_generation);
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
ALTER FUNCTION trace_record_account_trust_evaluation_v2(TEXT,UUID,UUID,TEXT,TEXT,TIMESTAMPTZ,
    INTEGER,BIGINT,INTEGER,INTEGER,INTEGER,INTEGER,INTEGER,INTEGER,INTEGER,INTEGER,BOOLEAN,
    TEXT,BIGINT)
    OWNER TO trace_account_trust_evaluation_guard;
REVOKE CREATE ON SCHEMA public FROM trace_account_trust_evaluation_guard;
REVOKE ALL ON FUNCTION trace_record_account_trust_evaluation_v2(TEXT,UUID,UUID,TEXT,TEXT,
    TIMESTAMPTZ,INTEGER,BIGINT,INTEGER,INTEGER,INTEGER,INTEGER,INTEGER,INTEGER,INTEGER,
    INTEGER,BOOLEAN,TEXT,BIGINT) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION trace_record_account_trust_evaluation_v2(TEXT,UUID,UUID,TEXT,TEXT,
    TIMESTAMPTZ,INTEGER,BIGINT,INTEGER,INTEGER,INTEGER,INTEGER,INTEGER,INTEGER,INTEGER,
    INTEGER,BOOLEAN,TEXT,BIGINT) TO trace_account_trust_worker;
GRANT EXECUTE ON FUNCTION trace_account_trust_input_generation(TEXT,UUID)
    TO trace_account_trust_worker;
REVOKE trace_account_trust_evaluation_guard FROM CURRENT_USER;
