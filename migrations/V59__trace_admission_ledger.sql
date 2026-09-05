-- Account-bound admission. All state is hash-only; no envelope or credentials.
-- Global uniqueness stores receipt digests WITHOUT tenant/account attribution.
-- A narrow NOBYPASSRLS function owner can touch these hashes and the aggregate
-- budget only; tenant tables retain FORCE RLS and the caller's tenant context.
CREATE TABLE IF NOT EXISTS trace_admission_challenges (
 tenant_id TEXT NOT NULL, anchor_hash TEXT NOT NULL CHECK(anchor_hash ~ '^[0-9a-f]{64}$'),
 challenge_hash TEXT NOT NULL CHECK(challenge_hash ~ '^[0-9a-f]{64}$'),
 expires_at TIMESTAMPTZ NOT NULL, consumed_by UUID,
 PRIMARY KEY(tenant_id, challenge_hash)
);
CREATE TABLE IF NOT EXISTS trace_admission_accounts (
 tenant_id TEXT NOT NULL, anchor_hash TEXT NOT NULL CHECK(anchor_hash ~ '^[0-9a-f]{64}$'),
 attempt_limit BIGINT NOT NULL CHECK(attempt_limit >= 0),
 cost_limit BIGINT NOT NULL CHECK(cost_limit > 0),
 attempts_used BIGINT NOT NULL DEFAULT 0 CHECK(attempts_used >= 0),
 cost_bound_used BIGINT NOT NULL DEFAULT 0 CHECK(cost_bound_used >= 0),
 PRIMARY KEY(tenant_id, anchor_hash)
);
CREATE TABLE IF NOT EXISTS trace_admission_submissions (
 tenant_id TEXT NOT NULL, submission_id UUID NOT NULL,
 anchor_hash TEXT NOT NULL CHECK(anchor_hash ~ '^[0-9a-f]{64}$'),
 body_hash TEXT NOT NULL CHECK(body_hash ~ '^[0-9a-f]{64}$'),
 kind TEXT NOT NULL CHECK(kind IN ('window','attested')),
 receipt_hash TEXT, challenge_hash TEXT,
 status TEXT NOT NULL CHECK(status IN ('reserved','processing','completed','released')),
 lease_id UUID NOT NULL, lease_expires_at TIMESTAMPTZ NOT NULL,
 last_cost_bound BIGINT NOT NULL CHECK(last_cost_bound > 0),
 attempt_held BOOLEAN NOT NULL DEFAULT FALSE, ever_processed BOOLEAN NOT NULL DEFAULT FALSE,
 PRIMARY KEY(tenant_id,submission_id),
 CHECK ((kind='window' AND receipt_hash IS NULL AND challenge_hash IS NULL) OR
        (kind='attested' AND receipt_hash ~ '^[0-9a-f]{64}$' AND challenge_hash ~ '^[0-9a-f]{64}$'))
);
CREATE TABLE IF NOT EXISTS trace_admission_receipts (
 receipt_hash TEXT PRIMARY KEY CHECK(receipt_hash ~ '^[0-9a-f]{64}$')
);
CREATE TABLE IF NOT EXISTS trace_admission_global_budget (
 singleton BOOLEAN PRIMARY KEY CHECK(singleton),
 cost_limit BIGINT NOT NULL CHECK(cost_limit > 0),
 cost_bound_used BIGINT NOT NULL CHECK(cost_bound_used >= 0)
);
DO $$ BEGIN
 IF NOT EXISTS(SELECT 1 FROM pg_roles WHERE rolname='trace_admission_guard') THEN
  CREATE ROLE trace_admission_guard NOLOGIN NOBYPASSRLS;
 END IF;
END $$;
GRANT trace_admission_guard TO CURRENT_USER;
GRANT USAGE ON SCHEMA public TO trace_admission_guard;
GRANT SELECT, INSERT, UPDATE ON trace_admission_challenges, trace_admission_accounts,
 trace_admission_submissions, trace_admission_global_budget TO trace_admission_guard;
GRANT SELECT, INSERT ON trace_admission_receipts TO trace_admission_guard;
ALTER TABLE trace_admission_challenges ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_admission_challenges FORCE ROW LEVEL SECURITY;
ALTER TABLE trace_admission_accounts ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_admission_accounts FORCE ROW LEVEL SECURITY;
ALTER TABLE trace_admission_submissions ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_admission_submissions FORCE ROW LEVEL SECURITY;
ALTER TABLE trace_admission_receipts ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_admission_receipts FORCE ROW LEVEL SECURITY;
ALTER TABLE trace_admission_global_budget ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_admission_global_budget FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS admission_challenge_tenant ON trace_admission_challenges;
CREATE POLICY admission_challenge_tenant ON trace_admission_challenges
 USING(tenant_id=trace_current_tenant_id()) WITH CHECK(tenant_id=trace_current_tenant_id());
DROP POLICY IF EXISTS admission_account_tenant ON trace_admission_accounts;
CREATE POLICY admission_account_tenant ON trace_admission_accounts
 USING(tenant_id=trace_current_tenant_id()) WITH CHECK(tenant_id=trace_current_tenant_id());
DROP POLICY IF EXISTS admission_submission_tenant ON trace_admission_submissions;
CREATE POLICY admission_submission_tenant ON trace_admission_submissions
 USING(tenant_id=trace_current_tenant_id()) WITH CHECK(tenant_id=trace_current_tenant_id());
DROP POLICY IF EXISTS admission_receipt_guard ON trace_admission_receipts;
CREATE POLICY admission_receipt_guard ON trace_admission_receipts TO trace_admission_guard
 USING(TRUE) WITH CHECK(TRUE);
DROP POLICY IF EXISTS admission_global_guard ON trace_admission_global_budget;
CREATE POLICY admission_global_guard ON trace_admission_global_budget TO trace_admission_guard
 USING(TRUE) WITH CHECK(TRUE);

CREATE OR REPLACE FUNCTION trace_reserve_admission(p_tenant TEXT,p_anchor TEXT,p_submission UUID,p_body TEXT,
 p_receipt TEXT,p_challenge TEXT,p_attempt_limit BIGINT,p_account_limit BIGINT,p_global_limit BIGINT,
 p_cost BIGINT,p_lease UUID,p_lease_seconds BIGINT) RETURNS TEXT
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE prior public.trace_admission_submissions%ROWTYPE;
 acct public.trace_admission_accounts%ROWTYPE;
 glob public.trace_admission_global_budget%ROWTYPE;
 challenge public.trace_admission_challenges%ROWTYPE;
 is_new BOOLEAN; charge_attempt BOOLEAN; receipt_added TEXT;
BEGIN
 IF p_tenant IS DISTINCT FROM public.trace_current_tenant_id() OR p_anchor !~ '^[0-9a-f]{64}$'
 OR p_body !~ '^[0-9a-f]{64}$' OR p_attempt_limit < 0 OR p_account_limit <= 0
 OR p_global_limit <= 0 OR p_cost <= 0 OR p_lease_seconds <= 0 OR p_lease_seconds > 86400
 OR (p_receipt IS NULL) <> (p_challenge IS NULL) THEN RETURN 'refused'; END IF;
 -- One fixed lock order for global, account and submission/challenge state.
 INSERT INTO public.trace_admission_global_budget VALUES(TRUE,p_global_limit,0) ON CONFLICT DO NOTHING;
 SELECT * INTO glob FROM public.trace_admission_global_budget WHERE singleton FOR UPDATE;
 IF glob.cost_limit <> p_global_limit THEN RETURN 'configuration_changed'; END IF;
 INSERT INTO public.trace_admission_accounts VALUES(p_tenant,p_anchor,p_attempt_limit,p_account_limit,0,0)
 ON CONFLICT DO NOTHING;
 SELECT * INTO acct FROM public.trace_admission_accounts
 WHERE tenant_id=p_tenant AND anchor_hash=p_anchor FOR UPDATE;
 IF acct.attempt_limit <> p_attempt_limit OR acct.cost_limit <> p_account_limit THEN RETURN 'configuration_changed'; END IF;
 SELECT * INTO prior FROM public.trace_admission_submissions
 WHERE tenant_id=p_tenant AND submission_id=p_submission FOR UPDATE;
 is_new := NOT FOUND;
 IF NOT is_new THEN
  IF prior.anchor_hash <> p_anchor OR prior.body_hash <> p_body
   OR prior.receipt_hash IS DISTINCT FROM p_receipt OR prior.challenge_hash IS DISTINCT FROM p_challenge
   THEN RETURN 'conflict'; END IF;
  IF prior.status='completed' THEN RETURN 'completed'; END IF;
  IF prior.status <> 'released' AND prior.lease_expires_at > clock_timestamp() THEN RETURN 'busy'; END IF;
  -- Expiry never releases a cost bound. A repeated processing attempt reserves
  -- ANOTHER bound, while retaining the original charge and the same attempt.
 END IF;
 IF p_cost > acct.cost_limit-acct.cost_bound_used OR p_cost > glob.cost_limit-glob.cost_bound_used
 THEN RETURN 'budget_exhausted'; END IF;
 charge_attempt := (is_new AND p_receipt IS NULL) OR (NOT is_new AND prior.kind='window' AND NOT prior.attempt_held);
 IF charge_attempt AND acct.attempts_used >= acct.attempt_limit THEN RETURN 'window_exhausted'; END IF;
 IF is_new AND p_receipt IS NOT NULL THEN
  SELECT * INTO challenge FROM public.trace_admission_challenges
   WHERE tenant_id=p_tenant AND challenge_hash=p_challenge AND anchor_hash=p_anchor FOR UPDATE;
  IF NOT FOUND OR challenge.expires_at <= clock_timestamp() OR challenge.consumed_by IS NOT NULL
   THEN RETURN 'evidence_refused'; END IF;
  INSERT INTO public.trace_admission_receipts(receipt_hash) VALUES(p_receipt)
   ON CONFLICT DO NOTHING RETURNING receipt_hash INTO receipt_added;
  IF receipt_added IS NULL THEN RETURN 'evidence_refused'; END IF;
  UPDATE public.trace_admission_challenges SET consumed_by=p_submission
   WHERE tenant_id=p_tenant AND challenge_hash=p_challenge;
 END IF;
 UPDATE public.trace_admission_global_budget SET cost_bound_used=cost_bound_used+p_cost WHERE singleton;
 UPDATE public.trace_admission_accounts SET cost_bound_used=cost_bound_used+p_cost,
  attempts_used=attempts_used+CASE WHEN charge_attempt THEN 1 ELSE 0 END
  WHERE tenant_id=p_tenant AND anchor_hash=p_anchor;
 IF is_new THEN
  INSERT INTO public.trace_admission_submissions VALUES(p_tenant,p_submission,p_anchor,p_body,
   CASE WHEN p_receipt IS NULL THEN 'window' ELSE 'attested' END,p_receipt,p_challenge,'reserved',
   p_lease,clock_timestamp()+make_interval(secs=>p_lease_seconds::double precision),p_cost,charge_attempt,FALSE);
 ELSE
  UPDATE public.trace_admission_submissions SET status='reserved',lease_id=p_lease,attempt_held=attempt_held OR charge_attempt,
   lease_expires_at=clock_timestamp()+make_interval(secs=>p_lease_seconds::double precision),last_cost_bound=p_cost
   WHERE tenant_id=p_tenant AND submission_id=p_submission;
 END IF;
 RETURN 'reserved';
END $$;

CREATE OR REPLACE FUNCTION trace_transition_admission(p_tenant TEXT,p_submission UUID,p_lease UUID,p_next TEXT)
RETURNS BOOLEAN LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE prior public.trace_admission_submissions%ROWTYPE;
BEGIN
 IF p_tenant IS DISTINCT FROM public.trace_current_tenant_id() THEN RETURN FALSE; END IF;
 -- Same global->account->submission lock order as reservation.
 PERFORM 1 FROM public.trace_admission_global_budget WHERE singleton FOR UPDATE;
 SELECT * INTO prior FROM public.trace_admission_submissions
  WHERE tenant_id=p_tenant AND submission_id=p_submission FOR UPDATE;
 IF NOT FOUND OR prior.lease_id<>p_lease THEN RETURN FALSE; END IF;
 IF p_next='processing' AND prior.status='reserved' AND prior.lease_expires_at>clock_timestamp() THEN
  UPDATE public.trace_admission_submissions SET status='processing',ever_processed=TRUE WHERE tenant_id=p_tenant AND submission_id=p_submission;
  RETURN TRUE;
 ELSIF p_next='completed' AND prior.status IN ('processing','completed') THEN
  UPDATE public.trace_admission_submissions SET status='completed' WHERE tenant_id=p_tenant AND submission_id=p_submission;
  RETURN TRUE;
 ELSIF p_next='released' AND prior.status='reserved' THEN
  -- Only a proven pre-processing failure may release its latest cost bound.
  UPDATE public.trace_admission_global_budget SET cost_bound_used=cost_bound_used-prior.last_cost_bound WHERE singleton;
  UPDATE public.trace_admission_accounts SET cost_bound_used=cost_bound_used-prior.last_cost_bound,
   attempts_used=attempts_used-CASE WHEN prior.attempt_held AND NOT prior.ever_processed THEN 1 ELSE 0 END
   WHERE tenant_id=p_tenant AND anchor_hash=prior.anchor_hash;
  -- Return an unused window slot; once processing started it can never be refunded.
  UPDATE public.trace_admission_submissions SET status='released',attempt_held=attempt_held AND ever_processed WHERE tenant_id=p_tenant AND submission_id=p_submission;
  RETURN TRUE;
 END IF;
 RETURN FALSE;
END $$;
GRANT CREATE ON SCHEMA public TO trace_admission_guard;
ALTER FUNCTION trace_reserve_admission(TEXT,TEXT,UUID,TEXT,TEXT,TEXT,BIGINT,BIGINT,BIGINT,BIGINT,UUID,BIGINT)
 OWNER TO trace_admission_guard;
ALTER FUNCTION trace_transition_admission(TEXT,UUID,UUID,TEXT) OWNER TO trace_admission_guard;
REVOKE CREATE ON SCHEMA public FROM trace_admission_guard;
REVOKE ALL ON FUNCTION trace_reserve_admission(TEXT,TEXT,UUID,TEXT,TEXT,TEXT,BIGINT,BIGINT,BIGINT,BIGINT,UUID,BIGINT) FROM PUBLIC;
REVOKE ALL ON FUNCTION trace_transition_admission(TEXT,UUID,UUID,TEXT) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION trace_reserve_admission(TEXT,TEXT,UUID,TEXT,TEXT,TEXT,BIGINT,BIGINT,BIGINT,BIGINT,UUID,BIGINT) TO CURRENT_USER;
GRANT EXECUTE ON FUNCTION trace_transition_admission(TEXT,UUID,UUID,TEXT) TO CURRENT_USER;
