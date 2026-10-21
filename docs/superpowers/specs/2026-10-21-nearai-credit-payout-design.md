# Paying contributors in NEAR AI credits -- design

Date: 2026-10-21. Owner: Zaki. Status: draft, nothing built. The three
owner decisions this spec was written around are open (section 11, a-c).
Review raised three more questions only the owner can settle (section 11,
d-f); they are open too.

## Owner decisions

- **2026-10-21: the contributor chooses the payout org in the app.** The app
  lists the NEAR AI organizations the contributor's sign-in can see and the
  contributor picks one. There is no automatic "personal org" guess, and no
  NEAR AI change is needed to find the org. The server still verifies the
  choice (section 5); a pick is never trusted on the client's word.

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
  (`crates/trace-commons-contributor/src/daemon/stored_cloud_credentials.rs:40-46`). "First active" is not "personal".
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
  gates `PATCH /v1/account/near-identities/{public_key}/payout`. It passes a
  strong session (`AccountCtx::is_strong_session`, client kind `passkey` or
  `near`, `crates/trace-commons-server/src/account_session.rs:143`), and it
  passes **any** session on an account with zero strong authenticators (the
  bootstrapping carve-out). A native token (`tcn1_`) is weak: it "can read
  and withdraw, never change authenticators or redirect payouts"
  (`account_session.rs:90-95`). Accounts with a passkey use the step-up page
  (`docs/operator/native-step-up-page.md`), which runs in a browser, "adds no
  API, and nothing comes back to the app".
- **Ledger shape.** `trace_credit_ledger.points_delta` is `TEXT` and
  `settlement_state` is `TEXT` (V1). `account_credit_totals`
  (`trace-commons-ingest.rs:19765`) is a display path: it walks the
  account's own submissions for a period, sums `points_delta` as `f32` and
  rounds to whole points. It is not usable for money.
- **Pooled tenants are an operator list.** `trace_legacy_invite_pooled_tenants`
  (V81); nothing is inferred. `is_anchored_tenant`
  (`crates/trace-commons-protocol/src/admission.rs:218`) requires a `near-`
  or `nearai-` prefix and a 64-hex suffix.
- **Pinned hosts.** The daemon's `CLOUD_API_ORIGIN` is a compiled-in
  `https://cloud-api.near.ai` (`daemon/nearai_credential/api.rs:31`). The
  server's login introspection base is compiled in too, with no override in
  non-test builds: `NEAR_AI_API_BASE_URL`
  (`crates/trace-commons-ingest/src/bin/trace_commons_ingest_internal/near_provisioning.rs:996-999`),
  whose comment says a configurable host "would create the very thing the
  allowlist exists for".
- **Wallet and NEAR AI anchors differ.** A `nearai-` tenant is anchored on
  the NEAR AI login subject (`seal_login_subject`). A `near-` tenant is
  anchored on a NEAR wallet account
  (`near_account_identity.rs:450-463`, `random_near_tenant_id` and
  `random_near_ai_tenant_id`). A `near-` tenant has no NEAR AI subject on
  file.
- **Tenant enumeration.** The pipeline worker's tenant list comes from
  config and the routing cache (`pipeline_worker_tenant_ids`,
  `trace_commons_ingest_internal/pipeline_runtime.rs:2139`), not from the
  database. Unauthenticated cross-tenant reads use a narrow role assumed
  with `SET ROLE` (`docs/operator/register-stats-role.md`) or a separate
  pool (`docs/operator/login-resolver-role.md`), never `BYPASSRLS`.
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
  delta returns 201 with `id` and the new program total. So a send with
  `recipient_cumulative: 0` is an exact read of the program total, taken
  under the org locks, for any active recipient.
- **Errors.** The body is `{"error":{"type":<code>,"message":..}}`; the code
  is at `error.type` (`crates/api/src/models.rs:1439-1449`). 403 covers
  `credential_refused` and `sender_not_enabled`; the latter fires both when
  the sender has no policy row and when its policy says `enabled: false`
  (`crates/database/src/repositories/credit_transfers.rs:116`,
  `crates/services/src/organization_credits/models.rs:278`). 409 includes
  `capacity_overflow`, when the recipient's incoming transfer total would
  overflow (`credit_transfers.rs:272`). The status table is in
  `crates/api/src/middleware/organization_credentials.rs:35-48`.
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
  most 366 days out. The create body also requires `name` (non-empty, at
  most 200 bytes). Create, update and rotate need an owner session and
  `credentials.enabled`. Create returns the credential row (including its
  `id` and `token_prefix`) plus the token, once. Rotation invalidates the
  old token at once.
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
  returns `organizations[]` with `id`, `name`, `role` and `is_active`, sorted
  by join time and capped at 100 (`crates/api/src/routes/users.rs:172-235`). **There is no "personal"
  flag** and no server-to-server lookup by user. NEAR-wallet users get a
  personal org at sign-up named `<account>-org-<4 hex>`; other providers get
  `<email-local>-org-<4 chars>`, so an org name can carry part of an email
  address and is treated as personal data here. So the contributor chooses
  the org in the app (owner decision above) rather than the app guessing.

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
  settled credit today (`docs/trace-commons-roadmap.md:31-38`). Because a
  sent payout cannot be taken back, section 4 offers a maturation window
  before credit counts as owed.
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
   needs a five-type `CREDIT_USAGE_ORDER` (one that includes `transfer`),
   chosen deliberately and pinned together with the allocation policy
   version (`v2` or a newer explicit one), in the same deployment revision as
   the transfer-aware image. `grant,staking_farm,transfer,payment,postpay` is
   only the `env.example:201` default, not a requirement. Which order and
   version staging runs is not visible from the repo. Ask NEAR AI; the order
   decides which of a recipient's credit types is spent first.
5. **Mint the payout credential** with an owner session:
   `POST /v1/organizations/{org_id}/credentials` with a `name` (required,
   non-empty, at most 200 bytes; a plain label such as "commons payouts,
   staging", nothing about a person),
   `permissions: ["credits:read", "credits:transfer"]`, `expires_at` at most
   90 days out on staging (366 is the ceiling), and
   `constraints["credits:transfer"]` with `max_amount_per_transfer`,
   `max_total_amount` (below the org's), `allowed_source_types: ["grant"]`
   and, on staging, `allowed_recipient_ids` set to the test recipient orgs.
   Both permissions are needed: `credits:transfer` alone cannot read
   capacity or history, so it cannot reconcile.
6. **Hold the secret.** The token is shown once. Put it straight into the
   host's secret store. Ingest reads it from env only, as the scorer key is:
   - `TRACE_COMMONS_NEAR_AI_PAYOUT_CREDENTIAL` (held as `SecretString`,
     refused if empty or not `oc_` plus 64 hex);
   - `TRACE_COMMONS_NEAR_AI_PAYOUT_CREDENTIAL_ID` (the credential `id` from
     the create response; not secret, used only for the log hash below);
   - `TRACE_COMMONS_NEAR_AI_PAYOUT_SENDER_ORG` (the reseller org UUID);
   - `TRACE_COMMONS_NEAR_AI_PAYOUT_SOURCE` (`grant` or `payment`).

   There is **no base-URL setting.** The payout host is the same compiled-in
   NEAR AI API base that login introspection uses
   (`NEAR_AI_API_BASE_URL`), switched only by the `near-ai-staging` build
   feature (section 8). One constant serves both the `/users/me` check in
   section 5 and every send, so the host that verified an org is the host
   that pays it, and no env value can point the contributor's NEAR AI token
   or the payout credential at another host.

   These names are proposals; none exists yet. Follow
   `docs/operator/commons-credential-storage.md` and
   `docs/operator/key-rotation.md`. The token is never passed on a command
   line, written to a file in the repo, or logged. Logs carry only
   `sha256:` of the credential `id`, so an operator can tell which
   credential was live. (Not the `token_prefix`: it is 11 characters with
   32 bits of entropy, so hashing it hides nothing and identifies little.)
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

`NearAiTransferClient` holds the sender org, source type and the credential
as `SecretString`. Its base URL is not a field anyone sets: it is the
compiled-in NEAR AI API base. `NEAR_AI_API_BASE_URL` moves from the ingest
bin into `trace-commons-server` so the introspection call and this client
read one constant (a test-only constructor may take a loopback mock URL,
as the scorer's tests do). It copies the scorer client's shape: TLS or
loopback only, `redirect(Policy::none())`, a fixed user agent, a 15 s
timeout, `bearer_auth`. cloud-api bounds the send transaction with a 10 s
timeout in the database repository
(`crates/database/src/repositories/credit_transfers.rs:51-53`), so 15 s
leaves room for the reply.
It exposes four calls: `send_cumulative`, `send_amount` (Option A only),
`capacity`, and `history_page`. It parses the error body's `error.type`
field into a closed enum and keeps nothing else from an error.

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

- **Owed credit** is an exact integer read of `trace_credit_ledger` for one
  tenant: `WHERE tenant_id = trace_current_tenant_id() AND settlement_state
  IN (<settled states>)` and `instrument_id` either `trace_credit` or null
  (legacy rows), under the tenant's RLS context. Pending-review and
  held credit are excluded. It does **not** reuse `account_credit_totals`:
  that path is per account principal and per period, sums `points_delta`
  as `f32` and rounds to whole points, which is fine for a display and
  wrong for money. Each `points_delta` (`TEXT`) is parsed as an exact
  decimal into signed microcredits (6 decimal places). A value with more
  than 6 fractional digits, an exponent, or anything that is not a plain
  decimal refuses the tenant's line as `payout_ledger_value_invalid` and
  holds it; nothing is rounded. The settled-state list is a named constant,
  pinned in the plan and covered by a test against every state the ledger
  writes.
- **Maturation window (option, decision c).** Owed may count only events
  settled more than N days before the pass. The ledger has no settlement
  timestamp, only `occurred_at` (V1); the plan either adds one or uses
  `occurred_at` and says so.
  A sent payout cannot be reversed, so without a window a contributor can
  upload, settle, be paid the next day and leave before a revocation lands;
  the deficit that revocation leaves is then paid off only by honest
  contributors' later credit. N = 0 reproduces the behaviour above. The
  recommendation in section 11c includes a window; its length is the
  owner's.
- **Rate.** A new append-only table `trace_nearai_payout_rates`
  (`rate_version`, `nano_usd_per_credit`, `effective_from`, `set_by_hash`).
  The repository carries no production value, as with S2. A new rate is
  refused unless its `effective_from` is in the future at insert time and
  later than every existing epoch's, so no epoch can be backdated and
  epochs never overlap. A database check enforces the ordering, not only
  the operator tool.
- **Conversion per rate epoch.** Each ledger event belongs to the epoch in
  force at its time (the same timestamp the maturation window uses),
  except a reversal (a negative row that undoes an earlier event): it is
  priced at the epoch of the event it reverses, found through the shared
  `submission_id` and `trace_id`, so a revocation after a rate change takes
  back exactly what the event added. A reversal
  whose original cannot be found holds the tenant as
  `payout_reversal_unpriced`. `owed_nano = sum over epochs of
  floor(atomic_units_in_epoch * nano_usd_per_credit / 1_000_000)`. A rate
  change re-prices only new credit. Dust is lost at most once per epoch, not
  once per trace.
- **Consumption.** A payout converts Trace Credit; it must not leave the
  same credit spendable anywhere else. Each confirmed line records
  `credit_through_micro`: the settled microcredit total its target was
  computed from. The tenant's converted credit is the largest
  `credit_through_micro` on a confirmed line (cumulative targets only grow,
  so the maximum is the total). Every other rail that pays out Trace Credit,
  today `NearPayoutAdapter` and its outbox, and any future redemption, must
  subtract converted credit before paying, and Insights shows it as its own
  line ("Sent as NEAR AI credit"), with the remaining balance beside it.
  Whether this is a full redemption in the sense of the onboarding-funding
  spec (credit moved into a hold, then consumed, l.161-196) or a simpler
  conversion record is open decision 11e; until it is decided, this record
  is the minimum and no second rail may be enabled.
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
  (`waiting`, `active`, `closing`, `closed`), `sealed_org_id JSONB`, `org_index`
  (keyed blind index), `org_key_ref`, `index_pepper_ref`, `payout_program`,
  `final_paid_nano NUMERIC(20,0)` (set on close), `unlink_requested_at`,
  `version`, timestamps,
  and a `tenant_storage_ref` column for Option A. Three partial unique
  indexes allow at most one `active`, one `waiting` and one `closing` row
  per tenant (a `waiting` row can sit beside the `active` one until the
  pass applies the change). Only an `active` link receives sends.
- `trace_nearai_payout_lines`: `tenant_id`, `line_id` (UUID, the line's
  key, used in metadata and logs only as a hash), `link_id`, `batch_id`,
  `target_nano`, `credit_through_micro NUMERIC(39,0)`, `rate_version`,
  `state` (`pending`, `confirmed`, `no_op`, `held`, `failed`),
  `transfer_id_hash`, `delta_nano`, `attempts`, `last_error_label`.
  Unique on `(link_id, batch_id)`; a partial unique index on `(link_id)
  WHERE state = 'pending'`, so a link has at most one unresolved line; and
  a partial unique index on `(transfer_id_hash) WHERE transfer_id_hash IS
  NOT NULL`, so one transfer can answer only one line.
- **Enumeration.** The pass and the drill need the set of tenants with an
  `active` or `closing` link, which no tenant context can see under forced
  RLS. A narrow role, `trace_nearai_payout_enumerator`, with a permissive
  `SELECT` policy on `trace_nearai_payout_links (tenant_id, state)` only,
  assumed with `SET ROLE` for that one query, as
  `docs/operator/register-stats-role.md` does. It can read nothing else:
  no sealed column, no ledger, no line. Every per-tenant read and write
  after that runs under that tenant's `trace_current_tenant_id()`. The
  pipeline worker's config-driven list (`pipeline_worker_tenant_ids`) is
  not reused, because a linked tenant need not be routed to the
  pipeline.

### One pass

0. **Single flight.** The pass takes a session-level PostgreSQL advisory
   lock on a fixed key (`pg_try_advisory_lock(hashtextextended(
   'nearai-payout-pass-v1', 0))`, the same hashing the pipeline uses at
   `versioned_pipeline.rs:2177`) and exits at once if another process holds
   it. The pass pins one connection for its whole run, so a pooled
   connection never carries the lock back to the pool, and releases it with
   `pg_advisory_unlock` on every exit path; a crash drops the connection and
   with it the lock. (If pinning a connection fights the per-tenant
   `SET LOCAL` pattern, a lease row with an expiry is the alternative.)
   Every change to a link's state (`active` to `closing`, `closing` to
   `closed`, `waiting` to `active`) is made by the pass while holding it.
   The link and unlink routes only record a request: a new `waiting` row,
   or `unlink_requested_at` on the live link. At the start of each tenant's
   turn, the pass applies any request (moving the live link to `closing`)
   before it considers a send. So no send can be in flight to a link when
   it starts closing.
1. Enumerate tenants (above). Take each tenant whose link is `active`, who
   is eligible (section 6), and whose link has **no `pending` line**. A
   tenant with a pending line goes to crash recovery first and gets no new
   line this pass. Read the link's program total with a
   `recipient_cumulative: 0` send (section "cloud-api #1271"): an exact
   total, no row, no quota. Skip the tenant unless its target exceeds that
   total. Withdrawal already forfeits legs before they reach the ledger, so
   the ledger sum is the guard; the pass adds none.
2. Write the line `pending` with its `target_nano` and
   `credit_through_micro`, committed, before any network call.
3. Probe `capacity` once per pass. If `sending_enabled` is false,
   `sender_reconciled` is false, or `credential_can_transfer` is false, stop
   the pass with that label. Pace sends to `sends_remaining_in_window` and to
   a local limit below `max_sends_per_minute`.
4. Send `recipient_cumulative = target_nano` with the link's program. Memo
   empty. Metadata `{"tc": "<sha256 of 'nearai-payout-line-v1:' || line_id>"}`,
   an opaque handle for support with no contributor identity.
5. Record the outcome:
   - 201: `confirmed`, `transfer_id_hash = sha256("nearai-transfer-v1:" || id)`,
     `delta_nano = amount`. If the returned program total is not the
     target, hold as `payout_program_total_mismatch`.
   - 200 `no_op`: if `recipient_cumulative >= target`, run recovery (below);
     otherwise hold as `payout_program_total_mismatch`.

### Crash recovery

A crash between step 2 and step 5 leaves a `pending` line whose send may or
may not have committed. Because a link has at most one pending line and the
pass is single-flight, no later line can have been written or sent for that
link in between. The next pass re-sends the **same** target before anything
else for that link:

- If the first send never committed, it is sent now (201).
- If it did, the reply is `no_op`. The pass then pages
  `GET .../credit/transfers?direction=outgoing` and filters client-side for
  this recipient, this program and `recipient_cumulative == target_nano`,
  skipping any transfer whose id hash is already on another line. The match
  gives the transfer id. History has no program filter, and the cursor is
  still returned on the last page, so the pass stops on a short page.
- If no row matches, hold the line as `payout_recovery_unmatched` for the
  operator. Under the one-pending-line rule this cannot happen without a
  bug or an outside send on our program, so it is not quietly marked
  `no_op`.

### Error mapping

| #1271 reply | Line state | Notes |
|---|---|---|
| 429 `transfer_rate_limited`, 503 `transfer_unavailable` / `accounting_busy`, timeout | stays `pending` | retried next pass, same target |
| 409 `insufficient_posted_capacity`, `sender_not_reconciled` | pass stops, `held` | operator: fund or reconcile the sender |
| 409 `credential_transfer_limit_exceeded`, `organization_transfer_limit_exceeded` | `held` | budget or recipient restriction; the code does not say which, so compare with `capacity` |
| 409 `capacity_overflow` | `held` | the recipient's incoming transfer total would overflow `i64`; operator review |
| 404 `organization_unavailable` | `held`, link marked `recipient_unavailable` | contributor must relink |
| 403 `credential_refused` | pass stops, fail closed | credential malformed, expired, revoked or wrong |
| 403 `sender_not_enabled` | pass stops, fail closed | the sender has no policy row, or its policy has transfers disabled; ask NEAR AI |
| 409 `idempotency_conflict` (Option A), 400 any | `failed` | a bug on our side; page the operator |

For Option A, the same table maps to `SettlementError`, which has three
unit variants (`Unavailable`, `Conflict`, `Rejected`) and carries no label:
retryable rows and held rows both map to `Unavailable` (the leg must not be
charged for a funding gap), and `failed` rows to `Rejected`. The runner
sees only `Unavailable`; the adapter records the hold and its label in its
own table, and readiness reads them from there.

### Fail-closed rules

- Payout configured but any of the four env values missing or malformed:
  ingest refuses to start with `near_ai_payout_config_missing`.
- The payout host is compiled in (section 2, step 6). A build with the
  `near-ai-staging` feature switches it for introspection and payout
  together, is refused as a release build, and reports itself in readiness.
  No env value names a host.
- `production_qualified` is true only when the build is not
  `near-ai-staging`, the credential is set, a rate is in force, and the
  startup capacity
  probe answers `sending_enabled: true`. Readiness reports the rail as
  `near_ai` with `qualified` and `payout_enabled`, as the existing JSON does
  (`versioned_pipeline_production.rs:933-975`).
- No rate in force: no line is written. Nothing falls back to a default.
- A link whose sealed org id fails to open: hold `payout_link_unreadable`.

### Hash-only logging

Logs and audit rows carry labels and `sha256:` values only: line id hash,
link id hash, transfer id hash, program hash, credential id hash, amounts
in nano-USD. Never the recipient or sender org UUID, the org name, the
token, the memo, the metadata, or the tenant id. The server never reads,
stores or logs a NEAR AI org name (section 5). This follows `docs/operator/hash-only-logging.md`.

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
2. It calls `/v1/users/me` itself and the app shows the active orgs where
   the user is owner, by name, for the contributor to choose from (owner
   decision, 2026-10-21). Nothing is preselected when there is more than one;
   with exactly one, it is shown as the only choice and still needs a
   confirm. Orgs the user does not own, or that are inactive, are not
   offered, because step 4 would refuse them. The name stays on the device:
   the daemon keeps it with the chosen org id, as it already keeps
   `organization_id`, and re-reads it from `/users/me` for the status line
   later. Choosing again later is the change flow below.
3. On confirm, the daemon sends the chosen org id **and the fresh NEAR AI
   access token in the request body** to ingest, as provisioning does
   (`daemon/nearai_onboarding.rs:32-42`).
4. Ingest calls `GET /v1/users/me` once with that token, at the
   compiled-in NEAR AI API base (the same constant the payout client uses;
   section 2, step 6), then drops the token. It reads the response into a
   **new, single-purpose struct**, not `IntrospectedUser`, holding `id` and
   `organizations[]{id, role, is_active}` only. It does not deserialize
   `name`. It checks that `id` matches the account's anchored NEAR AI
   subject (the ownership proof below), and that the chosen org is in the
   list, active, with `role == "owner"`.
5. Ingest sends a liveness probe: `recipient_cumulative: 0` on the fixed
   probe program `tcprobe-v1`, from the reseller credential. The name is
   outside the `tc1-` namespace, so the reconciliation drill never counts
   it, and since a zero target never sends, no history row ever carries it.
   Per the check order above, that returns 200 `no_op` only for an active
   recipient, writes no row and uses no quota. A 404 refuses the link as
   `near_ai_org_unavailable`. The send policy runs first, including the
   credential's `allowed_recipient_ids`, so on a closed pilot an org not on
   the list returns 409 `credential_transfer_limit_exceeded`. That refuses
   the link as `near_ai_org_not_permitted`, which is expected on staging. A
   403 refuses as `payout_unavailable`: either our credential is bad
   (`credential_refused`) or the sender org has no policy or has transfers
   disabled (`sender_not_enabled`).
6. Ingest stores the link and answers `linked` with the new version and an
   `org_ref` (below). It never returns, stores or logs the org name.

### Ownership proof per eligible class

Step 4 needs a NEAR AI login subject on file for the account. Not every
class in section 6 has one:

| Account class | Anchor on file | Proof at link time |
|---|---|---|
| `nearai-` tenant | NEAR AI login subject (`seal_login_subject`) | `/users/me` `id` matches the login blind index |
| `near-` tenant (wallet) | NEAR wallet account | none today: the wallet is not a NEAR AI subject |
| passkey-origin, `bound` | whatever the binding anchored (wallet or NEAR AI login) | as the row above for that anchor |
| legacy `tenant-...`, linked | the NEAR account it linked to (V81) | as the row above for that anchor |

So, as specified, only accounts that hold a NEAR AI login anchor can link.
For the others there are two ways forward, listed under decision 11b:
narrow eligibility to accounts with a NEAR AI login subject (simplest; the
others become eligible once they also sign in with NEAR AI and that login
is bound to the account), or add a separate proof for wallet accounts
(for example a wallet-signed statement naming the org, verified as the
wallet ceremony is today). The spec does not pick one.

### Getting a strong session and the NEAR AI token into one request

The link route sits behind `require_authenticator_change_allowed`, and two
facts collide:

- The NEAR AI access token is only in the daemon, which talks to ingest
  with its `tcn1_` native token. That token is weak, so on an account with a
  passkey the gate returns 403.
- The step-up page has a strong 15-minute browser session, but it "adds no
  API, and nothing comes back to the app", and it never holds the NEAR AI
  token.

So, as built, neither path can complete a link on an account with a
passkey. Two designs, decision 11d:

- **Link ceremony.** The daemon starts a link: it sends the org id and the
  fresh NEAR AI token over its weak session, and ingest runs steps 4 and 5
  at once, drops the token, and stores a `pending_confirmation` link
  request with a short lifetime (5 minutes), bound to the account and
  returning an opaque ceremony handle. The daemon opens the step-up page
  with `return=confirm-payout-org` (a fourth section, so the page gains one
  route). The person signs in with a passkey and confirms there; the page
  shows nothing about the org but "the organization you chose in the app",
  since the server does not hold the name. Only the strong session can
  confirm, and only the account that started the request. Expiry or a
  second request discards it. This keeps the strong-authenticator rule
  intact.
- **NEAR AI token as the second factor.** A freshly exchanged NEAR AI
  access token proves a live NEAR AI sign-in, and for a `nearai-` account
  that is the account's own login. The route could accept a weak session
  plus a fresh NEAR AI token whose subject matches, without step-up. This
  is simpler. It weakens the rule for this one route: a NEAR AI login
  mints a native, weak session today (`NATIVE_SESSION_CLIENT_KIND`,
  `near_provisioning.rs:1320`), and only `passkey` and `near` (wallet)
  sessions count as strong, so anyone holding the daemon's refresh token
  and its `tcn1_` token (one stolen device) could redirect payouts.

The recommendation is the ceremony; the owner decides.

Separately: on an account with **zero** strong authenticators, the gate's
bootstrapping carve-out lets any weak session, including a device-linked
one, link or change the payout org. Section 9 lists this.

### The registration call

`POST /v1/account/near-ai/payout-org`, copying the inference-connection
stack (`crates/trace-commons-contributor/src/inference_connection.rs:190-230`,
`crates/trace-commons-ingest/src/bin/trace_commons_ingest_internal/inference_connection.rs`):

- account session only; `DeviceBearer` is refused with 403;
- body `deny_unknown_fields`: `organization_id`, `near_ai_access_token`,
  `idempotency_key` (non-nil UUID), `expected_current_version`;
- **behind `require_authenticator_change_allowed`.** A link is a payout
  redirect. How an account with a passkey satisfies it is decision 11d
  (above);
- outcomes: `linked`, `idempotency_conflict` (409), `version_conflict`,
  `subject_mismatch`, `org_not_owned`, `near_ai_org_unavailable`,
  `near_ai_org_not_permitted`, `account_ineligible`;
- audit row `account_payout_org_linked` with the link id hash and version
  only.

`DELETE` on the same path unlinks, under the same gate.
`GET` returns `{linked, org_ref, state, version}`, never the id or a name.
`org_ref` is `sha256("nearai-payout-org-ref-v1:" || idempotency_key || ":"
|| org_id)`, with the `idempotency_key` the daemon sent on that link
request. The daemon keeps both beside each other, so it can recompute the
ref and check that the server holds the same org without the server echoing
it; the per-request key stops two links' refs from being compared. The status line's org
name comes from the daemon's own `/users/me` read, matched by that id.

### Storage

The send needs the raw UUID, so hash-only storage is impossible. The org id
is sealed with the same machinery as `sealed_account_name`: ciphertext in
`sealed_org_id`, a keyed blind index in `org_index`, and key and pepper refs
for rotation. The blind index lets an operator find two tenants pointing at
one org (section 9) without opening either. The key is not in the database.

### Change and unlink

- **Change** inserts the new generation as `waiting`. The route changes
  no existing link's state. The next pass, under its lock, moves the old
  link to `closing` before doing anything else for that tenant.
- **Closing.** The pass, holding the single-flight lock, sends nothing new
  to a closing link and never re-sends to it. First it resolves every
  `pending` line on it **through reads only**: a `recipient_cumulative: 0`
  read of the program total and, to find the transfer id, history paging
  (crash recovery above, without the re-send). A line whose target the
  total reached is `confirmed` with its transfer; one it did not reach is
  `no_op` (its send never committed, and none will follow). Only when no
  line on the link is `pending` does it write `final_paid_nano` (the read
  total), set the old link `closed` and the waiting one `active`, in one
  transaction. A waiting link gets no sends, so the two never race. A
  second change before the first has completed replaces the `waiting`
  row.
- **Old org gone.** If the old org is deactivated, the zero read returns
  404. The pass then takes the program total from history alone: the sum
  of outgoing rows for that recipient and program (history still lists
  them). So a deactivated org cannot leave a link stuck in `closing` and
  block its successor. If history cannot be read either (403 or 503), the
  link stays `closing` and the pass retries next time, with
  `payout_link_close_blocked` in readiness.
- **Unlink** records `unlink_requested_at` on the live link; the pass
  closes it the same way, with no successor. Credit keeps accruing and is
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
  Anyone else in {org name} can use it too. If {org name} owes NEAR AI for
  past use, new credit pays that first. You can change or remove this
  organization in Settings."
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
  `trace_account_bindings` (V97); an `unbound` one has no anchor at all.
- **But only some of them can prove an org today.** Linking needs a NEAR AI
  login subject to match against `/users/me` (section 5, "Ownership proof
  per eligible class"). `nearai-` tenants have one. `near-` (wallet)
  tenants, and bound or legacy accounts anchored on a wallet, do not.
  Whether to narrow eligibility to accounts with a NEAR AI login subject,
  or to add a wallet proof, is part of decision 11b.
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
- **Maturation window.** Any of the above can count only credit settled
  more than N days ago (section 4). With a window, a daily batch pays each
  day's credit N days later. It is the only protection against
  upload-settle-cash-out-leave, since a transfer cannot be reversed.

At 60 sends a minute a daily pass reaches 86,400 tenants in a day, far above
the pilot. The quota can be raised by policy.

## 8. Staging test plan and reconciliation drill

### Known staging gap

The daemon's cloud-api origin and the server's NEAR AI API base are both
compiled in as production, and the payout client uses that same base
(section 2, step 6). Neither the link flow nor the pass can reach staging as
built. Plan: a build-time feature (`near-ai-staging`) that switches the
daemon origin and the server base (introspection and payout together),
refused in a release build and reported in readiness. Until it exists, test the
transfer client against the mock #1271 server, and test the link flow
against production with a closed recipient list. A live staging payout run
waits for the feature.

Staging and production are separate deployments. Org ids differ; staging
contributors need staging accounts.

### Steps

1. Section 2 setup with `allowed_recipient_ids` set to two test orgs.
2. Unit: exact `points_delta` parsing (and refusal of excess precision);
   rate epochs, dust and backdated-rate refusal; reversal priced at its
   original's epoch; `u128` to `i64` bounds; error mapping; program
   derivation; target after a closed link; `credit_through_micro`.
3. Mock #1271 server tests: 201; `no_op` then history recovery; lost
   response; 429 pacing; each 409 hold (including `capacity_overflow`);
   both 403 causes stop the pass; a second pass refused while the first
   holds the lock; a link with a pending line gets no new line; closing a
   link with a pending line resolves it by reads only; closing after the
   old org is deactivated (404) completes from history.
4. Staging, live: two tenants, one pass. Check delta and program totals.
5. Kill ingest after step 2 of a pass and before step 5. Re-run. Expect one
   transfer, not two.
6. Change org for one tenant. Expect the closing link settled, then the new
   program starting at owed minus paid.
7. Exhaust `max_total_amount`. Expect a hold, no partial state.
8. Revoke the credential. Expect the pass to stop with `credential_refused`
   and readiness to drop.
9. Recipient org deactivated. Expect `recipient_unavailable`.
10. Deactivate a test org while its link is `closing`. Expect the close to
    complete from history and the waiting link to become active.

### Reconciliation drill

`POST /v1/admin/nearai-payout-reconcile-drill`, following
`docs/operator/drills.md`: admin bearer, JSON body, `ready` plus
label-only `blocking_gaps`, hash-only evidence, wired into
`rollout-smoke/evidence`. It finds links through the enumeration role
(section 4) and reads each tenant's lines under that tenant's context. It
pages outgoing history and compares, per link:
the sum of our confirmed `delta_nano` against the program total; every
confirmed `transfer_id_hash` against a history row; and no history row in a
`tc1-` program that we do not hold (the `tcprobe-v1` liveness probe is
outside that namespace and, sending nothing, leaves no row). Gaps: `payout_history_unmatched`,
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
  twice. The pass is single-flight, a link has at most one pending line,
  and a link closes only after its pending lines are resolved by reads, so
  `final_paid_nano` is never read short of a send in flight. In Option A
  the idempotency key does the same job.
- **Double redemption.** A payout records the credit it converted
  (section 4, "Consumption"); every other Trace Credit rail must subtract
  it. Until 11e is decided, no second rail may be enabled.
- **Cash out and leave.** Settled credit paid before a revocation lands
  cannot be recovered. The maturation window (decision 11c) is the
  control; without it, the gap is paid down by later honest credit.
- **Wrong recipient.** The org must be in the contributor's own
  `/users/me`, active and owned, checked by the server with the
  contributor's own token, against the same compiled-in host that pays.
  How a link change meets the strong-authenticator rule is decision 11d. A
  wrong send cannot be reversed; transfer credit cannot be re-sent.
- **Shared org.** "Owner" is not "personal". Credit sent to an org with
  other members is spendable by all of them, which sits uneasily with the
  holder-only redemption rule. Two of our tenants owning one org is visible
  through the blind index. Whether to allow, flag or refuse a shared org is
  decision 11f; the consent copy says other members can use the credit
  either way.
- **Recipient debt.** Incoming transfer credit first pays any overage the
  recipient org owes (`amount_applied_to_debt` in the recipient snapshot).
  The consent copy says so.
- **Sender out of funds.** 409 `insufficient_posted_capacity` holds the
  pass. Lines stay owed. Readiness reports it. No partial transfer is sent.
- **Rounding.** Floor per rate epoch; at most one nano-USD lost per tenant
  per epoch in Option B, per trace in Option A.
- **Metadata leak.** Memo empty, metadata an opaque hash. Both sides can read
  them, and we put nothing about the contributor there.
- **Over-reach by a weak session.** On an account with a passkey, a
  `tcn1_` token cannot link or unlink by itself (decision 11d chooses what
  it pairs with). On an account with **zero** strong authenticators, the
  bootstrapping carve-out in `require_authenticator_change_allowed` lets any
  weak session, including a device-linked one, link or change the payout
  org. That is the same exposure the existing NEAR payout redirect has; the
  plan should either accept it explicitly or require at least one strong
  authenticator before a first link.
- **Org name.** Non-wallet org names embed an email local part. The server
  never deserializes, stores or logs the name; only the daemon shows it.
- **Enumeration role.** The cross-tenant role reads only `tenant_id` and
  `state` of the link table. A leaked connection under it reveals which
  tenants are linked, nothing about whom to.

## 10. Delivery plan

Small stacked PRs. Each one passes the repo checks on its own.

1. **Protocol types** for link, unlink and status (permissive crate).
2. **Migration** for links, lines and rates, with RLS tests.
3. **Transfer client** with the mock #1271 server and the error table.
4. **Link route** in ingest, server-side `/users/me` check, liveness probe,
   the strong-authenticator path chosen in 11d (and the step-up page's
   fourth section if it is the ceremony), audit.
5. **Daemon and macOS** link flow and copy keys (draft copy behind a
   setting until the owner signs it off).
6. **Payout pass** (Option B), behind config, with crash points, the
   single-flight lock, the enumeration role, and the consumption record
   shown in Insights.
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
- Within whichever classes are chosen, how ownership of the org is proven
  (section 5, "Ownership proof per eligible class"): only accounts with a
  NEAR AI login subject (others join by signing in with NEAR AI and
  binding that login), or also wallet accounts through a new wallet-signed
  proof.
- **Recommendation:** section 6. Anchored accounts and linked legacy
  tenants. Pooled tenants excluded. For the proof, NEAR AI login subject
  only at first; a wallet proof later if wallet users ask for it.

### (c) When we pay

- Options: daily batch; weekly batch; per settled credit. Any of them with
  or without a maturation window (credit counts as owed only N days after
  it settles).
- This choice also picks the seam: per settled credit means Option A
  (per-leg adapter, amount mode); a batch means Option B (payout pass,
  cumulative mode).
- **Recommendation:** daily batch, Option B, with a maturation window
  whose length the owner sets.

### (d) How a payout-org link meets the strong-authenticator rule

- Options: a link ceremony started by the daemon and confirmed on the
  step-up page (section 5); or a fresh NEAR AI token with a matching
  subject accepted as the second factor on a weak session.
- Also: whether the zero-strong-authenticator carve-out applies to this
  route, or a first link requires a strong authenticator.
- **Recommendation:** the ceremony; keep the carve-out consistent with the
  existing NEAR payout redirect unless the owner wants it closed for both.

### (e) Does a NEAR AI payout redeem Trace Credit

- Options: a full redemption per the onboarding-funding state machine
  (credit held, then consumed on confirmation); a conversion record only
  (section 4, "Consumption"), with every other rail subtracting it; or no
  consumption, with NEAR AI credit as a bonus on top of Trace Credit.
- The holder-only redemption rule suggests the credit is used up.
- **Recommendation:** the conversion record, shown in Insights, and a
  rule that no second rail is enabled until it subtracts converted credit.

### (f) Shared organizations

- Options: allow (credit is spendable by every member); flag for review
  when two of our tenants own one org; refuse orgs with other members (the
  server cannot see membership today, so this needs a NEAR AI change or
  the daemon's word).
- **Recommendation:** allow with clear consent copy, and flag two tenants
  on one org for review.

### Other open questions (not owner decisions yet)

1. **Personal org.** Settled by the owner decision above: the contributor
   picks from owned, active orgs in the app, so no personal flag is needed.
2. **Staging API host:** `cloud-stg-api.near.ai` or `cloud-stg.near.ai`.
3. **Staging deployment flags** (`CREDIT_ALLOCATION_POLICY_VERSION=v2`,
   `CREDIT_USAGE_ORDER`): unknown.
4. **History filter.** No `payout_program` filter; recovery and the drill
   page all outgoing history. Fine at pilot scale; ask NEAR AI for a filter
   before it is not.
5. **Restriction vs budget.** One error code covers both. We infer from
   `capacity`.
6. **Credit predicate.** Section 4 fixes the shape (exact integer read by
   tenant, `settlement_state` and instrument); the plan pins the list of
   settled states and whether the ledger needs a settlement timestamp.
7. **Minimum send floor** and **credential expiry** values.
8. **Spending attribution.** Deferred in #1271, so we cannot show a
   contributor how much of their NEAR AI spend came from the commons.
