# Automatic credit settlement

Credit settlement finalizes eligible ledger awards. The independent `near_ai`
payout rail converts finalized awards to NEAR AI credit and pays a confirmed
organization. Neither operation requires enabling the older on-chain NEAR
outbox or `TRACE_COMMONS_NEAR_SETTLEMENT_MODE`.

The scheduler defaults to its existing `tenant` scope, authenticated by
`TRACE_COMMONS_CREDIT_SETTLEMENT_SCHEDULER_TOKEN`. That token covers one tenant.
Deployment operators can explicitly select `all_tenants`, which uses no bearer
and exposes no new HTTP route:

```
TRACE_COMMONS_CREDIT_SETTLEMENT_SCHEDULER_ENABLED=true
TRACE_COMMONS_CREDIT_SETTLEMENT_SCHEDULER_SCOPE=all_tenants
TRACE_COMMONS_CREDIT_SETTLEMENT_SCHEDULER_POLICY_VERSION=staging-2026-10
TRACE_COMMONS_CREDIT_SETTLEMENT_SCHEDULER_DRY_RUN=false
```

The staging renderer accepts `TC_CREDIT_SETTLEMENT_ALL_TENANTS=1` together with
`TC_CREDIT_SETTLEMENT_POLICY_VERSION`. Its default remains off. Migrate first,
then grant the dedicated enumerator role to the existing runtime login:

```sql
GRANT trace_credit_settlement_enumerator TO <runtime_login>;
```

Do not grant that role to contributors or exporters. Enumeration runs under
`SET LOCAL ROLE`, reads only candidate-filter columns, and returns tenant IDs.
Normal runtime queries retain tenant RLS. Startup probes the enumeration
permission. Database mirror writes and reviewer reads must be enabled; a
tenant whose rollout gates disable database reviewer reads is refused.

The default interval is 300 seconds. Each tick processes a bounded page and
advances a cursor, including past tenants whose work is held or fails. Every
tenant uses the existing settlement kernel and its advisory lock, account cap,
credit holds, policy allowlist, issuer approval and rollout-smoke checks.
Configured central-issuer allowlists must admit the internal scheduler actor;
the scheduler never impersonates an existing credential. Tick logs contain
aggregate counts and hashed tenant identifiers only.

The stable internal actor is `principal_sha256:69f364e7d457dbb631c61eb20175ef70c9cedc721f511c0f0e532ddebb900ee1`.
It is derived as `principal_sha256:` followed by the lowercase SHA-256 hex of
the exact UTF-8 bytes `internal:credit-settlement-scheduler:v1` (no newline).
When `TRACE_COMMONS_CREDIT_SETTLEMENT_CENTRAL_ISSUER_PRINCIPAL_REFS` is configured,
explicitly admit this value. It is an identity identifier, not a bearer credential, and cannot be
used to authenticate an HTTP request.

Only accepted, positive, eligible awards are candidates. Gate novelty awards
must carry gate provenance and meet the configured settlement cutover. Changing
the scheduler does not change historical consent, award amounts, eligibility,
or the cutover. An empty candidate page is not evidence that a new upload earned
credit.

A committed settlement wakes the payout rails. Enabled rails also retry every
60 seconds. Payment still requires a confirmed recipient, a live qualified
rail, an installed rate, sufficient capacity and no account/rail holds.

Before enabling, inspect a dry-run settlement and the credit-risk summary for
the intended policy. Verify actual finalized batches and confirmed payout
lines separately after enabling; a scheduler startup log proves neither.
