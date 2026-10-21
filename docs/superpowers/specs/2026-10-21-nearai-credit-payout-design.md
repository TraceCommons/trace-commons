# Paying contributors in NEAR AI credits -- design

Date: 2026-10-21. Owner: Zaki. Status: draft, nothing built. Three owner
decisions are open (section 11).

This spec covers paying Trace Commons contributors in NEAR AI inference
credit through the org-to-org transfer API that nearai/cloud-api PR #1271
added. It covers the commons side (a transfer client, where it sits in the
settlement code, crash recovery and reconciliation), the contributor side
(linking a NEAR AI organization), eligibility, schedule, a staging test plan
and the threat model.

It builds on, and does not repeat:

- [`2026-09-09-versioned-pipeline-design.md`](2026-09-09-versioned-pipeline-design.md),
  the authority for instruments, settlement legs and payout rails.
- [`2026-09-04-onboarding-funding-contract-design.md`](2026-09-04-onboarding-funding-contract-design.md),
  which set the rules for an account -> provider identity -> org mapping
  (l.154-159) and a redemption state machine (l.161-196). Its premise that no
  provider operation offered idempotency or an operation lookup is no longer
  true after #1271. Section 4 and section 5 below replace that premise; the
  mapping rules still hold.
- [`2026-10-15-settlement-estimate-design.md`](2026-10-15-settlement-estimate-design.md),
  which put the credit-to-dollar rate in operator hands
  (`usd_micros_per_credit`, decision S2).

## What already exists, verified on 2026-10-21

Our tree is `origin/main` at f164d68f9. cloud-api is the #1271 merge commit
71bbb0be. Paths without a repository name are ours.

### Our settlement code

- **The seam.** `crates/trace-commons-gate-api/src/settlement.rs` (AGPL)
  defines `SettlementAdapter` with `instrument_id`, `adapter_identity`,
  `production_qualified` (default `false`), `payout_rail` and async `settle`.
  `SettlementRequest` carries `tenant_storage_ref`, `run_id`,
  `instrument_id`, `atomic_units`, `operation_ref_hash` and
  `expected_result_ref_hash`. It carries **no recipient**.
  `SettlementReceipt` is `internal(..)` or `external(..)`, the latter with an
  `external_receipt_hash`.
- **The idempotency contract** (`settlement.rs`, doc on `settle`): a
  repeated `operation_ref_hash` must return the same receipt, including the
  same external hash, and must not repeat the effect. The pipeline calls
  `settle` before it records the leg.
- **One operation per run.** `operation_ref_hash` is
  `sha256("pipeline-operation-v1:{run_id}:{instrument}:{atomic_units}")`
  (`crates/trace-commons-server/src/versioned_pipeline_bundle.rs:288`). A leg
  is per trace, not per recipient.
- **The tenant ref is one-way.** `TenantStorageRef` is `tenant_sha256:` plus
  the first 32 hex digits of a SHA-256 of the tenant id
  (`crates/trace-commons-gate-api/src/pipeline.rs:153-176`,
  `crates/trace-commons-server/src/versioned_pipeline.rs:171`). An adapter
  cannot set `trace_current_tenant_id()` from it, so it cannot read a
  tenant-scoped table under RLS without help.
- **One production adapter.** `InternalTraceCreditSettlementAdapter`
  (`crates/trace-commons-server/src/versioned_pipeline_production.rs:378-424`)
  has no external effect, rail `none`, and is production-qualified.
  `production_pipeline_builder` (l.735-778) is the only place a registry is
  built. Only `trace_credit` has a cap (`production_pipeline_caps`, l.687).
- **Per-leg outcomes** (`versioned_pipeline.rs`, Settle loop around
  l.15175): `Unavailable` retries uncharged; `Conflict` and `Rejected` fail
  the leg for good; a receipt that does not answer the request fails it as
  `settlement_result_mismatch`. `external_receipt_hash` is unique per tenant
  (V94), so one receipt can answer only one leg.
- **The withdrawal guard is not re-checked for external legs.** The comment
  at `versioned_pipeline.rs:15139-15147` says that before a second
  non-Trace-Credit adapter is registered, its call must re-check the guard
  under the submission row lock.
- **A second, separate payout seam exists for NEAR.** `NearPayoutAdapter`
  (`crates/trace-commons-server/src/versioned_pipeline_credit.rs:262-276`)
  pays out after Trace Credit has settled internally, from a per-batch
  outbox (`trace_near_credit_outbox`, V2). It runs in `process_payouts` and
  `dispatch_near_settlements`. `payout_eligible` requires rail `near`.
  Production never enables it (`PipelineServiceBuilder::with_payout` is not
  called).
- **No balance, no debit.** Credit is a sum of `trace_credit_ledger.points_delta`
  rows. There is no redemption or cash-out code.
  `crates/trace-commons-server/src/inference_funding.rs` only decodes a NEAR
  AI org balance (int64 nano-USD, scale 9).
- **Units.** Trace Credit has 6 decimals: 1 atomic unit = 1 microcredit,
  capped at `i64::MAX` (versioned-pipeline design l.131-146). `AtomicUnits`
  is a `u128`.

### Our identity code

- **The commons does not know any contributor's NEAR AI org.**
  `IntrospectedUser` in `crates/trace-commons-server/src/near_ai_login.rs`
  keeps only `id` and `auth_provider`. Its doc comment says the response's
  `organizations` and `workspaces` are dropped on purpose, and forbids adding
  a field "without deciding it is anchorable". No table holds an org id.
- **The daemon does know one org.** Sign-in mints an inference key in the
  first active org from `GET /v1/organizations`
  (`crates/trace-commons-contributor/src/daemon/nearai_credential/api.rs:427`)
  and stores `organization_id` as non-secret metadata
  (`stored_cloud_credentials.rs:40-46`). "First active" is not "personal".
  The client deserializes no org type or role.
- **The daemon holds a wide session.** `near_ai_session` holds a refresh
  token that can read org state and mint keys. `near_ai_funding`
  (`daemon/nearai_credential/funding.rs:141`) already re-reads the key's org
  with a fresh session exchange and binds a click to what was shown with a
  `connection_revision`.
- **Sealing pattern.** `trace_near_account_anchors` stores a keyed blind
  index plus `sealed_account_name JSONB` and `index_pepper_ref` /
  `account_name_key_ref` (V61). The seal lives in
  `crates/trace-commons-server/src/near_account_identity.rs` (`seal`,
  `seal_login_subject`).
- **Payout redirects need a strong authenticator.**
  `require_authenticator_change_allowed`
  (`crates/trace-commons-ingest/src/bin/trace-commons-ingest.rs:22776`)
  gates `PATCH /v1/account/near-identities/{public_key}/payout`. A native
  token (`tcn1_`) "can read and withdraw, never change authenticators or
  redirect payouts". Accounts with a passkey use the step-up page.
- **Pooled tenants are an operator list.** `trace_legacy_invite_pooled_tenants`
  (V81); nothing is inferred. `is_anchored_tenant`
  (`crates/trace-commons-protocol/src/admission.rs:218`) requires a `near-`
  or `nearai-` prefix and a 64-hex suffix.
- **Pinned hosts.** The daemon's `CLOUD_API_ORIGIN` is a compiled-in
  `https://cloud-api.near.ai` (`daemon/nearai_credential/api.rs:31`). The
  server's login introspection base is compiled in too, with no override in
  non-test builds (`crates/trace-commons-ingest/src/bin/trace_commons_ingest_internal/near_provisioning.rs:989-1040`).
- **Outbound secret patterns.** The NEAR AI scorer reads its key from env
  only, never argv (`versioned_pipeline_production/gate_env.rs:30-38`), and
  its client sets `redirect(Policy::none())`, requires TLS or loopback and
  refuses an empty key. The legacy NEAR submitter holds its bearer as a
  `SecretString` and validates its URL (no credentials, query or fragment;
  http only on loopback). `reqwest` 0.12 and `secrecy` 0.10 are already
  server dependencies. **No new dependency is needed.**

### cloud-api #1271 (merge commit 71bbb0be)

- **Send.** `POST /v1/organizations/{org_id}/credit/transfer`
  (`crates/api/src/routes/credit_transfers.rs:43`). Body
  (`crates/services/src/organization_credits/models.rs`, `TransferRequest`,
  `deny_unknown_fields`): `recipient_organization_id` (UUID),
  `source_credit_type` (`payment`, `staking_farm` or `grant`), exactly one
  of `amount` (i64 nano-USD, >= 1) or `recipient_cumulative` (i64, >= 0),
  `payout_program` (1-100 bytes, no surrounding whitespace), `memo`
  (<= 2000 bytes), `metadata` (object, <= 16 KiB, depth <= 16). Memo and
  metadata are visible to both sides.
- **Amount mode** needs an `Idempotency-Key` header (<= 100 chars). A replay
  with identical terms returns 200 and the original result, including the
  transfer `id`. Different terms return 409 `idempotency_conflict`.
- **Cumulative mode** needs `payout_program` and forbids the key. Under the
  org locks the server sends `max(target - SUM(program), 0)`, where the sum
  covers sender + recipient + program across credentials and sources
  (`crates/database/src/repositories/credit_transfers.rs:121-163`). A zero
  delta returns 200 `{"amount":0,"no_op":true,"recipient_cumulative":<total>}`,
  writes no row, uses no quota and **returns no transfer id**. A positive
  delta returns 201 with `id` and the new program total.
- **Order of checks** (`credit_transfers.rs:82-216`): send policy, then
  recipient active (404 `organization_unavailable`), then the no-op return,
  then org and credential budgets, then `sender_not_reconciled`, then the
  per-minute quota (429 `transfer_rate_limited`), then source capacity.
- **Quota.** `credits_transfer.max_sends_per_minute`, default 60, range
  1-100,000, a durable count of the sender's rows in the last minute.
- **Credentials.** `oc_` bearer tokens bound to one org, stored as SHA-256
  only. Permissions `credits:read` and `credits:transfer`. Constraints under
  `"credits:transfer"`: `max_amount_per_transfer`, `max_total_amount`
  (lifetime, from the ledger, not reset by rotate or update),
  `allowed_source_types`, `allowed_recipient_ids` (<= 1000; an empty list
  permits none, null permits any). `expires_at` must be in the future and at
  most 366 days out. Create, update and rotate need an owner session and
  `credentials.enabled`. Rotation invalidates the old token at once.
- **Admin policy.** `PUT /v1/admin/organizations/{org_id}/policy`, system
  admin only, replaces the whole policy: `policy.credentials.enabled`,
  `policy.credits_transfer.enabled`, `policy.credits_transfer.max_sends_per_minute`,
  `policy.credits_transfer.constraints.{max_amount_per_transfer, max_total_amount, allowed_source_types, allowed_recipient_ids}`,
  and a top-level `reason`. With `allowed_source_types` omitted, only
  `payment` and `staking_farm` are allowed. **A grant-funded sender must list
  `grant`.**
- **Read.** `GET .../credit/transfer/capacity`, `GET .../credit/transfers`
  (`direction`, `idempotency_key`, `limit` 1-100, cursor
  `before_time`+`before_id`; newest first; **no `payout_program` filter**)
  and `GET .../credit/transfers/{id}`. All need `credits:read`.
- **Recipient.** Any existing, active org. No opt-in. Credit arrives as
  `transfer` credit, cannot be re-sent, and pays any recipient overage first.
- **Finding a user's org.** `GET /v1/users/me` with the user's own session
  returns `organizations[]` with `id`, `role` and `is_active`, sorted by join
  time (`crates/api/src/routes/users.rs:172-215`). **There is no "personal"
  flag** and no server-to-server lookup by user. NEAR-wallet users get a
  personal org at sign-up named `<account>-org-<4 hex>`; other providers get
  `<email-local>-org-<4 chars>`. "First active org where the user is owner"
  is a heuristic.

## 1. Goal and non-goals

**Goal.** A contributor who has linked a NEAR AI organization receives NEAR
AI inference credit for the Trace Credit they have earned. The commons sends
it from its own reseller org with a scoped credential. Every send is
idempotent, budget-capped and reconcilable against #1271's transfer history.

**Funding model** (owner, 2026-10-19). The commons runs one reseller org.
NEAR AI funds it first with an admin grant. Later the commons buys payment
credits. USD only. The reseller sends; contributors receive `transfer`
credits in their personal orgs.

**Non-goals.**

- Cash, tokens or any rail other than NEAR AI credit. The NEP-141 Trace
  Credit path and `NearPayoutAdapter` are untouched.
- Clawback. #1271 has no reversal, and revocation does not claw back
  settled credit today (`docs/trace-commons-roadmap.md:31-38`).
- Attributing what a recipient spends to the commons. #1271 defers it.
- Re-sending or pooling contributor credit. Transfer credit cannot be
  re-sent, by design.
- Paying pooled tenants (section 6).

## 2. Staging setup (operator steps)

The owner does these once on staging, then again for production. Org ids and
tokens never go in the repository, in a PR, or in a log.

1. **Create the reseller org** in the staging UI. The owner reports the UI is
   `cloud-stg.near.ai`. cloud-api's own scripts default the API base to
   `https://cloud-stg-api.near.ai` (`test_signature.sh:9` and four scripts).
   **Confirm the API host before configuring it.**
2. **Ask NEAR AI to set the admin policy** (system admin only):

   ```json
   {"policy": {
      "credentials": {"enabled": true},
      "credits_transfer": {
        "enabled": true,
        "max_sends_per_minute": 60,
        "constraints": {
          "max_amount_per_transfer": 5000000000,
          "max_total_amount": 100000000000,
          "allowed_source_types": ["grant", "payment"]
        }}},
    "reason": "Trace Commons contributor payouts, staging"}
   ```

   The amounts are placeholders ($5 per send, $100 lifetime). `grant` must be
   listed or a grant-funded send fails 409 `organization_transfer_limit_exceeded`.
   List `payment` now so buying credits later needs no policy change. Leave
   `allowed_recipient_ids` out at org level for open enrollment.
3. **Grant.** NEAR AI posts the grant to the reseller org. The sender must
   have an `organization_balance` row with zero legacy and unfunded amounts,
   or every send fails 409 `sender_not_reconciled`.
4. **Deployment check.** #1271's rollout note (`docs/credit-allocation.md:408-433`)
   needs `CREDIT_USAGE_ORDER=grant,staking_farm,transfer,payment,postpay` and
   `CREDIT_ALLOCATION_POLICY_VERSION=v2` on the deployment. Whether staging
   has them is not visible from the repo. Ask NEAR AI.
5. **Mint the payout credential** with an owner session:
   `POST /v1/organizations/{org_id}/credentials` with
   `permissions: ["credits:read", "credits:transfer"]`, `expires_at` at most
   90 days out on staging (366 is the ceiling), and
   `constraints["credits:transfer"]` with `max_amount_per_transfer`,
   `max_total_amount` (below the org's), `allowed_source_types: ["grant"]`
   and, on staging, `allowed_recipient_ids` set to the test recipient orgs.
   Both permissions are needed: `credits:transfer` alone cannot read
   capacity or history, so it cannot reconcile.
6. **Hold the secret.** The token is shown once. Put it straight into the
   host's secret store. Ingest reads it from env only, as the scorer key is:
   - `TRACE_COMMONS_NEAR_AI_PAYOUT_BASE_URL` (TLS, no trailing slash, no
     credentials, query or fragment; http only on loopback);
   - `TRACE_COMMONS_NEAR_AI_PAYOUT_CREDENTIAL` (held as `SecretString`,
     refused if empty or not `oc_` plus 64 hex);
   - `TRACE_COMMONS_NEAR_AI_PAYOUT_SENDER_ORG` (the reseller org UUID);
   - `TRACE_COMMONS_NEAR_AI_PAYOUT_SOURCE` (`grant` or `payment`).

   These names are proposals; none exists yet. Follow
   `docs/operator/commons-credential-storage.md` and
   `docs/operator/key-rotation.md`. The token is never passed on a command
   line, written to a file in the repo, or logged. Logs carry only
   `sha256:` of the token prefix (`token_prefix` from the create response)
   so an operator can tell which credential was live.
7. **Payout program.** Set nothing in cloud-api; programs are just names. Our
   side derives them (section 4).

## 3. Placement: which seam, which mode

This is the central design choice. The brief asked for an adapter behind
`SettlementAdapter` that sends in `recipient_cumulative` mode. The two do not
compose, and the spec should say why rather than hide it.

**Why cumulative does not fit a per-leg adapter.**

- A leg is one trace. `settle` must return the same `external_receipt_hash`
  on retry.
- A cumulative send returns a transfer `id` only when its delta is positive.
  A retry after a lost response returns `no_op` with no id. The adapter would
  have to persist the first outcome itself, before the pipeline records the
  leg, and find the id by paging history.
- Two legs of one tenant that settle close together compute two targets. The
  lower target gets `no_op` on its first call: its money went out inside the
  other leg's delta. It has no receipt of its own, and the per-tenant unique
  index on `external_receipt_hash` forbids sharing one. Serializing legs per
  tenant fixes this only by making every leg wait on the one before it.

**Option A: per-leg adapter, amount mode.** A new instrument (for example
`nearai_usd`, 9 decimals) with `NearAiTransferSettlementAdapter`. Each leg
sends `amount` with `Idempotency-Key: tc1-{operation_ref_hash hex}` (68
chars) and `payout_program` set, so the program total still counts it. A
replay returns the original `id`. This matches the `settle` contract exactly.
Costs: one transfer per trace, the 60/min quota, per-leg rounding dust lost
for good, a new bundle award and cap, and the l.15139 guard fix first.

**Option B: payout pass, cumulative mode.** `trace_credit` keeps settling
internally, unchanged. A new payout pass, beside `process_payouts`, computes
each linked tenant's cumulative owed amount from the ledger and sends one
cumulative target per tenant per run. It has its own outbox table and its
own trait, `NearAiCreditPayout`, not `NearPayoutAdapter` (that one is
on-chain and its `payout_eligible` is wired to rail `near`). Retries and
re-runs converge by construction. One transfer per tenant per batch. Dust
carries forward.

**Recommendation: Option B,** with Option A kept as the per-settled-credit
variant of open decision 11c. The two share the HTTP client, the link table,
the rate table and the error mapping. Only the caller differs. If the owner
picks per-settled-credit, build Option A instead; do not build both.

The rest of this spec describes Option B and notes where A differs.

## 4. The transfer client and the payout pass

### Crate and licence

All of it lives in `crates/trace-commons-server` (AGPL): a client module
`near_ai_transfer.rs`, the pass in the versioned pipeline, and storage in
`db/`. The link request and response types go in `crates/trace-commons-protocol`
(permissive), beside `inference_connection.rs`, because the daemon sends
them. No permissive crate gains a server or gate dependency.
`tests/license_boundary.rs` must stay unchanged.

### Client

`NearAiTransferClient` holds the base URL, sender org, source type and the
credential as `SecretString`. It copies the scorer client's shape: TLS or
loopback only, `redirect(Policy::none())`, a fixed user agent, a 15 s
timeout (cloud-api's own whole-operation timeout is 10 s), `bearer_auth`.
It exposes four calls: `send_cumulative`, `send_amount` (Option A only),
`capacity`, and `history_page`. It parses the error body's `type` field into
a closed enum and keeps nothing else from an error.

### Payout program and link generation

The cumulative total is scoped to sender + recipient + program. If one
program name served every link, a contributor who moved from org X to org Y
would get a fresh total at Y, and a cumulative target would pay all their
history again. So:

- Each link has a **generation id** (UUID) and its own program:
  `tc1-` plus the first 32 hex of `sha256("nearai-payout-program-v1:" || link_id)`.
  It is opaque to the recipient and fits in 100 bytes.
- When a link closes, the pass first settles it (see change/unlink in
  section 5) and records `final_paid_nano` from #1271.
- The target for the live link is
  `owed_nano(tenant) - sum(final_paid_nano of the tenant's closed links)`.
  It is never below zero.
- Relinking to an org used before still gets a new generation, so a new
  program. Nothing collides.

### Owed amount and the rate

- **Owed credit** is the tenant's settled `trace_credit` ledger sum: events
  in a settled state only. Pending-review and held credit are excluded. The
  exact predicate reuses `account_credit_totals`
  (`crates/trace-commons-ingest/src/bin/trace-commons-ingest.rs:19678-19738`)
  minus its pending bucket; the implementation plan pins it.
- **Rate.** A new append-only table `trace_nearai_payout_rates`
  (`rate_version`, `nano_usd_per_credit`, `effective_from`, `set_by_hash`).
  The repository carries no production value, as with S2.
- **Conversion per rate epoch.** Each ledger event belongs to the epoch in
  force at its settlement time. `owed_nano = sum over epochs of
  floor(atomic_units_in_epoch * nano_usd_per_credit / 1_000_000)`. A rate
  change re-prices only new credit. Dust is lost at most once per epoch, not
  once per trace.
- **Bounds.** All arithmetic is in `u128`, checked. A result above
  `i64::MAX` (about 9.2 billion USD) refuses the line as
  `payout_amount_out_of_range` and holds it.
- **Owed going down.** If revocation lowers the sum, the target drops below
  what was paid and the send is a `no_op`. Nothing is clawed back. New
  credit fills the gap before anything more is sent.

### State and storage

Two new tables, RLS forced, tenant predicates through
`trace_current_tenant_id()`. Migration number: the next free one per the
repo's numbering convention (V130 is the highest; check the convention
before picking).

- `trace_nearai_payout_links`: `tenant_id`, `link_id`, `state`
  (`active`, `closing`, `closed`), `sealed_org_id JSONB`, `org_index`
  (keyed blind index), `org_key_ref`, `index_pepper_ref`, `payout_program`,
  `final_paid_nano NUMERIC(20,0)` (set on close), `version`, timestamps,
  and a `tenant_storage_ref` column for Option A. A partial unique index
  allows one `active` or `closing` row per tenant.
- `trace_nearai_payout_lines`: `tenant_id`, `link_id`, `batch_id`,
  `target_nano`, `rate_version`, `state` (`pending`, `sent`, `confirmed`,
  `no_op`, `held`, `failed`), `transfer_id_hash`, `delta_nano`,
  `attempts`, `last_error_label`. Unique on `(link_id, batch_id)`.

### One pass

1. Pick tenants that are eligible (section 6), have an `active` link, and
   whose target exceeds the last confirmed program total. Withdrawal
   already forfeits legs before they reach the ledger, so the ledger sum is
   the guard; the pass adds none.
2. Write the line `pending` with its `target_nano`, committed, before any
   network call.
3. Probe `capacity` once per pass. If `sending_enabled` is false,
   `sender_reconciled` is false, or `credential_can_transfer` is false, stop
   the pass with that label. Pace sends to `sends_remaining_in_window` and to
   a local limit below `max_sends_per_minute`.
4. Send `recipient_cumulative = target_nano` with the link's program. Memo
   empty. Metadata `{"tc": "<sha256 of 'nearai-payout-line-v1:' || line id>"}`,
   an opaque handle for support with no contributor identity.
5. Record the outcome:
   - 201: `confirmed`, `transfer_id_hash = sha256("nearai-transfer-v1:" || id)`,
     `delta_nano = amount`. If the returned program total is not the
     target, hold as `payout_program_total_mismatch`.
   - 200 `no_op`: if `recipient_cumulative >= target`, run recovery (below);
     otherwise hold as `payout_program_total_mismatch`.

### Crash recovery

A crash between step 2 and step 5 leaves a `pending` line whose send may or
may not have committed. The next pass re-sends the same target:

- If the first send never committed, it is sent now (201).
- If it did, the reply is `no_op`. The pass then pages
  `GET .../credit/transfers?direction=outgoing` and filters client-side for
  this recipient, this program and `recipient_cumulative == target_nano`.
  The match gives the transfer id. History has no program filter, and the
  cursor is still returned on the last page, so the pass stops on a short
  page.
- If no row matches, an earlier line must have reached past this target.
  Mark it `no_op` and let reconciliation explain it.

### Error mapping

| #1271 reply | Line state | Notes |
|---|---|---|
| 429 `transfer_rate_limited`, 503 `transfer_unavailable` / `accounting_busy`, timeout | stays `pending` | retried next pass, same target |
| 409 `insufficient_posted_capacity`, `sender_not_reconciled` | pass stops, `held` | operator: fund or reconcile the sender |
| 409 `credential_transfer_limit_exceeded`, `organization_transfer_limit_exceeded` | `held` | budget or recipient restriction; the code does not say which, so compare with `capacity` |
| 404 `organization_unavailable` | `held`, link marked `recipient_unavailable` | contributor must relink |
| 403 `credential_refused`, `sender_not_enabled` | pass stops, fail closed | credential expired, revoked or wrong |
| 409 `idempotency_conflict` (Option A), 400 any | `failed` | a bug on our side; page the operator |

For Option A, the same table maps to `SettlementError`: retryable rows to
`Unavailable`, held rows to `Unavailable` with a hold label (the leg must
not be charged for a funding gap), and `failed` rows to `Rejected`.

### Fail-closed rules

- Payout configured but any of the four env values missing or malformed:
  ingest refuses to start with `near_ai_payout_config_missing`.
- `production_qualified` is true only when the base URL is the production
  host, the credential is set, a rate is in force, and the startup capacity
  probe answers `sending_enabled: true`. Readiness reports the rail as
  `near_ai` with `qualified` and `payout_enabled`, as the existing JSON does
  (`versioned_pipeline_production.rs:933-975`).
- No rate in force: no line is written. Nothing falls back to a default.
- A link whose sealed org id fails to open: hold `payout_link_unreadable`.

### Hash-only logging

Logs and audit rows carry labels and `sha256:` values only: line id hash,
link id hash, transfer id hash, program hash, amounts in nano-USD. Never the
recipient or sender org UUID, the token, the memo, the metadata, or the
tenant id. This follows `docs/operator/hash-only-logging.md`.

## 5. Linking a contributor to a NEAR AI organization

### What the app knows

The daemon holds the org its inference key was minted in and a refresh
session. It does not know which org is personal. cloud-api's `/v1/users/me`
returns each org's `role` and `is_active`, which the daemon does not read
today.

### Verification on the server, not the client

A client-asserted org id is exactly what `near_ai_login.rs` refuses to
trust. The link call therefore follows the provision and bind shape:

1. The daemon exchanges its refresh token for a fresh access token
   (`session::exchange`).
2. It calls `/v1/users/me` itself, lists the active orgs where the user is
   owner, and proposes the first one (the heuristic above). The app shows its
   name.
3. On confirm, the daemon sends the chosen org id **and the fresh NEAR AI
   access token in the request body** to ingest, as provisioning does
   (`daemon/nearai_onboarding.rs:32-42`).
4. Ingest calls `GET /v1/users/me` once with that token, then drops the
   token. It reads the response into a **new, single-purpose struct**, not
   `IntrospectedUser`, holding `id` and `organizations[]{id, role, is_active}`
   only. It checks that `id` matches the account's anchored subject (through
   the existing blind index), and that the chosen org is in the list,
   active, with `role == "owner"`.
5. Ingest sends a liveness probe: `recipient_cumulative: 0` on a fixed probe
   program, from the reseller credential. Per the check order above, that
   returns 200 `no_op` only for an active recipient, writes no row and uses
   no quota. A 404 refuses the link as `near_ai_org_unavailable`.
6. Ingest stores the link and answers with the org name it verified, so the
   app can show what was saved.

### The registration call

`POST /v1/account/near-ai/payout-org`, copying the inference-connection
stack (`crates/trace-commons-contributor/src/inference_connection.rs:190-230`,
`crates/trace-commons-ingest/src/bin/trace_commons_ingest_internal/inference_connection.rs`):

- account session only; `DeviceBearer` is refused with 403;
- body `deny_unknown_fields`: `organization_id`, `near_ai_access_token`,
  `idempotency_key` (non-nil UUID), `expected_current_version`;
- **behind `require_authenticator_change_allowed`.** A link is a payout
  redirect. An account with a passkey must pass the step-up page
  (`docs/operator/native-step-up-page.md`);
- outcomes: `linked`, `idempotency_conflict` (409), `version_conflict`,
  `subject_mismatch`, `org_not_owned`, `near_ai_org_unavailable`,
  `account_ineligible`;
- audit row `account_payout_org_linked` with the link id hash and version
  only.

`DELETE` on the same path unlinks, under the same gate.
`GET` returns `{linked, org_name_observed_at, state, version}`, never the id.

### Storage

The send needs the raw UUID, so hash-only storage is impossible. The org id
is sealed with the same machinery as `sealed_account_name`: ciphertext in
`sealed_org_id`, a keyed blind index in `org_index`, and key and pepper refs
for rotation. The blind index lets an operator find two tenants pointing at
one org (section 9) without opening either. The key is not in the database.

### Change and unlink

- **Change** closes the old link and opens a new generation in one
  transaction. The old link goes to `closing`.
- **Closing.** The next pass sends nothing new to a closing link. It
  confirms any `pending` line, reads the program total from history, writes
  `final_paid_nano`, and sets `closed`. Until then the new link gets no
  sends, so the two never race.
- **Unlink** does the same with no new link. Credit keeps accruing and is
  sent when the contributor links again.

### Credit earned before linking

Nothing extra. The first target for a new link is everything owed minus what
prior links received, so back-pay is the first send. It may exceed
`max_amount_per_transfer`; the pass then sends a capped target and steps up
over later passes (target = min(owed, sent + cap)).

### Consent copy (draft, for owner review)

New keys in `crates/trace-commons-contributor/src/consent_copy.rs`, one each
for title, body, button and accessibility name, per `docs/ui-copy-guide.md`.
**All text below is draft and needs owner review.**

- Title: "Get paid in NEAR AI credit"
- Body: "The commons will send NEAR AI credit for your traces to {org name}.
  Credit sent there stays there. It can't be moved to another organization.
  You can change or remove this organization in Settings."
- Scope line beside the button: "Sends credit to {org name} only."
- Button: "Send credit to {org name}"
- Secondary: "Not now"
- Unlink button: "Stop sending credit"; body: "Credit you earn is kept and
  sent when you choose an organization again."
- Status: "Credit goes to {org name}" / "No organization chosen" /
  "Organization unavailable. Choose another."

The confirm step names the org shown, and a `connection_revision`-style
check (as `near_ai_funding` does) refuses a click if the org changed since
it was displayed. macOS is the lead shell; the others follow it.

## 6. Eligibility

Proposed default, **not decided for payouts** (open decision 11b):

- **Eligible:** anchored account tenants, `is_anchored_tenant` true
  (`near-` / `nearai-` plus 64 hex), with a `trace_accounts` row that is not
  closed. A passkey-origin account must be `bound` in
  `trace_account_bindings` (V97); an `unbound` one has no NEAR AI subject to
  verify against.
- **Excluded:** every tenant in `trace_legacy_invite_pooled_tenants`. One
  pooled tenant holds many people, so its credit has no single owner.
- **Legacy `tenant-...` tenants:** eligible only once linked to a NEAR
  account through the legacy invite link (V81). This mirrors the 2026-10-21
  decision for the judgement service. It is a proposal here.

The predicate lives in one server function, checked both at link time and
in every pass.

## 7. Schedule and batching

- **Daily batch** (recommended): one pass a day, one send per tenant whose
  target grew. Recipients see one transfer a day at most.
- **Weekly batch:** fewer, larger transfers. Credit waits up to a week.
- **Per settled credit** (Option A): one transfer per trace, minutes after
  settlement. Most sends, most dust, the quota matters.
- **Minimum send.** Skip a tenant whose delta is under a floor (proposed
  $0.10) until it grows. Cumulative mode makes this free: nothing is lost.

At 60 sends a minute a daily pass reaches 86,400 tenants in a day, far above
the pilot. The quota can be raised by policy.

## 8. Staging test plan and reconciliation drill

### Known staging gap

The daemon's cloud-api origin and the server's introspection base are both
compiled in as production. The link flow cannot reach staging as built.
Plan: a build-time feature (`near-ai-staging`) that switches both, refused
in a release build and reported in readiness. Until then, test the payout
pass on staging with links written by an operator tool, and test the link
flow against production with a closed recipient list.

Staging and production are separate deployments. Org ids differ; staging
contributors need staging accounts.

### Steps

1. Section 2 setup with `allowed_recipient_ids` set to two test orgs.
2. Unit: rate epochs and dust; `u128` to `i64` bounds; error mapping; program
   derivation; target after a closed link.
3. Mock #1271 server tests: 201; `no_op` then history recovery; lost
   response; 429 pacing; each 409 hold; 403 stops the pass.
4. Staging, live: two tenants, one pass. Check delta and program totals.
5. Kill ingest after step 2 of a pass and before step 5. Re-run. Expect one
   transfer, not two.
6. Change org for one tenant. Expect the closing link settled, then the new
   program starting at owed minus paid.
7. Exhaust `max_total_amount`. Expect a hold, no partial state.
8. Revoke the credential. Expect the pass to stop with `credential_refused`
   and readiness to drop.
9. Recipient org deactivated. Expect `recipient_unavailable`.

### Reconciliation drill

`POST /v1/admin/nearai-payout-reconcile-drill`, following
`docs/operator/drills.md`: admin bearer, JSON body, `ready` plus
label-only `blocking_gaps`, hash-only evidence, wired into
`rollout-smoke/evidence`. It pages outgoing history and compares, per link:
the sum of our confirmed `delta_nano` against the program total; every
confirmed `transfer_id_hash` against a history row; and no history row in a
`tc1-` program that we do not hold. Gaps: `payout_history_unmatched`,
`payout_program_total_mismatch`, `payout_line_missing_transfer`. It reports
counts and hashes only. cloud-api's admin reconciliation route is system
admin only, so we cannot call it; NEAR AI can.

## 9. Threat model

- **Stolen credential.** Bounded by `max_amount_per_transfer`,
  `max_total_amount` (lifetime, survives rotation), expiry (90 days
  proposed), and source restriction. `allowed_recipient_ids` only helps a
  closed pilot: with it set, every new contributor's org is refused, and the
  list holds at most 1000. For open enrollment the cap and expiry are the
  defence. Response: revoke in the UI, rotate, update the env, restart.
  Rotation cuts the old token at once, so plan a short pause.
- **Stolen database.** Org ids are sealed; the key is not in the database.
  The credential is not in the database.
- **Double pay.** Cumulative targets converge, so retries and re-runs cannot
  overpay one link. Per-link programs stop a relink from paying history
  twice. The closing state stops two links racing. In Option A the
  idempotency key does the same job.
- **Wrong recipient.** The org must be in the contributor's own
  `/users/me`, active and owned, checked by the server with the
  contributor's own token. A link change needs a strong authenticator. A
  wrong send cannot be reversed; transfer credit cannot be re-sent.
- **Shared org.** "Owner" is not "personal". Two of our tenants owning one
  org is visible through the blind index; flag it for review, do not block.
- **Sender out of funds.** 409 `insufficient_posted_capacity` holds the
  pass. Lines stay owed. Readiness reports it. No partial transfer is sent.
- **Rounding.** Floor per rate epoch; at most one nano-USD lost per tenant
  per epoch in Option B, per trace in Option A.
- **Metadata leak.** Memo empty, metadata an opaque hash. Both sides can read
  them, and we put nothing about the contributor there.
- **Over-reach by a weak session.** A `tcn1_` token cannot link or unlink on
  an account with a passkey.

## 10. Delivery plan

Small stacked PRs. Each one passes the repo checks on its own.

1. **Protocol types** for link, unlink and status (permissive crate).
2. **Migration** for links, lines and rates, with RLS tests.
3. **Transfer client** with the mock #1271 server and the error table.
4. **Link route** in ingest, server-side `/users/me` check, liveness probe,
   step-up gate, audit.
5. **Daemon and macOS** link flow and copy keys (draft copy behind a
   setting until the owner signs it off).
6. **Payout pass** (Option B), behind config, with crash points.
7. **Reconciliation drill** and readiness wiring.
8. **Staging feature** for the pinned hosts, then the staging run.
9. **Operator runbook** under `docs/operator/`, listed in its README.

If the owner picks per settled credit (Option A), step 6 becomes: first the
`versioned_pipeline.rs:15139` guard re-check under the row lock, then the
`nearai_usd` instrument, cap and bundle award, then
`NearAiTransferSettlementAdapter`.

## 11. Open decisions (owner)

### (a) Rate from Trace Credit to USD

- Options: a fixed rate set now; a rate tied to `usd_micros_per_credit` in
  the settlement table so the estimate and the payout agree; a rate per
  period set from the funding budget.
- **Recommendation:** one rate, the same value as the published
  `usd_micros_per_credit`, stored in `trace_nearai_payout_rates` with epochs.
  The estimate then shows what is paid.

### (b) Who is eligible

- Options: anchored accounts only; plus linked legacy tenants; plus pooled
  tenants by some split.
- **Recommendation:** section 6. Anchored accounts and linked legacy
  tenants. Pooled tenants excluded.

### (c) When we pay

- Options: daily batch; weekly batch; per settled credit.
- This choice also picks the seam: per settled credit means Option A
  (per-leg adapter, amount mode); a batch means Option B (payout pass,
  cumulative mode).
- **Recommendation:** daily batch, Option B.

### Other open questions (not owner decisions yet)

1. **Personal org.** #1271 has no personal flag and no server lookup by user.
   Ask NEAR AI for one. Until then, the user picks from owned, active orgs.
2. **Staging API host:** `cloud-stg-api.near.ai` or `cloud-stg.near.ai`.
3. **Staging deployment flags** (`CREDIT_ALLOCATION_POLICY_VERSION=v2`,
   `CREDIT_USAGE_ORDER`): unknown.
4. **History filter.** No `payout_program` filter; recovery and the drill
   page all outgoing history. Fine at pilot scale; ask NEAR AI for a filter
   before it is not.
5. **Restriction vs budget.** One error code covers both. We infer from
   `capacity`.
6. **Credit predicate.** Which ledger states count as owed. The plan pins it.
7. **Minimum send floor** and **credential expiry** values.
8. **Spending attribution.** Deferred in #1271, so we cannot show a
   contributor how much of their NEAR AI spend came from the commons.
