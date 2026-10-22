-- Payout reversal hold decisions: one row per (tenant, rail, reversal event)
-- whose reversal accrual could not price, recorded when accrual opens the
-- `payout_reversal_unpriced` hold for it
-- (docs/superpowers/specs/2026-10-21-nearai-credit-payout-design.md,
-- section 4 "Reversals" and "Holds").
--
-- Accrual is a pure function of stored rows and replans every run. Without a
-- record of the decision, an unpriceable reversal planned its hold again on
-- every run, so a hold an operator cleared came back on the next run with
-- another hold_set audit row. A hold is opened for a reversal event only
-- when this row is first written; a later run finds the row and leaves the
-- hold as the operator left it. A new unpriceable reversal is a new row and
-- a new hold.
--
-- NUMBERING. V132 is the payout rail tables; V131 is #1536's. This is the
-- next number free on main and in every open PR.
--
-- TENANT TABLE. Carries tenant_id, forces RLS, and uses the central
-- trace_corpus_tenant_isolation policy, so it is registered in
-- TRACE_COMMONS_RLS_TABLES. It holds no account id, amount or rate: the
-- hold row and its audit row say whom it held. With no account foreign key,
-- it does not change what the unbound-account reaper (V101) may delete.
--
-- RUNTIME GRANTS, on V105's terms: append-only (SELECT, INSERT).

CREATE TABLE trace_payout_reversal_hold_decisions (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    rail_id TEXT NOT NULL CHECK (rail_id ~ '^[a-z0-9_.-]{1,64}$'),
    reversal_event_id UUID NOT NULL,
    decided_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (tenant_id, rail_id, reversal_event_id)
);

ALTER TABLE trace_payout_reversal_hold_decisions ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_payout_reversal_hold_decisions FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_payout_reversal_hold_decisions;
CREATE POLICY trace_corpus_tenant_isolation ON trace_payout_reversal_hold_decisions
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

GRANT SELECT, INSERT ON trace_payout_reversal_hold_decisions TO trace_ingest_runtime;
