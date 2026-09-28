-- Earned account trust, M1 (docs/superpowers/specs/2026-09-26-earned-account-trust-design.md).
--
-- Widens V77's trace_account_trust_facts to the kinds the growth rule reads,
-- and records when each fact happened. Admission still reads none of it: the
-- growth rule ships in shadow (see V86), and the production policy keeps
-- "growth_rule": "none".
--
-- New kinds, each verified from its server row by the V85 definer function:
--   submission_withdrawn   -> 'withdrawn'    (trace_submissions.withdrawn_at, V43)
--   submission_revoked     -> 'revoked'      (status 'revoked', withdrawn_at NULL)
--   submission_quarantined -> 'quarantined'  (status 'quarantined')
--   abuse_penalty          -> 'penalized'    (a trace_credit_ledger abuse_penalty event)
-- and two non-qualifying outcomes of accepted_submission, recorded so the
-- exclusion is auditable rather than silent:
--   accepted_self_reviewed     approved by a principal of the same account
--   accepted_approver_unknown  the approving principal cannot be established
--
-- A submission-scoped kind names its submission as its source, so
-- source_id = submission_id is part of the kind/outcome check.
ALTER TABLE trace_account_trust_facts
    DROP CONSTRAINT trace_account_trust_facts_source_kind_check,
    DROP CONSTRAINT trace_account_trust_facts_outcome_check,
    DROP CONSTRAINT trace_account_trust_facts_check;
ALTER TABLE trace_account_trust_facts
    ADD CONSTRAINT trace_account_trust_facts_source_kind_check CHECK (source_kind IN (
        'accepted_submission','gate_evaluation','submission_withdrawn',
        'submission_revoked','submission_quarantined','abuse_penalty')),
    ADD CONSTRAINT trace_account_trust_facts_outcome_check CHECK (outcome IN (
        'accepted','accepted_self_reviewed','accepted_approver_unknown',
        'evaluated_passed','evaluated_failed','evaluated_not_accepted',
        'withdrawn','revoked','quarantined','penalized')),
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
            AND evaluator_version IS NULL));

-- occurred_at is the source row's time (credit event, gate decided_at,
-- withdrawn_at, revoked_at), never the recorder's. V77's recorded_at is when
-- the worker ran; a backfill would stamp months of history with one day and
-- put it all in one week, and the rule buckets by week.
--
-- No worker has recorded facts in production (docs/operator/account-trust.md),
-- so there should be nothing to backfill. The UPDATE below fills any rows the
-- migrating role can see from their sources. Forced RLS hides every row from a
-- non-superuser owner with no tenant context, so the rewrite that follows
-- falls back to recorded_at for anything left: ALTER COLUMN TYPE ... USING is
-- a table rewrite, which RLS does not filter, and it never leaves a NULL for
-- SET NOT NULL to trip on.
ALTER TABLE trace_account_trust_facts ADD COLUMN occurred_at TIMESTAMPTZ;
UPDATE trace_account_trust_facts f
   SET occurred_at = CASE f.source_kind
        WHEN 'accepted_submission' THEN (
            SELECT min(c.occurred_at) FROM trace_credit_ledger c
             WHERE c.tenant_id=f.tenant_id AND c.submission_id=f.submission_id
               AND c.event_type='accepted')
        WHEN 'gate_evaluation' THEN (
            SELECT g.decided_at FROM trace_gate_decisions g
             WHERE g.tenant_id=f.tenant_id AND g.decision_id=f.source_id)
        END
 WHERE f.occurred_at IS NULL;
ALTER TABLE trace_account_trust_facts
    ALTER COLUMN occurred_at TYPE TIMESTAMPTZ USING COALESCE(occurred_at, recorded_at),
    ALTER COLUMN occurred_at SET NOT NULL;
CREATE INDEX trace_account_trust_facts_account_time
    ON trace_account_trust_facts(tenant_id, account_id, occurred_at);

-- V43 keeps its withdrawal tombstone free of a foreign key to
-- trace_submissions so a hard delete of the submission stays possible
-- (V43__trace_withdrawal.sql). A trust fact must not be what blocks it. The
-- cascade removes a submission's positive and netting facts together, so the
-- account's units come out the same.
ALTER TABLE trace_account_trust_facts
    DROP CONSTRAINT trace_account_trust_facts_tenant_id_submission_id_fkey,
    ADD CONSTRAINT trace_account_trust_facts_tenant_id_submission_id_fkey
        FOREIGN KEY (tenant_id, submission_id)
        REFERENCES trace_submissions(tenant_id, submission_id) ON DELETE CASCADE;
