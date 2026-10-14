# NEAR AI inference credit settlement — design

Status: draft, 2026-10-14, revised the same day around NEAR AI's staking-farm credit mechanism. Owner goal: settle contributor credit into NEAR AI
inference credit, built up to the point where only the NEAR AI admin key is
missing, behind configuration that fails closed without it.

Source read: `nearai/cloud-api` `origin/main` at `752e2976a7ee` (2026-10-14).
Citations of the form `cloud-api:<path>:<line>` are into that tree; bare
paths are into this repository.

## 1. What NEAR AI actually offers

### 1.1 The one credit-granting primitive: `PATCH /v1/admin/organizations/{org_id}/limits`

- Route: `cloud-api:crates/api/src/lib.rs:2263-2266`; handler
  `cloud-api:crates/api/src/routes/admin.rs:1437`. The handler's doc calls it
  the endpoint "typically called by a billing service with an admin API key
  when a customer makes a purchase" (`admin.rs:1413-1416`).
- Request (`cloud-api:crates/api/src/models.rs:4610-4621`):
  `{"type": "grant"|"payment"|"staking_farm"|"postpay", "source"?: string,
  "spendLimit": {"amount": i64, "currency": string}, "changedBy"?: string,
  "changeReason"?: string}`. `type` is a closed enum (`models.rs:4556-4565`).
- Response (`models.rs:4656-4665`): `{organization_id, type, source,
  spendLimit: {amount, scale: 9, currency}, updated_at}`.
- **Absolute, not a delta.** The repository closes the active row for
  `(organization_id, credit_type)` and inserts a new one carrying the
  requested amount (`cloud-api:crates/database/src/repositories/organization_limits.rs:73-121`).
  At most one active row per `(org, credit_type)` is enforced by
  `idx_org_limits_active_by_type` (`cloud-api:crates/database/src/migrations/sql/V0044__add_credit_type_to_organization_limits.sql:16-18`).
- **Replacement is keyed on `credit_type` alone. `source` is a label.** The
  close statement filters only on `credit_type` (`organization_limits.rs:76-80`);
  `source` is described as "nearai, stripe, hot-pay, etc." (V0044:26). Two
  writers of `grant` for one organization (a NEAR AI promotional grant and
  ours) replace each other. This is the headline hazard, ahead of "no CAS".
- **Ceilings are cumulative against lifetime consumption.** Consumption per
  type survives row replacement, "so raising a cumulative ceiling adds only
  new capacity" (`organization_limits.rs:169-172`). To add `d` we must write
  `current + d`.
- **No compare-and-swap, no idempotency key.** Nothing in `crates/api/src`
  reads `If-Match`, `Idempotency-Key` or an expected-version field (grep over
  the tree, 0 hits). The DB write takes `FOR UPDATE` on the organization row
  (`organization_limits.rs:58-62`), which serialises concurrent PATCHes but
  not a client's read-modify-write across two HTTP calls.
- **Provenance is recorded.** Each row stores `changed_by` (self-reported,
  `VARCHAR(255)`), `change_reason` (`VARCHAR(500)`)
  (`V0003__add_organization_limits_history.sql:20-21`), plus the authenticated
  `changed_by_user_id` / `changed_by_user_email` taken from the admin token's
  creator (`admin.rs:1460-1472`).
- **Units.** Integer nano-USD, scale 9, currency upper-cased
  (`models.rs:4623-4637`, `admin.rs:1468`). Validation: `0 <= amount <=
  1e18` ($1B) (`cloud-api:crates/services/src/admin/mod.rs:1497-1520`).
- **New capacity pays prior debt first.** `settle_unfunded_usage` runs in the
  same transaction (`organization_limits.rs:123`); a grant to an organization
  with unfunded overage is partly or wholly absorbed by it.
- The `staking_farm` writer is the in-tree precedent for an external feed:
  it owns a dedicated `credit_type`, writes an absolute cumulative amount and
  only ever raises it (`cloud-api:crates/database/src/repositories/organization_staking_farm_sources.rs:327-334`).
  Owning a type is what makes that writer idempotent by construction.

### 1.2 `GET /v1/admin/organizations/{org_id}/limits/history`

- Route `lib.rs:2267-2270`; handler `admin.rs:1564`. Query `limit` (default
  50), `offset`. Response `{history: [OrgLimitsHistoryEntry], total, limit,
  offset}` (`models.rs:4669-4700`); each entry has `id`, `type`, `source`,
  `spendLimit`, `effectiveFrom`, `effectiveUntil`, `changedBy`,
  `changeReason`, `changedByUserId`, `changedByUserEmail`, `createdAt`.
- Ordered `effective_from DESC` (`organization_limits.rs:272-281`).
- **404 `organization_not_found` when the organization has no rows at all**
  (`cloud-api:crates/services/src/admin/mod.rs:590-594`). An organization's
  first-ever grant must treat 404 as "no prior limit", and must confirm the
  organization exists another way (below).
- This is the reconciliation source: it is the only place a write's author is
  visible.

### 1.3 `GET /v1/admin/organizations/{org_id}/usage/balance`

- Route `lib.rs:2271-2274`; handler `admin.rs:1685`; response
  `cloud-api:crates/api/src/routes/usage.rs:86-109`, including
  `credit_limits: [{type, source, amount, consumed, available, currency}]`.
  Read-only. Returns 200 for an organization with no limits. Useful for the
  contributor-facing "available" figure, not for authorship.

### 1.4 `POST /v1/admin/organizations/{org_id}/credit/reconciliations`

- Route `lib.rs:2411-2422`. Has a real `idempotency_key`
  (`cloud-api:crates/api/src/routes/admin_credit_reconciliation.rs:19-29`),
  but it reclassifies legacy unattributed consumption between credit types
  and "does not rewrite usage logs, settle invoices, alter limits"
  (`:146-148`). **Not a grant path.** Unusable for settlement.

### 1.5 `GET|PUT /v1/admin/organizations/{org_id}/usage-discount`

- Route `lib.rs:2425-2430`. A basis-point discount on usage
  (`cloud-api:crates/api/src/routes/admin_usage_discount.rs`, request
  `discount_basis_points`, `apply_since`). Not a grant. Out of scope.

### 1.6 Admin key model

- Admin access tokens start `adm_` and are validated only as admin tokens
  (`cloud-api:crates/api/src/middleware/auth.rs:402-405`).
- **User-Agent bound.** Validation requires `user_agent = $3 OR user_agent IS
  NULL` (`cloud-api:crates/database/src/repositories/admin_access_token.rs:140`),
  where the stored value is the User-Agent of the request that minted it
  (`admin.rs:3570-3576`). A key minted from a browser fails from our client.
- **Expires.** `expires_in_hours` must be positive (`admin.rs:3584-3592`); no
  upper bound in the handler.
- **Two permissions.** `read_only` tokens may call only an explicit GET list
  (`cloud-api:crates/api/src/middleware/admin_policy.rs:34-71`), which
  includes `limits/history`, `usage/balance`, `usage-discount`,
  `organizations/{org_id}` and its `members`. Everything else, including the
  limits PATCH, needs `read_write` (`auth.rs:97-105`). Token management is
  session-only (`admin_policy.rs:31-33`). Read-only issuance can be disabled
  by NEAR AI config (`admin.rs:3594-3604`). The token creator must still pass
  the admin email-domain check (`auth.rs:477-490`).
- Every write records the token creator's user id and email as
  `changed_by_user_id/email` — the grant is attributable to whoever minted
  our key.

### 1.7 Login → organization

- Our server introspects `GET {base}/users/me`
  (`crates/trace-commons-server/src/near_ai_login.rs:50-52`), served at
  `cloud-api:crates/api/src/routes/api.rs:177` nested under `/users`
  (`api.rs:228`), handler `cloud-api:crates/api/src/routes/users.rs:172`.
- The response carries `organizations: [{id, name, description, role, is_active,
  created_at}]` (`models.rs:3170`, `:3177-3184`), ordered by join time
  (`users.rs:204-218`). **A listing failure returns an empty list with 200**
  (`users.rs:230-233`); an empty list is "unknown", never "no organizations".
- We deliberately keep only `id` and `auth_provider` from that response
  (`near_ai_login.rs:62-83`) and store only an HMAC blind index of the
  subject (`near_account_identity.rs:181-205`). **No NEAR AI organization id
  is stored anywhere in this repository today.** Recipient mapping is
  net-new work (section 3.4).

### 1.8 The 2026-09-04 memo, checked

`docs/superpowers/specs/2026-09-04-onboarding-funding-contract-design.md`
said there is no payment, refund, transfer or redemption endpoint, no
idempotency header, only per-key `spendLimit` and an admin org-limits PATCH
with no CAS (`:20`, `:83-88`, `:184-189`). **Confirmed** against current
source, with three additions: (a) `credit/reconciliations` now carries an
`idempotency_key` but is not a grant; (b) replacement is per `credit_type`,
so `source` gives no isolation; (c) `limits/history` records provenance,
which is what makes a detect-and-freeze design possible at all.

### 1.9 Verdict, and the staking-farm precedent

The limits PATCH is not a safe primitive for an external rewards source.
The 2026-10-14 first draft of this spec built a detect-and-freeze protocol
on top of it: read the prior value, write prior plus delta, read back, and
freeze on any foreign write. The owner redirected the design the same day:
staking NEAR for inference already grants credit through an admin
mechanism, and we should use the same one.

NEAR AI's staking-farm sync is that mechanism
(`cloud-api:crates/services/src/staking_farm.rs:319-430`,
`organization_staking_farm_sources.rs:286-385`):

- a dedicated `staking_farm` credit type with a single writer;
- each sync recomputes the **cumulative** credit and writes
  `max(computed, last_synced)`;
- the write happens under the organization accounting lock, after
  rechecking that the source and the organization are active, and only
  when the value grows.

Monotonic cumulative writes are idempotent and need no compare-and-swap.
This design adopts that model and proposes a `trace_commons` credit type to
NEAR AI. See
[`2026-10-14-near-ai-trace-commons-credit-type-proposal.md`](./2026-10-14-near-ai-trace-commons-credit-type-proposal.md)
for the proposal and a local cloud-api draft.

## 2. Our side today (checked)

- **Legacy outbox.** `trace_near_credit_outbox`
  (`migrations/V2__trace_credit_settlement.sql:56-73`) is fed by finalized
  `trace_credit_settlement_batches` (`V2:20-37`,
  `settled_credit_micros BIGINT`). A per-tenant scheduler drains it
  (`trace-commons-ingest.rs:45711`). Its kill switch,
  `TRACE_COMMONS_NEAR_SETTLEMENT_MODE`, defaults to `Disabled`.
- **Multi-instrument is partly in place**, contrary to the earlier memory
  note. `instrument_id` exists on the ledger, batches and outbox (V94), and
  `pipeline_run_settlements` carries `atomic_units NUMERIC(39,0)` and a
  `payout_rail`. Only `trace_credit` on rail `near` is ever paid.
- **Revocation does not claw back settled credit**
  (`trace-commons-ingest.rs:65811-65819`).
- **Tenant identifiers.** On the legacy path, tenant ids leave only as an
  unsalted, truncated SHA-256 (`trace-commons-ingest.rs:78371-78413`). This
  rail sends no tenant identifier at all.

## 3. Design

### 3.1 A cumulative ledger behind a rail seam

- **`SettlementRail`** (`settlement_rail.rs`) is held as
  `Arc<dyn SettlementRail>`. Its two operations are
  `publish(CumulativeStatement) -> Published` and
  `observe(recipient) -> Option<u128>`. The dispatcher knows only rail ids,
  instruments and cumulative totals.
- **Recipient ledger.** The per-recipient ledger is
  `trace_settlement_rail_ledgers` (V129). It holds:
  - `cumulative_units` and `cumulative_micros` (`NUMERIC(39,0)`);
  - `confirmed_units`: what the external side last confirmed holding;
  - a rounding carry and the hash of the rate the carry is denominated in;
  - a publish state.

  A trigger refuses any decrease of a total. `confirmed_units` can never
  exceed `cumulative_units`.
- **Batch credits.** `trace_settlement_rail_credits` (V129) holds one
  insert-only row per `(batch, rail)`: what that batch added, and at which
  rate. A batch accrues once.
- **Discriminators.** `rail_id` (`near_ai_trace_commons`) and
  `instrument_id` (`near_ai_credit_usd`, 9 decimals) identify the rail and
  instrument. A NEP-141 trace-credit rail or another instrument would be
  another rail id on the same tables.
- **What V129 replaces.** It replaces the first draft's outbox, carry table
  and per-delivery state machine (the never-merged V127). V127 and V128 are
  taken by open PRs.

### 3.2 Epoch / batch boundary

The unit of accrual is an existing **finalized
`trace_credit_settlement_batches` row**. Accrual adds the batch to the
active recipient's cumulative total in one transaction. Publishing is
decoupled from accrual and states the total, not the batch. A batch pays
out on exactly one path: triggers refuse a rail credit beside a NEAR
receipt outbox row for the same batch, and the reverse.

### 3.3 Conversion

- `TRACE_COMMONS_NEAR_AI_CREDIT_RATE` is `"num/den"` nano-USD per
  microcredit. It has no default; unset refuses with
  `missing_control:near_ai_credit_rate`.
- Each accrual computes `floor((micros * num + carry) / den)` in checked
  u128 arithmetic and keeps the remainder in the ledger's carry. If the rate
  changes, the carry restarts at zero (less than one atomic unit is
  forgone).
- Every credit row records the rate text and its hash, so a past accrual is
  never repriced.

### 3.4 Recipient mapping

Unchanged from the first draft. `near_ai_recipient::verify_recipient` calls
`/users/me` with the contributor's own token and accepts an `owner` or
`admin` of an active organization. An empty list is "unverified", never
"none". The election is stored in `trace_near_ai_credit_recipients`.

NEAR AI keeps one total per organization, and that total never decreases.
So an organization's total must always equal exactly one ledger, forever.
Three rules follow:

- **One active recipient per tenant.** A tenant elects one organization at
  a time.
- **One tenant per organization, ever.** The unique index on
  `org_ref_hash` is not conditional on revocation. If a second tenant
  could take over the organization, its ledger would start at zero below
  the organization's existing maximum. A fresh row for the same tenant
  would do the same, and both cases would freeze as "external ahead".
- **Rebinding reactivates.** A tenant that moves to another organization
  and back gets its original recipient and ledger again.

The org id is the one external identifier we send; audit uses
`org_ref_hash`.

### 3.5 Delivery: push or pull

| | Push (recommended now) | Pull (staking-farm style) |
|---|---|---|
| Who acts | we `PUT` our cumulative total to the proposed `trace-commons/credit` admin route | NEAR AI fetches our signed statement and applies it with the same repository function |
| Credential on our side | a NEAR AI read-write admin token, bound to the User-Agent it was minted with | none; an optional read-only token lets us reconcile |
| NEAR AI work | one credit type, one route, one repository function (drafted) | the same, plus binding, key pinning, a sync schedule, perhaps AML |
| Latency | ours to choose | theirs (schedule or preflight) |
| Built here | `PushTraceCommonsCreditRail` | `PullTraceCommonsCreditRail`, `near_ai_credit_statement` (Ed25519 over a fixed preimage), `signed_statement` |

**Recommendation: push first.** It is the smaller change for NEAR AI and
mirrors how their billing service already writes limits. The monotonic type
removes the overwrite hazard that made push unsafe in the first draft. Pull
is the later step that removes our admin credential, and a NEP-141 balance
can replace the signed statement after that.

### 3.6 Publish outcomes

A publish states `cumulative_units`. NEAR AI returns what it now holds,
which we call `effective`.

- `effective == cumulative`: confirmed. The ledger goes `idle`.
- `published <= effective < cumulative`: an accrual landed meanwhile. The
  ledger goes back to `due`.
- `effective > cumulative`: someone else wrote the type, or we lost an
  accrual. The ledger is frozen with `settlement_rail_external_ahead`.
- `effective < published`: a monotonic max cannot be below our total. The
  ledger is frozen with `settlement_rail_external_refused_total`.
- Transport failures, timeouts, 401, 429 and 5xx: retried with backoff.
  Repeating the total is harmless, including after a write that committed
  behind a failed response.
- 403 (read-only key), 404 (organization), 400: frozen.
- Pull: `Deferred`, and the ledger goes to `published`.

### 3.7 Reconciler

The reconciler observes the external total and compares it with the
ledger:

- equal to `cumulative`: verified;
- between the confirmed value and `cumulative`: behind, and published
  again;
- above `cumulative`: frozen with `settlement_rail_external_ahead`;
- below the confirmed value: frozen with
  `settlement_rail_external_regressed`.

It needs only a read-only key. Without one, a pull rail is unobservable and
is counted as such.

### 3.8 States

`publish_state` is `idle`, `due`, `leased`, `published` or `frozen`.

- Leases use `FOR UPDATE SKIP LOCKED`. Two workers publishing the same
  total are harmless anyway.
- A frozen ledger keeps accruing and publishes nothing until an operator
  unfreezes it.
- Unfreezing without fixing the cause freezes it again on the next publish.

### 3.9 Audit

- Labels only (`^[a-z0-9_]{1,64}$`) and hashes only: `org_ref_hash`,
  `rate_ref_hash`, and `statement_ref`, a hash of rail, recipient hash and
  total.
- No tenant id, org id or admin email appears in any log line or stored
  label.
- HTTP errors carry a status class only, never a body.

### 3.10 Kill switch and config

| Variable | Required | Meaning |
|---|---|---|
| `TRACE_COMMONS_NEAR_AI_CREDIT_RAIL` | — | `disabled` (default, also any unknown value), `dry_run`, `live` |
| `TRACE_COMMONS_NEAR_AI_CREDIT_DELIVERY` | — | `push` (default) or `pull`; anything else refuses |
| `TRACE_COMMONS_NEAR_AI_ADMIN_BASE_URL` | push; pull with a read key | API root including `/v1`; `https://` (loopback http for tests) |
| `TRACE_COMMONS_NEAR_AI_ADMIN_KEY` | push `live` | read-write admin token |
| `TRACE_COMMONS_NEAR_AI_ADMIN_READ_KEY` | push `dry_run` without the write key; optional otherwise | read-only admin token |
| `TRACE_COMMONS_NEAR_AI_ADMIN_USER_AGENT` | whenever an admin key is used | exact UA the key was minted with |
| `TRACE_COMMONS_NEAR_AI_CREDIT_STATEMENT_KEY` | pull | base64 PKCS#8 Ed25519 signing key |
| `TRACE_COMMONS_NEAR_AI_CREDIT_RATE` | always | `num/den` |
| `TRACE_COMMONS_NEAR_AI_CREDIT_MAX_CUMULATIVE_NANO_USD` | always | per-recipient ceiling, at most 1e18; above it freezes |
| `TRACE_COMMONS_NEAR_AI_ADMIN_TIMEOUT_MS` | — | 100 to 120000, default 15000 |

Every refusal is `missing_control:<name>`, and there is no fallback. Ingest
refuses to start on an enabled rail with a missing control. The kill switch
at `disabled` stops new publishes.

### 3.11 What needs which credential

| Needs nothing | Read-only key | Read-write key |
|---|---|---|
| accrual, conversion, recipient binding (contributor's own token), pull statements, fake-server tests | reconciler; push `dry_run` | push `live` |

## 4. What this slice built

`settlement_rail.rs`, `near_ai_admin.rs`, `near_ai_inference_credit.rs`
(both rails and the config), `near_ai_credit_statement.rs`,
`near_ai_recipient.rs`, `settlement_rail_store.rs`, `V129`, the ingest boot
refusal, a fake of the proposed route, and the runbook stub.

Deferred:

- the contributor binding route and UI;
- a worker route and scheduler that call `accrue_batch`, `publish_tick` and
  `reconcile_tick`;
- the pull statement HTTP route;
- an operator route for `unfreeze`;
- migrating the legacy NEAR outbox onto the rail tables.

## 5. Open questions for the owner

1. **Send the proposal and the draft PR?** Recommendation: yes. The draft
   is small and in their style. It is the precondition for going live.
2. **Push or pull.** Recommendation: push now, and propose pull as the
   follow-up that removes our admin credential.
3. **Conversion or addition?** Recommendation: conversion. A batch pays on
   exactly one path, which is enforced.
4. **Rate.** Recommendation: set by the owner, with its hash recorded on
   every credit row and past accruals never repriced.
5. **Which roles may receive.** Recommendation: `owner` or `admin` only.
6. **Keys.** Recommendation: a read-write key for push and a read-only key
   for the reconciler, both minted with our exact User-Agent and rotated
   before expiry. NEAR AI attributes each write to the admin who minted the
   key.
7. **Unfunded debt.** New capacity first pays an organization's prior
   overage. Recommendation: say so in contributor copy.
8. **Recipient id at rest.** Recommendation: envelope-encrypt the org id
   before production.
