-- V157: account age carries across a live, non-pooled, device-proven link
-- to an older identity.
--
-- Owner decision O6 (earned-trust pipeline plan, 2026-11-07). The spec's
-- position that legacy `tenant-…` history earns nothing
-- (2026-09-26-earned-account-trust-design.md, "Submissions from legacy
-- identities never map") is relaxed for AGE ONLY: a NEAR account linked
-- through V81 to a legacy tenant it proved control of may count its age from
-- that tenant's first accepted contribution. Units, active weeks and attested
-- units are still earned fresh, from the account's own facts.
--
-- What proves the older identity, and nothing else:
--   * a live tenant link (trace_legacy_invite_links, revoked_at IS NULL) of
--     this account: the legacy device signed the link statement with its
--     registered key and the server countersigned it (V81, V146 for an
--     instance-enrolled device);
--   * the legacy tenant is not marked pooled;
--   * the contribution's auth_principal_ref is one the link's own claim
--     recorded for a proving device (trace_legacy_link_claimed_principals,
--     V148, under trace_legacy_link_hash(link_id)), and that device is still
--     live in the legacy tenant, `invite` origin, not NEAR-provisioned. The
--     same proof V148's trace_legacy_payout_proven_accounts reads;
--   * the submission is accepted (status accepted, expired or purged) and
--     neither withdrawn nor revoked. Its time is COALESCE(reviewed_at,
--     received_at), the time V137 gives an accepted_submission_status fact.
--
-- Pieces:
-- 1. trace_account_trust_carried_age: one row per (account, tenant link) in
--    the NEAR account's own tenant, holding the carried first-contribution
--    time. No reference to any legacy-tenant row: the tenant link it names
--    lives in the account's tenant, and deleting the link cascades. Forced
--    RLS under the usual tenant predicate.
-- 2. V114's trace_account_trust_advance_frontier is attached to it, so any
--    insert, change or removal of a carry bumps the account's evaluation
--    generation (O7: fail closed). An unchanged refresh writes nothing.
-- 3. trace_account_trust_carry_guard (NOLOGIN, NOBYPASSRLS) owns the
--    functions below. It reaches a legacy tenant only by setting the tenant
--    context inside them, so forced RLS applies to every row it reads.
-- 4. trace_account_trust_refresh_carried_age(tenant, account): called by the
--    fact-recording pass (trace_account_trust_worker) per account, under the
--    account's tenant context. It re-derives every carry of the account and
--    writes only what changed. Links made before V157 are carried by the
--    first pass after it; there is no migration-time backfill.
-- 5. Removal in the event's own transaction: revoking a tenant link and
--    marking a legacy tenant pooled each delete the carries they invalidate
--    (triggers). Both triggers take the tenant link's lock (V81, V146) before
--    deleting, as the refresh does before reading: they sort, and so fire,
--    before V146's and V148's own triggers on the same event, so they cannot
--    rely on those to have taken it. A refresh holding the lock is waited
--    for and its carry is then seen and deleted; a refresh that waits sees
--    the revocation or the pooled row and carries nothing. Deleting a link
--    cascades (its row lock conflicts with the carry's foreign-key lock, so
--    it waits for the refresh too). Every removal bumps the generation
--    through (2). A proving device revoked in the legacy tenant,
--    or a legacy submission withdrawn, is reconciled by the next refresh;
--    age only ever reaches a tier through an applied evaluation, which the
--    next pass recomputes.
-- 6. trace_account_trust_evaluation_inputs gains one arm: each live carry
--    as a 'carried_account_age' row, outcome 'carried', re-checked against
--    the link's revoked_at at read time. The rest of V156's body is
--    unchanged. The rule reads the row for age only, and only when the
--    policy opts in (carried_age), so existing policy versions keep their
--    meaning.
--
-- Hash-only: nothing here logs or stores a legacy tenant id, device key or
-- principal outside the rows that already hold them.
--
-- PostgreSQL 15 compatible: no PG16-only syntax.

-- 3. The guard.
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_account_trust_carry_guard') THEN
        CREATE ROLE trace_account_trust_carry_guard NOLOGIN NOBYPASSRLS;
    END IF;
END $$;
ALTER ROLE trace_account_trust_carry_guard NOLOGIN;
DO $$ BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles
                WHERE rolname = 'trace_account_trust_carry_guard'
                  AND (rolsuper OR rolbypassrls OR rolcanlogin)) THEN
        RAISE EXCEPTION 'V157: the carry guard is privileged or can log in';
    END IF;
END $$;
GRANT USAGE ON SCHEMA public TO trace_account_trust_carry_guard;

-- 1. Carries.
CREATE TABLE trace_account_trust_carried_age (
    tenant_id TEXT NOT NULL,
    account_id UUID NOT NULL,
    link_id UUID NOT NULL REFERENCES trace_legacy_invite_links(link_id) ON DELETE CASCADE,
    first_contribution_at TIMESTAMPTZ NOT NULL,
    carried_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (tenant_id, account_id, link_id),
    CHECK (tenant_id ~ '^(near-|nearai-)[0-9a-f]{64}$'),
    FOREIGN KEY (tenant_id, account_id)
        REFERENCES trace_accounts(tenant_id, account_id) ON DELETE CASCADE
);
CREATE INDEX trace_account_trust_carried_age_link
    ON trace_account_trust_carried_age (link_id);
ALTER TABLE trace_account_trust_carried_age ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_account_trust_carried_age FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_account_trust_carried_age;
CREATE POLICY trace_corpus_tenant_isolation ON trace_account_trust_carried_age
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

-- 2. Every carry write moves the account's generation. The frontier
-- function is closed to PUBLIC (V114), so the migrator holds its owner while
-- it attaches the trigger, as V114 did for its own.
GRANT trace_account_trust_evaluation_guard TO CURRENT_USER;
CREATE TRIGGER account_trust_carried_age_frontier
    AFTER INSERT OR UPDATE OR DELETE ON trace_account_trust_carried_age
    FOR EACH ROW EXECUTE FUNCTION trace_account_trust_advance_frontier();
REVOKE trace_account_trust_evaluation_guard FROM CURRENT_USER;

-- What the guard touches, and only that.
GRANT SELECT, INSERT, DELETE ON trace_account_trust_carried_age TO trace_account_trust_carry_guard;
GRANT UPDATE (first_contribution_at, carried_at)
    ON trace_account_trust_carried_age TO trace_account_trust_carry_guard;
GRANT SELECT (tenant_id, account_id, legacy_tenant_id, link_id, revoked_at)
    ON trace_legacy_invite_links TO trace_account_trust_carry_guard;
-- Only the pooled trigger needs it: it runs under the legacy tenant's
-- context and finds that tenant's links in the account tenants.
CREATE POLICY account_trust_carry_tenant_links ON trace_legacy_invite_links
    FOR SELECT TO trace_account_trust_carry_guard USING (TRUE);
GRANT SELECT (tenant_id) ON trace_legacy_invite_pooled_tenants TO trace_account_trust_carry_guard;
GRANT SELECT (tenant_id, legacy_link_hash, device_key_id, principal_ref)
    ON trace_legacy_link_claimed_principals TO trace_account_trust_carry_guard;
GRANT SELECT (tenant_id, device_key_id, revoked_at, onboarding_origin)
    ON device_keys TO trace_account_trust_carry_guard;
GRANT SELECT (tenant_id, device_key_id)
    ON trace_near_provisioned_devices TO trace_account_trust_carry_guard;
GRANT SELECT (tenant_id, auth_principal_ref, status, withdrawn_at, revoked_at, reviewed_at, received_at)
    ON trace_submissions TO trace_account_trust_carry_guard;
GRANT SELECT (tenant_id, account_id, closed_at) ON trace_accounts TO trace_account_trust_carry_guard;
GRANT EXECUTE ON FUNCTION public.trace_legacy_link_hash(UUID) TO trace_account_trust_carry_guard;

GRANT trace_account_trust_carry_guard TO CURRENT_USER;
GRANT CREATE ON SCHEMA public TO trace_account_trust_carry_guard;

-- 4. The refresh. Under the account's own tenant context; refused under any
-- other. Returns how many carries the account holds afterwards. Takes each
-- tenant link's own lock (V81, V146) before reading it, so a revocation
-- committed while it waited is seen, and restores the account's tenant
-- context before every write.
CREATE FUNCTION public.trace_account_trust_refresh_carried_age(p_tenant TEXT, p_account UUID)
RETURNS INTEGER
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE
    v_link RECORD;
    v_first TIMESTAMPTZ;
    v_carried INTEGER := 0;
BEGIN
    IF p_tenant IS NULL OR p_tenant IS DISTINCT FROM public.trace_current_tenant_id()
        OR p_tenant !~ '^(near-|nearai-)[0-9a-f]{64}$' OR p_account IS NULL THEN
        RAISE EXCEPTION 'account_trust_carry_refused' USING ERRCODE = '42501';
    END IF;
    -- A closed account carries nothing.
    IF NOT EXISTS (SELECT 1 FROM public.trace_accounts a
                    WHERE a.tenant_id = p_tenant AND a.account_id = p_account
                      AND a.closed_at IS NULL) THEN
        DELETE FROM public.trace_account_trust_carried_age c
         WHERE c.tenant_id = p_tenant AND c.account_id = p_account;
        RETURN 0;
    END IF;
    -- A carry whose link is no longer this account's live link goes. The
    -- triggers already remove these; this is the backstop.
    DELETE FROM public.trace_account_trust_carried_age c
     WHERE c.tenant_id = p_tenant AND c.account_id = p_account
       AND NOT EXISTS (SELECT 1 FROM public.trace_legacy_invite_links l
                        WHERE l.link_id = c.link_id AND l.tenant_id = c.tenant_id
                          AND l.account_id = c.account_id AND l.revoked_at IS NULL);
    FOR v_link IN
        SELECT l.link_id, l.legacy_tenant_id
          FROM public.trace_legacy_invite_links l
         WHERE l.tenant_id = p_tenant AND l.account_id = p_account AND l.revoked_at IS NULL
           AND l.legacy_tenant_id !~ '^(near-|nearai-)'
         ORDER BY l.legacy_tenant_id, l.link_id
    LOOP
        PERFORM pg_advisory_xact_lock(
            hashtextextended('trace_legacy_invite_link:' || v_link.legacy_tenant_id, 0));
        v_first := NULL;
        IF EXISTS (SELECT 1 FROM public.trace_legacy_invite_links l
                    WHERE l.link_id = v_link.link_id AND l.revoked_at IS NULL) THEN
            PERFORM set_config('trace_commons.trace_tenant_id', v_link.legacy_tenant_id, true);
            IF NOT EXISTS (SELECT 1 FROM public.trace_legacy_invite_pooled_tenants p
                            WHERE p.tenant_id = v_link.legacy_tenant_id) THEN
                SELECT min(COALESCE(s.reviewed_at, s.received_at)) INTO v_first
                  FROM public.trace_submissions s
                 WHERE s.tenant_id = v_link.legacy_tenant_id
                   AND s.status IN ('accepted', 'expired', 'purged')
                   AND s.withdrawn_at IS NULL AND s.revoked_at IS NULL
                   AND s.auth_principal_ref IN (
                       SELECT c.principal_ref
                         FROM public.trace_legacy_link_claimed_principals c
                        WHERE c.tenant_id = v_link.legacy_tenant_id
                          AND c.legacy_link_hash = public.trace_legacy_link_hash(v_link.link_id)
                          AND EXISTS (SELECT 1 FROM public.device_keys d
                                       WHERE d.tenant_id = c.tenant_id
                                         AND d.device_key_id = c.device_key_id
                                         AND d.revoked_at IS NULL
                                         AND d.onboarding_origin = 'invite')
                          AND NOT EXISTS (SELECT 1 FROM public.trace_near_provisioned_devices n
                                           WHERE n.tenant_id = c.tenant_id
                                             AND n.device_key_id = c.device_key_id));
            END IF;
            PERFORM set_config('trace_commons.trace_tenant_id', p_tenant, true);
        END IF;
        IF v_first IS NULL THEN
            DELETE FROM public.trace_account_trust_carried_age c
             WHERE c.tenant_id = p_tenant AND c.account_id = p_account
               AND c.link_id = v_link.link_id;
        ELSE
            INSERT INTO public.trace_account_trust_carried_age AS c
                (tenant_id, account_id, link_id, first_contribution_at)
            VALUES (p_tenant, p_account, v_link.link_id, v_first)
            ON CONFLICT (tenant_id, account_id, link_id) DO UPDATE
               SET first_contribution_at = EXCLUDED.first_contribution_at,
                   carried_at = clock_timestamp()
             WHERE c.first_contribution_at IS DISTINCT FROM EXCLUDED.first_contribution_at;
            v_carried := v_carried + 1;
        END IF;
    END LOOP;
    RETURN v_carried;
END $$;
ALTER FUNCTION public.trace_account_trust_refresh_carried_age(TEXT, UUID)
    OWNER TO trace_account_trust_carry_guard;
REVOKE ALL ON FUNCTION public.trace_account_trust_refresh_carried_age(TEXT, UUID) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.trace_account_trust_refresh_carried_age(TEXT, UUID)
    TO trace_account_trust_worker;

-- 5a. Revoking a tenant link removes its carry, in the revoking
-- transaction. SECURITY DEFINER under the link's own (account) tenant. The
-- tenant link's lock first, so a refresh that read the link live has
-- committed its carry before the DELETE takes its snapshot. Taking it
-- before any carry or frontier row lock also keeps the lock order the
-- refresh uses (advisory lock, then carry, then frontier).
CREATE FUNCTION public.trace_account_trust_link_revocation_drops_carry()
RETURNS TRIGGER
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE
    v_prior TEXT := current_setting('trace_commons.trace_tenant_id', true);
BEGIN
    IF OLD.revoked_at IS NOT NULL OR NEW.revoked_at IS NULL THEN
        RETURN NEW;
    END IF;
    PERFORM pg_advisory_xact_lock(hashtextextended('trace_legacy_invite_link:' || NEW.legacy_tenant_id, 0));
    PERFORM set_config('trace_commons.trace_tenant_id', NEW.tenant_id, true);
    DELETE FROM public.trace_account_trust_carried_age c
     WHERE c.tenant_id = NEW.tenant_id AND c.link_id = NEW.link_id;
    PERFORM set_config('trace_commons.trace_tenant_id', coalesce(v_prior, ''), true);
    RETURN NEW;
END $$;
ALTER FUNCTION public.trace_account_trust_link_revocation_drops_carry()
    OWNER TO trace_account_trust_carry_guard;
REVOKE ALL ON FUNCTION public.trace_account_trust_link_revocation_drops_carry() FROM PUBLIC;
CREATE TRIGGER trace_account_trust_link_revocation_drops_carry
    AFTER UPDATE OF revoked_at ON trace_legacy_invite_links
    FOR EACH ROW EXECUTE FUNCTION public.trace_account_trust_link_revocation_drops_carry();

-- 5b. Marking a legacy tenant pooled removes every carry from it. Runs
-- under the legacy tenant's context; each carry is deleted under its own
-- account tenant's. Takes the tenant link's lock first, as 5a does: the
-- refresh reads the pooled set under it.
CREATE FUNCTION public.trace_account_trust_pooled_tenant_drops_carries()
RETURNS TRIGGER
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE
    v_prior TEXT := current_setting('trace_commons.trace_tenant_id', true);
    v_link RECORD;
BEGIN
    PERFORM pg_advisory_xact_lock(hashtextextended('trace_legacy_invite_link:' || NEW.tenant_id, 0));
    FOR v_link IN
        SELECT l.tenant_id, l.link_id FROM public.trace_legacy_invite_links l
         WHERE l.legacy_tenant_id = NEW.tenant_id
         ORDER BY l.tenant_id, l.link_id
    LOOP
        PERFORM set_config('trace_commons.trace_tenant_id', v_link.tenant_id, true);
        DELETE FROM public.trace_account_trust_carried_age c
         WHERE c.tenant_id = v_link.tenant_id AND c.link_id = v_link.link_id;
    END LOOP;
    PERFORM set_config('trace_commons.trace_tenant_id', coalesce(v_prior, ''), true);
    RETURN NEW;
END $$;
ALTER FUNCTION public.trace_account_trust_pooled_tenant_drops_carries()
    OWNER TO trace_account_trust_carry_guard;
REVOKE ALL ON FUNCTION public.trace_account_trust_pooled_tenant_drops_carries() FROM PUBLIC;
CREATE TRIGGER trace_account_trust_pooled_tenant_drops_carries
    AFTER INSERT ON trace_legacy_invite_pooled_tenants
    FOR EACH ROW EXECUTE FUNCTION public.trace_account_trust_pooled_tenant_drops_carries();

REVOKE CREATE ON SCHEMA public FROM trace_account_trust_carry_guard;
REVOKE trace_account_trust_carry_guard FROM CURRENT_USER;

-- 6. The evaluation inputs read live carries. V156's body, plus one arm.
-- CREATE OR REPLACE keeps the owner (trace_account_admission_guard) and its
-- grants; the owner gains read of the carries and the link columns the arm
-- joins on.
GRANT trace_account_admission_guard TO CURRENT_USER;
GRANT SELECT (tenant_id, account_id, link_id, first_contribution_at)
    ON trace_account_trust_carried_age TO trace_account_admission_guard;
GRANT SELECT (tenant_id, account_id, link_id, revoked_at)
    ON trace_legacy_invite_links TO trace_account_admission_guard;
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
           -- O4: a submission the pipeline refused as a duplicate is never
           -- first in its cluster, whatever simhash cluster it formed. The
           -- gate path is closed by the outcome above (evaluated_failed);
           -- this closes the attested path, which reads first_in_cluster
           -- alone.
           CASE WHEN f.source_kind IN ('gate_evaluation','attested_submission')
                THEN public.trace_account_trust_first_in_cluster(f.tenant_id,f.submission_id)
                     AND NOT EXISTS (
                         SELECT 1 FROM public.trace_gate_decisions dup
                          WHERE dup.tenant_id=f.tenant_id AND dup.submission_id=f.submission_id
                            AND dup.source='pipeline_settle'
                            AND dup.credit_withheld_reason IN ('skipped_duplicate','cached')) END
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
    UNION ALL
    -- V157 (owner decision O6): carried account age. One row per live carry,
    -- read only while its tenant link is still unrevoked, so a revocation
    -- the triggers have not yet reached still hides it. The rule reads it
    -- for age alone, and only under a policy that sets carried_age.
    SELECT 'carried_account_age'::text,c.link_id,c.link_id,'carried'::text,
           NULL::text,c.first_contribution_at,NULL::bigint,NULL::integer,NULL::text,NULL::boolean
      FROM public.trace_account_trust_carried_age c
      JOIN public.trace_legacy_invite_links l ON l.link_id=c.link_id AND l.tenant_id=c.tenant_id
           AND l.account_id=c.account_id AND l.revoked_at IS NULL
     WHERE c.tenant_id=p_tenant AND c.account_id=p_account AND p_tenant=public.trace_current_tenant_id()
    )
    SELECT * FROM observed ORDER BY source_kind,source_id;
$$;
REVOKE trace_account_admission_guard FROM CURRENT_USER;
