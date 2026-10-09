-- V114 seeded one trust-input frontier row per existing account with
--     INSERT INTO trace_account_trust_frontiers(tenant_id,account_id)
--         SELECT tenant_id,account_id FROM trace_accounts;
-- trace_accounts has forced row security and a migration runs with no tenant
-- context, so a migrator that is neither superuser nor BYPASSRLS sees no
-- account and the backfill inserts nothing. The pilot applied V114 that way
-- (INSERT 0 0). Only accounts created after V114 have a row, because the
-- frontier trigger creates one on account insert and only advances it after.
-- An account without a row cannot record an external trust evaluation
-- (trace_record_external_account_trust_evaluation refuses a NULL generation),
-- so once external growth is enabled every older account is stuck at its
-- current tier.
--
-- The backfill runs here as trace_account_trust_evaluation_guard, which V114
-- already lets write every tenant's frontier row. Its read of trace_accounts is
-- a policy created for this statement and dropped in the same transaction; it
-- gains no lasting cross-tenant read. ON CONFLICT DO NOTHING keeps the rows
-- the trigger already made, with their generations. On a database V114
-- backfilled correctly (a superuser or BYPASSRLS migrator) this inserts nothing.
GRANT trace_account_trust_evaluation_guard TO CURRENT_USER;
CREATE POLICY account_trust_frontier_backfill ON trace_accounts
    FOR SELECT TO trace_account_trust_evaluation_guard USING (TRUE);
SET LOCAL ROLE trace_account_trust_evaluation_guard;
INSERT INTO public.trace_account_trust_frontiers(tenant_id,account_id)
    SELECT tenant_id,account_id FROM public.trace_accounts
    ON CONFLICT (tenant_id,account_id) DO NOTHING;
RESET ROLE;
DROP POLICY account_trust_frontier_backfill ON trace_accounts;
REVOKE trace_account_trust_evaluation_guard FROM CURRENT_USER;
