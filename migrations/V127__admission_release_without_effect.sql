-- V127 (#1237): release a `processing` admission attempt whose upload was
-- refused without effect.
--
-- V59's transition releases an attempt, and gives back the cost bound its
-- reservation took, only while it is `reserved`. Once the upload handler has
-- marked it `processing`, a refusal left it `processing` until its lease
-- ended: a retry inside the lease was refused as in progress, and a retry
-- after the lease was charged a second bound.
--
-- This adds one transition, `released_without_effect`, from `processing` to
-- `released`, that gives the latest cost bound back to the account and to the
-- global budget. The handler asks for it only for a refusal that it knows
-- wrote nothing (the versioned pipeline's receipt refusals that commit no
-- run). Every other failure after `processing` keeps V59's behaviour: plain
-- `released` still refuses a `processing` attempt, so a failure whose effects
-- are not known keeps its charge.
--
-- The window slot is not returned: `ever_processed` stays TRUE and
-- `attempt_held` is unchanged, so V59's "once processing started it can never
-- be refunded" holds for the slot, and a retry re-reserves without charging
-- another one.
--
-- The stored status is `released`, which V59's CHECK already allows; no table
-- changes. The function keeps its signature, owner (trace_admission_guard)
-- and grants: CREATE OR REPLACE preserves both. Replacing it needs the
-- owner's privileges, taken and given back exactly as V77 does.
--
-- Idempotent.

GRANT trace_admission_guard TO CURRENT_USER;

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
 ELSIF p_next='released_without_effect' AND prior.status='processing' THEN
  -- V127 (#1237): the caller knows the refusal wrote nothing. The latest
  -- cost bound is given back; the window slot is not.
  UPDATE public.trace_admission_global_budget SET cost_bound_used=cost_bound_used-prior.last_cost_bound WHERE singleton;
  UPDATE public.trace_admission_accounts SET cost_bound_used=cost_bound_used-prior.last_cost_bound
   WHERE tenant_id=p_tenant AND anchor_hash=prior.anchor_hash;
  UPDATE public.trace_admission_submissions SET status='released' WHERE tenant_id=p_tenant AND submission_id=p_submission;
  RETURN TRUE;
 END IF;
 RETURN FALSE;
END $$;

REVOKE trace_admission_guard FROM CURRENT_USER;
