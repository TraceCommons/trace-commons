-- Repair deployments that already recorded the pre-revocation V59. Fresh V59
-- already granted EXECUTE WITH GRANT OPTION; do not circularly re-grant it.
DO $$ BEGIN
 IF pg_has_role(current_user,'trace_admission_guard','MEMBER') THEN
  GRANT EXECUTE ON FUNCTION trace_reserve_admission(TEXT,TEXT,UUID,TEXT,TEXT,TEXT,BIGINT,BIGINT,BIGINT,BIGINT,UUID,BIGINT) TO CURRENT_USER WITH GRANT OPTION;
  GRANT EXECUTE ON FUNCTION trace_transition_admission(TEXT,UUID,UUID,TEXT) TO CURRENT_USER WITH GRANT OPTION;
  REVOKE trace_admission_guard FROM CURRENT_USER;
 END IF;
END $$;
CREATE INDEX IF NOT EXISTS trace_admission_challenges_expiry ON trace_admission_challenges(tenant_id, expires_at);
-- Bounded expiry cleanup only; durable replay and budget history is never pruned.
DO $$ BEGIN
 IF NOT EXISTS(SELECT 1 FROM pg_roles WHERE rolname='trace_onboarding_retention_guard') THEN
  CREATE ROLE trace_onboarding_retention_guard NOLOGIN NOBYPASSRLS;
 END IF;
END $$;
GRANT trace_onboarding_retention_guard TO CURRENT_USER;
GRANT USAGE ON SCHEMA public TO trace_onboarding_retention_guard;
GRANT SELECT, DELETE ON trace_near_provisioning_ceremonies, trace_admission_challenges
 TO trace_onboarding_retention_guard;
GRANT UPDATE(expires_at) ON trace_near_provisioning_ceremonies, trace_admission_challenges
 TO trace_onboarding_retention_guard;
-- Ceremonies precede tenant assignment. The retention role can see only expired
-- ceremonies, never live wallet proof material. Challenges keep canonical RLS.
DROP POLICY IF EXISTS trace_near_ceremony_expiry ON trace_near_provisioning_ceremonies;
CREATE POLICY trace_near_ceremony_expiry ON trace_near_provisioning_ceremonies
 TO trace_onboarding_retention_guard USING (expires_at <= statement_timestamp());
CREATE OR REPLACE FUNCTION trace_prune_onboarding_expiry(p_tenant TEXT,p_limit INTEGER,p_dry_run BOOLEAN)
RETURNS BIGINT LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE ceremonies BIGINT; challenges BIGINT;
BEGIN
 IF p_tenant IS NULL OR p_tenant IS DISTINCT FROM public.trace_current_tenant_id()
 OR p_limit IS NULL OR p_limit < 1 OR p_limit > 1000 OR p_dry_run IS NULL
 THEN RAISE EXCEPTION 'onboarding_retention_refused'; END IF;
 IF p_dry_run THEN
  SELECT count(*) INTO ceremonies FROM
   (SELECT 1 FROM public.trace_near_provisioning_ceremonies
    WHERE expires_at <= statement_timestamp() LIMIT p_limit) q;
  SELECT count(*) INTO challenges FROM
   (SELECT 1 FROM public.trace_admission_challenges
    WHERE tenant_id=p_tenant AND expires_at <= statement_timestamp()
    LIMIT p_limit-ceremonies) q;
 ELSE
  WITH expired AS (
   SELECT ceremony_hash FROM public.trace_near_provisioning_ceremonies
   WHERE expires_at <= statement_timestamp() ORDER BY expires_at
   LIMIT p_limit FOR UPDATE SKIP LOCKED
  ), removed AS (
   DELETE FROM public.trace_near_provisioning_ceremonies t USING expired e
   WHERE t.ceremony_hash=e.ceremony_hash RETURNING 1
  ) SELECT count(*) INTO ceremonies FROM removed;
  WITH expired AS (
   SELECT challenge_hash FROM public.trace_admission_challenges
   WHERE tenant_id=p_tenant AND expires_at <= statement_timestamp()
   ORDER BY expires_at LIMIT p_limit-ceremonies FOR UPDATE SKIP LOCKED
  ), removed AS (
   DELETE FROM public.trace_admission_challenges t USING expired e
   WHERE t.tenant_id=p_tenant AND t.challenge_hash=e.challenge_hash RETURNING 1
  ) SELECT count(*) INTO challenges FROM removed;
 END IF;
 RETURN ceremonies+challenges;
END $$;
GRANT CREATE ON SCHEMA public TO trace_onboarding_retention_guard;
ALTER FUNCTION trace_prune_onboarding_expiry(TEXT,INTEGER,BOOLEAN) OWNER TO trace_onboarding_retention_guard;
REVOKE CREATE ON SCHEMA public FROM trace_onboarding_retention_guard;
REVOKE ALL ON FUNCTION trace_prune_onboarding_expiry(TEXT,INTEGER,BOOLEAN) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION trace_prune_onboarding_expiry(TEXT,INTEGER,BOOLEAN) TO CURRENT_USER WITH GRANT OPTION;
REVOKE trace_onboarding_retention_guard FROM CURRENT_USER;
