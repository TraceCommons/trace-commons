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
    `near-` and `nearai-`) have no NEAR AI login anchor either. They are
    paid through a payout link carried from the holder's NEAR AI account
    to the legacy accounts its tenant link covers (fix F11, V146); see
    [the legacy account payout migration spec](../superpowers/specs/2026-11-03-legacy-account-payout-migration-design.md),
    section 8, for the pilot queries and rollout order, and
    [legacy-invite-migration.md](./legacy-invite-migration.md) for the
    tenant link it rides on. Roll it out before going live.
  - **Pooled Devfolio tenants** (`trace_legacy_invite_pooled_tenants`) are
    excluded from payouts by design; they need no migration.
- **Run the two legacy rollout checks and have the owner review them**
  (owner, 2026-11-03; spec section 13, "Production-payout gates"). Both
  are read-only and return counts only. RLS is forced, so run them as the
  migration owner or once per tenant.
  - **Source provenance.** Live payout links that may have been proved by
    a wallet anchor before #1862, split by whether the account holds a
    live tenant link (and so would be carried):

    ```sql
    SELECT EXISTS (SELECT 1 FROM trace_legacy_invite_links t
                    WHERE t.tenant_id = l.tenant_id AND t.account_id = l.account_id
                      AND t.revoked_at IS NULL) AS carries,
           count(*)
      FROM trace_payout_links l
     WHERE l.state IN ('waiting', 'active') AND l.unlink_requested_at IS NULL
       AND (l.created_at < '<#1862 deploy time>'
            OR EXISTS (SELECT 1 FROM trace_payout_audit a
                        WHERE a.tenant_id = l.tenant_id
                          AND a.account_id_hash = 'sha256:' || encode(sha256(convert_to(
                                  'payout-account-v1:' || l.account_id::TEXT, 'UTF8')), 'hex')
                          AND a.action = 'link_requested' AND a.label = 'near_wallet'))
     GROUP BY 1;
    ```

    V147 (#1868) revokes the ones on accounts with no recorded NEAR AI
    login. Run it again after V147: what remains with `carries = true` are
    pre-#1862 links on accounts that do hold a NEAR AI login (including
    pre-#1841 links, and any wallet-proved link on such an account). V147
    cannot tell how those were proved. Run it on the pilot first; if it
    finds any such rows, stop and ask the owner before going live (owner,
    2026-11-03).
  - **Q5.** Run Q5 from the
    [legacy payout migration spec](../superpowers/specs/2026-11-03-legacy-account-payout-migration-design.md),
    section 8 (unowned accruals in non-pooled legacy tenants), and give the
    result to the owner to review. It need not be zero: credit from
    contributor-label subjects stays unowned and unpaid by design.

Links already made on accounts with no recorded NEAR AI login are revoked
at deploy by migration V147, on the `near_ai` rail. This covers every such
account, a legacy or pilot account's own links included, but not a link
carried to it from a NEAR AI account (V146), which the NEAR AI login
proved. It makes the moves V146's carry release makes: a waiting link,
which has never sent, closes at zero; an active link gets an unlink
request, so the next pass closes it before paying anything (one zero send
per link out of that run's send window). Each gets a `trace_payout_audit`
row labelled `near_ai_login_required`. Pending requests are left to expire:
the new service's confirm refuses a link on an account with no NEAR AI
login, so a request started before the deploy cannot land after it, and a
pending unlink of a revoked active link fails its version check (one of
a closed waiting link confirms and answers that nothing is linked). Stop the old
service before the new one starts: an old pass can still act on links it
loaded before V147 committed.

Before deploying, run this count **on the pilot first**. It uses V147's own
selection, so it is what V147 will revoke (hash-only; RLS is forced, so use
a cross-tenant role or run per tenant). If it shows any legacy or pilot
account's own links (tenants outside `near-` and `nearai-`) that are being
paid today, stop and ask the owner before deploying (owner, 2026-11-03):

```sql
SELECT l.tenant_id, l.state, count(*) FROM trace_payout_links l
 WHERE l.rail_id = 'near_ai'
   AND (l.state = 'waiting'
        OR (l.state = 'active' AND l.unlink_requested_at IS NULL))
   AND NOT EXISTS (SELECT 1 FROM trace_near_account_anchors a
                    WHERE a.tenant_id = l.tenant_id AND a.account_id = l.account_id
                      AND a.identity_source = 'near_ai_login')
   AND NOT EXISTS (SELECT 1 FROM trace_legacy_payout_carries c
                    WHERE c.tenant_id = l.tenant_id AND c.link_id = l.link_id
                      AND c.released_at IS NULL)
 GROUP BY l.tenant_id, l.state;
```

V147 refuses to run, and the service does not start, if an unanchored
account's waiting link has a payout line (the store never writes one; it
would have to be resolved by hand). A `lock_timeout` failure also stops
startup; the runner does not retry, so the deploy depends on the
supervisor restarting the service.

After it, see what it revoked (the counts should match, by tenant and
state):

```sql
SELECT tenant_id, prior_state, count(*) FROM trace_payout_audit
 WHERE rail_id = 'near_ai' AND label = 'near_ai_login_required'
 GROUP BY tenant_id, prior_state;
```

A legacy account whose own link V147 revoked is paid again only through a
carry, and only at a carry event (owner, 2026-11-03, on #1879): its
holder's next payout-link confirm, a tenant link, or a new account opening
in a linked legacy tenant, and only to an account a signed device claimed.
A carry never runs on a pass, so nothing replaces the revoked link until
such an event. A revoked own link (unlink-marked or closing) no longer
blocks the carry, which then lands as its waiting successor. The pass
closes the revoked link before activating the carry, so the account is
never paid through both.

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
