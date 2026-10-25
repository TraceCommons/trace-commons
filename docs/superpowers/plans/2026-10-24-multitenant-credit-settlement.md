# Multi-tenant credit settlement

1. Add a bounded PostgreSQL tenant enumerator for accepted, positive, unsettled eligible credit. Give its dedicated role only the required columns and SELECT policies; retain ordinary tenant RLS.
2. Preserve token-scoped scheduling by default. Add explicitly enabled `all_tenants` scheduling with a private internal actor and call the existing settlement kernel under each tenant. Retain caps, holds, policy and issuer checks, and per-tenant serialization. Do not enable the on-chain outbox.
3. Use a cursor to avoid starving later tenants. Continue after a tenant error, report failures, and wake existing payout rails only through the settlement kernel's existing committed-success path.
4. Add opt-in staging configuration and explain the independent NEAR AI payout rail. Verify parser/boot refusals, two-tenant settlement and replay, pagination, and database privileges with meaningful red/green regressions.
