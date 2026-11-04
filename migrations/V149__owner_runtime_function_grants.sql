-- V149: EXECUTE for an ingest login that is also the migrator.
--
-- V90 names trace_ingest_runtime as the ingest runtime's role, and a
-- deployment whose ingest login is not the migrator grants it to that login.
-- A deployment whose ingest login IS the migrator (staging: one login owns
-- the database, migrates it and serves) holds its table privileges by owning
-- the tables, and is not a member of trace_ingest_runtime. Ownership does
-- not reach a SECURITY DEFINER function owned by a guard role, so a function
-- granted to trace_ingest_runtime alone is one that login cannot call.
--
-- Seven such functions exist at V148:
--
--   * V146/V148, legacy payout carries: the account standing every payout
--     pass reads for every link calls trace_legacy_payout_carry_invalid and
--     trace_legacy_payout_carry_unproven, so on staging every live pass
--     ended payout_store_failed (permission denied, SQLSTATE 42501). The
--     other three serve the same pass and the link and release paths.
--   * V124, pipeline default routing: the two cross-tenant routing reads,
--     which fail the same way once default routing is armed.
--
-- Each also goes to CURRENT_USER, as V74 did for the public-run functions:
-- harmless where the migrator does not serve, and the only grant that
-- reaches the login where it does. The owner's rights are borrowed for the
-- grant and handed back, as the migrations that created the functions did.
-- tests/migration_atomicity_pg.rs checks that an owner-run deployment can
-- execute every function granted to trace_ingest_runtime.
--
-- Idempotent, and appliable by a non-superuser CREATEROLE migrator.

GRANT trace_legacy_payout_carry_guard TO CURRENT_USER;
GRANT EXECUTE ON FUNCTION
    public.trace_legacy_payout_carry_invalid(TEXT, UUID),
    public.trace_legacy_payout_carry_unproven(TEXT, UUID),
    public.trace_legacy_payout_proven_accounts(TEXT),
    public.trace_legacy_payout_release_unproven_carries(TEXT, TEXT, TEXT),
    public.trace_legacy_payout_source_account(TEXT)
    TO CURRENT_USER;
REVOKE trace_legacy_payout_carry_guard FROM CURRENT_USER;

GRANT trace_pipeline_routing_enumeration_guard TO CURRENT_USER;
GRANT EXECUTE ON FUNCTION
    public.trace_pipeline_unrouted_tenants(TEXT, BIGINT),
    public.trace_pipeline_routed_tenants(TEXT, BIGINT)
    TO CURRENT_USER;
REVOKE trace_pipeline_routing_enumeration_guard FROM CURRENT_USER;
