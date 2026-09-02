# A credit-numbers API: one contributor's figures, and the register's

The desktop clients and the CLI want to show a contributor what their work
earned. A landing page wants to show what the register holds. These are two
different questions with two different audiences, and the only safe way to
answer both is to answer them separately.

This spec covers both surfaces. It does not cover redemption, settlement, or
anything that moves credit — see
[`2026-09-01-trace-credit-redemption-design.md`](./2026-09-01-trace-credit-redemption-design.md).

## Two surfaces, sharing nothing

| | `GET /v1/account/credit-summary` | `GET /v1/public/register-stats` |
|---|---|---|
| Audience | one contributor, about themselves | anyone |
| Auth | account session, as `/v1/account/*` | none |
| Tenant context | auth-derived, as everywhere else | none, deliberately |
| Data | one account's figures | aggregate only, never a row |
| Cache | none | server-side, minutes |

They share no handler, no struct and no query. A field added to one is not
added to the other by default, because the whole risk in this feature is one
person's figures reaching the wrong surface.

`/v1/public/register-stats` becomes the **third** unauthenticated route on the
server, after `/health` and `/v1/source`. That is a short list on purpose and it
should stay short.

## The unit problem, and why the answer is both

The design that prompted this shows `$12.40 earned, covering 30% of your bill`.
The server cannot honestly say that today.

- The ledger holds **points** (`credit_points_pending` and its siblings on
  `TraceCommonsTenantCreditResponse`), not currency.
- There is **no points-to-currency conversion anywhere in the repository**.
- Nothing has settled. The pilot holds 307 credit events, every one `pending`,
  and `settlement_state` is a write-once label rather than a lifecycle — see
  the redemption spec.
- The graded pipeline that would make a figure meaningful — quality, duplicate
  penalty, per-contributor cap — is shadow-mode.

So the response carries **points always, currency only when a deployment has
configured a rate**:

```jsonc
{
  "points": {
    "earned_this_period": 1240,
    "spent_this_period": 4190,        // inference points consumed
    "lifetime_earned": 9310
  },
  "currency": {                        // ABSENT unless a rate is configured
    "code": "USD",
    "earned_this_period": "12.40",
    "spent_this_period": "41.90"
  },
  "posture": {
    "settlement": "disabled",
    "graded": false,
    "explanation": "Credit is recorded but not settled: on-chain settlement is not enabled on this deployment, so this figure is an estimate and may be revised."
  }
}
```

Rules that make this safe:

- **`currency` is absent, not null or zero, when no rate is configured.** A
  client that sees the key shows money; a client that does not shows points. An
  absent key cannot be mistaken for "you earned nothing".
- **The client must not compute a rate.** No fallback constant, no last-known
  value. If `currency` is missing the UI says points, and the mockups' "$12.40
  covering 30%" simply does not render.
- **`posture.explanation` is derived from the live deployment**, exactly as
  `settlement_posture_explanation` already derives a receipt's line from
  `TRACE_COMMONS_NEAR_SETTLEMENT_MODE`. Reuse that function rather than writing
  a second sentence that can disagree with it.
- **`graded: false` while the credit pipeline is shadow-mode.** A client
  showing an ungraded figure must be able to say so; a figure that can be
  revised downward later should not be presented as banked.

The rate itself is one new config key, absent by default:
`TRACE_COMMONS_CREDIT_POINTS_PER_CURRENCY_UNIT` plus
`TRACE_COMMONS_CREDIT_CURRENCY_CODE`. Absent means no `currency` block. Setting
it is a deliberate operator act, and it is the moment a deployment starts
telling contributors their work is worth money.

## `GET /v1/account/credit-summary`

Authenticated exactly as `/v1/account/traces` is: account session, tenant and
actor derived from auth, never from the request. Follow
`account_traces_list_handler`'s shape rather than inventing one.

Scoped to the account's **active principal set** — the union in
`trace_account_principals`, the same expansion the credit read path already
uses. A contributor with a device key, a passkey and a NEAR key has several
`auth_principal_ref`s and must see one total, not one per credential.

Beyond the figures above:

- `by_harness`: per-source breakdown, keyed on the envelope's declared source
  (`claude-code`, `codex`, …) so the clients can put a number on each row.
  Points only, currency under the same rule.
- `period`: the window the "this period" figures cover, stated explicitly with
  its bounds rather than implied by the field name.
- `pending_review`: how many submissions are held and not yet counted, so a
  contributor whose figure looks low has somewhere to look.

**No hashes, no ids, no other account.** The response is the contributor's own
figures and nothing that could address anyone else's.

## `GET /v1/public/register-stats`

No auth, no identity, no tenant. Aggregate register facts:

```jsonc
{
  "traces_accepted": 4820,
  "contributors": 37,
  "points_issued": 512400,
  "as_of": "2026-09-01T00:00:00Z",
  "posture": { "settlement": "disabled", "graded": false }
}
```

### RLS, and why this needs a role rather than an exception

Every Trace Commons table forces RLS through `trace_current_tenant_id()`. An
unauthenticated request has no tenant, so the usual predicate matches nothing.

The wrong fixes, named so nobody reaches for them: `BYPASSRLS` on the
connection, a superuser pool, or dropping `FORCE` on a table. All three trade a
narrow read for a broad hole, and a superuser test connection would hide the
failure — a role-scoped policy that is missing looks fine until it runs as
something less privileged.

The right one, and there is precedent for the shape: a **dedicated read role**
with a role-scoped `USING (true)` SELECT policy on an aggregate view only, and
`NOBYPASSRLS` on the role. The role can read the view and nothing else. Test it
with `SET ROLE`, not as the owner, or the test proves nothing.

### A suppression floor

The pilot has few contributors. "Points issued this week" plus a known cohort
can identify one person's earnings, and a public endpoint is a standing oracle
for anyone who wants to watch a number move.

- Below a configured contributor floor, counts are returned as withheld rather
  than as a small number — an explicit `"withheld": true`, never a zero.
- Figures are **period totals at rest**, not live counters: served from a
  cached aggregate refreshed on a schedule, so the endpoint cannot be polled to
  watch a single submission land.
- No breakdown by tenant, source, model or time bucket finer than the refresh
  period. Each of those is a lever for re-identification and none of them is
  needed to say what the register holds.

### Rate limiting and cache

Unauthenticated and therefore abusable. Served from cache with a short TTL, an
`ETag`, and a per-IP limit. A cache miss must never fan out into a table scan —
the aggregate is materialised on the refresh schedule, and the handler reads
one row.

## What this does not do

- **No settlement, no redemption, no balance that can be spent.** This surface
  reports; it never moves credit.
- **No write.** Both endpoints are `GET`.
- **No per-contributor data on the public surface**, at any aggregation, ever.
- **No currency on the pilot**, because no rate is configured there — which is
  the correct state until the graded pipeline leaves shadow mode.

## Open questions

- What the contributor floor should be for suppression. It wants a number
  chosen against the real contributor count, not a guess.
- Whether `spent_this_period` is even knowable server-side today. The design
  shows inference spend, but the server sees submissions, not a contributor's
  inference bill — that figure may have to come from the inference provider, in
  which case the client composes it and this API does not carry it at all.
  **This is the one open item that could change the response shape**, and it
  should be settled before the contract is published.
- Whether `by_harness` should exist at launch or wait. It is the field most
  likely to want reshaping once real clients draw it.
