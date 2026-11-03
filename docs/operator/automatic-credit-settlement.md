# Automatic credit settlement

Credit settlement finalizes eligible ledger awards. The independent `near_ai`
payout rail converts finalized awards to NEAR AI credit and pays a confirmed
organization. Neither operation requires enabling the older on-chain NEAR
outbox or `TRACE_COMMONS_NEAR_SETTLEMENT_MODE`.

## Before production payouts

Do not turn the `near_ai` payout rail to `live` for production until both
F10 and F11 in
[the one-connected-account spec](../superpowers/specs/2026-11-02-one-connected-account-design.md)
have landed (owner, 2026-11-03):

- **Accounts need a recorded NEAR AI login.** A payout link is proved
  only against one of the account's recorded NEAR AI login anchors
  (`near_ai_login`), on staging and production alike (owner, 2026-11-02:
  "It should just be NEAR AI login on both staging and production"). A
  NEAR wallet anchor proves nothing. So:
  - **Wallet-origin accounts** (`near-` tenants) link only once they have
    recorded a NEAR AI login, which the change-account flow does (**F10**).
    Until then they are refused. F10 checks the account's current identity
    before recording the new one, and for a wallet-origin account that
    identity is the NEAR wallet, so F10 needs a wallet-capable proof of
    current identity: the NEAR AI wallet-placeholder match #1862 removed
    from the payout check is not available to it.
  - **Principal-linked legacy and pilot accounts** (tenants outside
    `near-` and `nearai-`) have no NEAR AI login anchor either. Migrating
    the non-pooled ones is **F11** in the same spec, which names the two
    candidate designs.
  - **Pooled Devfolio tenants** (`trace_legacy_invite_pooled_tenants`) are
    excluded from payouts by design; they need no migration.

Links already made on accounts with no recorded NEAR AI login are revoked
at deploy by migration V147, on the `near_ai` rail. This covers every such
account, principal-linked legacy and pilot accounts included, which stop
being paid until F11. Each live link is marked recipient-unavailable and
unlink-requested, so the pass sends nothing more and closes it (one zero
send per link out of the pass's send window), with a `trace_payout_audit`
row labelled `near_ai_login_required`. The same migration drops those
accounts' pending link and unlink requests, with no audit row of their own.
The new service's confirm also refuses a link on an account with no NEAR AI
login, so a request started before the deploy cannot land after it. Stop
the old service before the new one starts: an old pass can still act on
links it loaded before V147 committed.

Before deploying, count what V147 will revoke (hash-only; RLS is forced, so
use a cross-tenant role or run per tenant):

```sql
SELECT tenant_id, state, count(*) FROM trace_payout_links l
 WHERE l.rail_id = 'near_ai' AND l.state IN ('waiting', 'active')
   AND NOT EXISTS (SELECT 1 FROM trace_near_account_anchors a
                    WHERE a.tenant_id = l.tenant_id AND a.account_id = l.account_id
                      AND a.identity_source = 'near_ai_login')
 GROUP BY tenant_id, state;
```

After it, see what it revoked:

```sql
SELECT tenant_id, prior_state, count(*) FROM trace_payout_audit
 WHERE label = 'near_ai_login_required'
 GROUP BY tenant_id, prior_state;
```

## Scheduling

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
