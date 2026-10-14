# NEAR AI inference credit settlement — design

Status: draft, 2026-10-14. Owner goal: settle contributor credit into NEAR AI
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
  net-new work (section 4).

### 1.8 The 2026-09-04 memo, checked

`docs/superpowers/specs/2026-09-04-onboarding-funding-contract-design.md`
said there is no payment, refund, transfer or redemption endpoint, no
idempotency header, only per-key `spendLimit` and an admin org-limits PATCH
with no CAS (`:20`, `:83-88`, `:184-189`). **Confirmed** against current
source, with three additions: (a) `credit/reconciliations` now carries an
`idempotency_key` but is not a grant; (b) replacement is per `credit_type`,
so `source` gives no isolation; (c) `limits/history` records provenance,
which is what makes a detect-and-freeze design possible at all.

### 1.9 Verdict

Not a safe primitive on its own, but a **detectable** one: every write
leaves a history row with our `changedBy`/`changeReason`, so we can prove
whether our write landed, whether anyone else wrote the bucket, and whether
ours was overwritten. We build on it only with (i) a precondition that the
active `grant` row is absent or ours, (ii) a correlation marker in
`changeReason`, (iii) read-back verification and (iv) a reconciler, with
every anomaly a freeze rather than a retry. The durable fix is a dedicated
credit type for Trace Commons (Q1), which turns every write into the
staking-farm pattern.

## 2. Our side today (checked)

- Legacy outbox: `trace_near_credit_outbox` (`migrations/V2__trace_credit_settlement.sql:56-73`),
  fed by finalized `trace_credit_settlement_batches` (`V2:20-37`,
  `settled_credit_micros BIGINT`), drained by a per-tenant scheduler
  (`run_trace_near_credit_outbox_scheduler_tick`, `trace-commons-ingest.rs:45711`),
  kill switch `TRACE_COMMONS_NEAR_SETTLEMENT_MODE` defaulting to `Disabled`
  (`trace-commons-ingest.rs:26845-26867`). No backoff columns.
- `near_credit.rs` builds non-transferable contract calls only; `ft_transfer`
  is refused by allowlist (`near_credit.rs:303-316`). Amount `i64` micros
  (`:25`).
- Multi-instrument is **partly** in place, contrary to the memory note:
  `instrument_id` exists on the ledger, batches and outbox (V94:33-106), and
  `pipeline_run_settlements` carries `instrument_id`, `atomic_units
  NUMERIC(39,0)` with a u128 bound, and a `payout_rail` (`V94:108-235`). Only
  `trace_credit` on rail `near` is ever paid (`versioned_pipeline.rs:3955-3962`).
  No per-instrument decimals column exists.
- Revocation does not claw back settled credit (`trace-commons-ingest.rs:65811-65819`,
  `docs/trace-commons.md:1293`, `:1300`).
- Tenant ids leave only as `tenant_sha256:` + 16 bytes of an unsalted SHA-256
  (`trace-commons-ingest.rs:78371-78413`). Safe for random `near-`/`nearai-`
  ids, guessable for low-entropy legacy ids. The new rail sends **no tenant
  identifier at all**.

## 3. Design

### 3.1 Shape: a rail, not another NEAR-only path

- New generic table `trace_settlement_rail_outbox`, discriminated by
  `rail_id` and `instrument_id`, with `atomic_units NUMERIC(39,0)` and
  `instrument_decimals SMALLINT`. The NEAR AI rail is `rail_id =
  'near_ai_org_limits'`, `instrument_id = 'near_ai_credit_usd'`, decimals 9.
- A `SettlementRail` trait (`crates/trace-commons-server/src/settlement_rail.rs`)
  held as `Arc<dyn SettlementRail>`; the dispatcher knows nothing NEAR
  AI-specific. Below it, `NearAiAdminApi` is a second trait object so the
  HTTP client and the test fake are interchangeable.
- The legacy NEAR outbox is not migrated onto the new table in this slice;
  the spec leaves that as a follow-up once the new table has run in
  production.

### 3.2 Epoch / batch boundary

The unit of settlement is an existing **finalized
`trace_credit_settlement_batches` row** for a tenant. A batch is converted to
one rail outbox row per elected rail. A batch settles on exactly one payout
path: a trigger refuses a rail row for a batch that already has a
`trace_near_credit_outbox` row and vice versa, so one batch can never pay out
both as NEAR receipts and as NEAR AI credit (Q3).

### 3.3 Conversion

- Config `TRACE_COMMONS_NEAR_AI_CREDIT_RATE` = `"<numerator>/<denominator>"`,
  integer nano-USD per microcredit, as a rational. No default: unset refuses
  with `missing_control:near_ai_credit_rate`. No floats anywhere.
- `nano_usd = floor((micros * num + carry) / den)`, remainder carried per
  `(tenant, rail)` in `trace_settlement_rail_carry`, so rounding never loses
  or invents value across batches. Computed in u128 with checked arithmetic;
  overflow refuses.
- The rate's text and a SHA-256 of it are recorded on every outbox row
  (`rate_ref_hash`), so a later rate change never reprices a queued row.
- A per-delivery ceiling `TRACE_COMMONS_NEAR_AI_CREDIT_MAX_DELIVERY_NANO_USD`
  (required) bounds the blast radius of a bad rate; a row above it is frozen
  `near_ai_delivery_above_ceiling`, not split.

### 3.4 Recipient mapping

- New table `trace_near_ai_credit_recipients` (tenant-scoped, RLS forced):
  `tenant_id`, `org_id UUID`, `org_ref_hash` (HMAC-free SHA-256 over a
  domain-separated string, for audit only), `role_at_bind`, `bound_at`,
  `revoked_at`. One active recipient per tenant.
- Binding is contributor-initiated and needs **no admin key**: the daemon
  sends its NEAR AI access token once; the server calls `/users/me` with it,
  deserialises a **separate** struct that reads `organizations[{id, role,
  is_active}]` (the existing `IntrospectedUser` is not widened — its docs
  forbid it), and accepts the chosen `org_id` only if it is listed, active,
  and the role is `owner` or `admin`. An empty list is refused
  (`near_ai_org_membership_unverified`), never treated as "none".
- The org id is the one external identifier we must send; it never appears in
  logs, audit rows, or contributor-visible status (2026-09-04 memo `:158-159`).
  Audit uses `org_ref_hash`.
- Without an active recipient a tenant's batches are not enqueued for this
  rail (they stay eligible for the legacy path or wait).

### 3.5 Exactly-once delivery without CAS

For each outbox row, under a **per-organization** session advisory lock
(`pg_try_advisory_lock` over a hash of the org id; multiple tenants may bind
one org), with marker `changeReason = "trace-commons:settle:v1:<delivery_hash>"`
where `delivery_hash = sha256(rail_id || instrument_id || tenant-scoped
outbox id)` (no tenant id inside the marker, only a hash):

1. **Scan.** Read the full `limits/history` (paginate to `total`; 404 =
   empty). If any row's `changeReason` equals our marker, the write already
   landed (e.g. a timeout after send): verify and record, never PATCH again.
2. **Precondition.** Let `active` be the open row of the configured credit
   type. Require `active` is absent, or `changedBy == "trace-commons-settlement"`
   and its `changeReason` starts with our prefix **and** its id equals the
   row we last recorded for this org. Anything else freezes the row
   `near_ai_grant_bucket_foreign_write` and writes nothing. (A pre-existing
   NEAR AI grant therefore blocks the first delivery until an operator adopts
   it; Q2.)
3. **Bound.** `new = active.amount + delta`, checked `<= 1e18` and the
   ceiling.
4. **PATCH** `{type, source: "trace-commons", spendLimit: {amount: new,
   currency: "USD"}, changedBy: "trace-commons-settlement", changeReason:
   marker}`.
5. **Read-back.** Re-read history. Require exactly one row with our marker,
   `spendLimit.amount == new`, and that the row it superseded is `active`
   (same id, same amount). Else freeze `near_ai_grant_interleaved_write`.
6. **Record** `prior_amount`, `written_amount`, delta, and a hash of the
   history row id (`external_receipt_hash`) — the row id itself is not
   stored.

Failure classes:

| Outcome | Row state | Next action |
|---|---|---|
| transport error before send, 5xx, 429 | `retry` with backoff | redo from step 1 |
| timeout / connection reset after send | `retry` | step 1 finds or does not find the marker |
| 401 (missing, expired, UA-mismatched key) | `retry`, label `near_ai_admin_unauthorized` | operator; all rows wait |
| 403 `insufficient_permissions` | `frozen` `near_ai_admin_read_only_key` | operator |
| 404 on PATCH (org inactive/deleted) | `frozen` `near_ai_org_not_found` | operator / contributor rebinds |
| 400 `invalid_limits` | `frozen` `near_ai_limit_rejected` | operator |
| foreign or interleaved write | `frozen` | operator, section 3.7 |

### 3.6 Reconciler

A separate loop needs only a **read-only** key. For each org with delivered
rows it reads `limits/history` and checks: every delivered marker is present
with its recorded amount; the active row is our latest write. A foreign row
after ours sets the recipient `frozen` (`near_ai_grant_bucket_foreign_write`)
so no further delivery can build on an overwritten base, and marks the
affected rows `reconcile_mismatch`. Delivered and verified rows move to
`reconciled`.

### 3.7 States

`trace_settlement_rail_outbox.state`:
`pending -> leased -> delivered -> reconciled`, with `retry` (backoff via
`next_attempt_at`, `attempt_count`), `frozen` (label, operator `unfreeze`
returns to `retry` — a frozen row is never deleted), and `cancelled` (operator,
only from `pending`/`retry`/`frozen`; the batch becomes eligible for another
path). Identity columns (tenant, batch, rail, instrument, amount, marker) are
immutable by trigger, as V94 does for legs.

### 3.8 Audit

Every transition writes the row's own `last_error_label` (`^[a-z0-9_]{1,64}$`)
and hashes only: `delivery_hash`, `org_ref_hash`, `external_receipt_hash`,
`rate_ref_hash`. No org id, tenant id, admin email, or NEAR AI row id in any
log line. The HTTP client's errors carry a status class and a label, never a
body (NEAR AI error bodies echo the org id: `admin/mod.rs:590-594`).

### 3.9 Kill switch and config

| Variable | Required when enabled | Meaning |
|---|---|---|
| `TRACE_COMMONS_NEAR_AI_CREDIT_RAIL` | — | `disabled` (default, also any unknown value), `dry_run` (everything except the PATCH), `live` |
| `TRACE_COMMONS_NEAR_AI_ADMIN_BASE_URL` | yes | API root including `/v1` |
| `TRACE_COMMONS_NEAR_AI_ADMIN_KEY` | `live` | `read_write` admin token, `adm_` prefix |
| `TRACE_COMMONS_NEAR_AI_ADMIN_READ_KEY` | reconciler | `read_only` token; falls back to nothing |
| `TRACE_COMMONS_NEAR_AI_ADMIN_USER_AGENT` | yes | exact UA the key was minted with |
| `TRACE_COMMONS_NEAR_AI_CREDIT_RATE` | yes | `num/den` nano-USD per microcredit |
| `TRACE_COMMONS_NEAR_AI_CREDIT_MAX_DELIVERY_NANO_USD` | yes | per-row ceiling |
| `TRACE_COMMONS_NEAR_AI_CREDIT_TYPE` | no, default `grant` | NEAR AI `type` to write |

`live` without the key refuses at boot with `missing_control:near_ai_admin_key`;
there is no fallback to `dry_run`. Setting the mode to `disabled` stops new
leases immediately; in-flight rows finish their current step and stop.

### 3.10 What needs the key

| Needs nothing | Read-only key | Read-write key |
|---|---|---|
| enqueue, conversion, carry, recipient binding via the contributor's own token, dry-run, fake-server tests | reconciler, balance display, pre-flight history scan | the PATCH |

## 4. Build plan in this slice

1. `settlement_rail.rs`: trait, amounts, rate, labels.
2. `near_ai_admin.rs`: `NearAiAdminApi` trait and reqwest client.
3. `near_ai_inference_credit.rs`: the rail (section 3.5), reconciler check,
   config with fail-closed refusal.
4. `V127__settlement_rail_outbox.sql`: outbox, carry, recipients, triggers,
   RLS, grants.
5. A per-tenant dispatch tick over the store.
6. Tests: a fake NEAR AI admin server modelling 1.1-1.6 (per-type
   replacement, no CAS, 404 history, UA binding, read-only 403).
7. Runbook stub `docs/operator/near-ai-inference-credit.md`.

Also built: `near_ai_recipient.rs`, the `/users/me` organization check that
gates an election (owner or admin of an active organization; an empty list
is unverified), and a startup refusal in `trace-commons-ingest` for an
enabled rail with a missing control.

Deferred: the contributor binding route and client UI; a worker route and
scheduler that call `enqueue_batch`, `dispatch_tick` and `reconcile_tick`;
admin routes for unfreeze, cancel and adopting a foreign baseline (Q2);
migrating the legacy NEAR outbox.

## 5. Open questions for the owner

1. **Ask NEAR AI for a dedicated credit type** (e.g. `trace_commons`), or a
   `(type, source)`-scoped replace. Recommendation: yes, before going live.
   With it, we write the absolute lifetime total per org — idempotent by
   construction, no precondition freezes, no clobbering of their grants.
2. **Pre-existing NEAR AI grants.** Recommendation: freeze, and give the
   operator an explicit "adopt baseline" action that records the foreign
   row's hash and amount; do not auto-adopt.
3. **Conversion or addition?** Recommendation: conversion — a batch pays on
   exactly one path (enforced), chosen per tenant.
4. **Rate.** Recommendation: start at an owner-set rate, record its hash on
   every row, never reprice queued rows.
5. **Which roles may receive.** Recommendation: `owner` or `admin` of the
   org only.
6. **Who mints the key.** Recommendation: two keys (read-write for dispatch,
   read-only for the reconciler), minted with our exact User-Agent, 90-day
   expiry, on a rotation runbook. Every grant is attributed to the minting
   admin's email on NEAR AI's side.
7. **Unfunded debt.** New capacity pays the org's prior overage first.
   Recommendation: say so in the contributor copy rather than work around it.
8. **Recipient id at rest.** Recommendation: plaintext UUID in an RLS-forced
   table now; envelope-encrypt with the artifact KEK before production.
