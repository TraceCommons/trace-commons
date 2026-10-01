-- Versioned pipeline follow-ups after #1143 (V105 and V106 are frozen at
-- that merge; V107 and V108 belong to #1166).
--
-- Zaki review 3, Z3-M3: a Trace Credit leg the V94-era code seeded with
-- payout `pending` has no batch line under the account's settlement key and
-- hold (`payout_eligible`, V105, stays false for it), so the payout never pays
-- it. Its payout is marked `disabled` -- a payout nothing will make (Ruling
-- T10-2) -- instead of reading `pending` for good. Legs of other instruments
-- and payout-eligible legs keep their state.
--
-- Forced row security would hide every row from the migrator here (no tenant
-- is set), so it is lifted for this one statement, inside the migration's
-- transaction, and forced again at once. No grant changes.
ALTER TABLE pipeline_run_settlements NO FORCE ROW LEVEL SECURITY;
UPDATE pipeline_run_settlements
   SET payout_state = 'disabled', updated_at = NOW()
 WHERE instrument_id = 'trace_credit'
   AND payout_state = 'pending'
   AND NOT payout_eligible;
ALTER TABLE pipeline_run_settlements FORCE ROW LEVEL SECURITY;
