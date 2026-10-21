# Paying contributors in NEAR AI credits -- design

Date: 2026-10-21. Owner: Zaki. Status: draft, nothing built. The owner
settled every design decision on 2026-10-21 (below). What is still open is
listed in section 12. Section 11 maps each point of the review of the
earlier settlement-rail draft (#1415) to where this design handles it.

## Owner decisions

All dated 2026-10-21. The owner's words are quoted.

- **(D1) The contributor chooses the payout org in the app.** The app lists
  the NEAR AI organizations the contributor's sign-in can see and the
  contributor picks one. There is no automatic "personal org" guess, and no
  NEAR AI change is needed to find the org. The server still verifies the
  choice (section 5); a pick is never trusted on the client's word.
- **(D2) Rate.** "We should have work in the repo on exchange rate based on
  the profiling my trace data on my machine." The payout rate is the
  settlement estimate's `usd_micros_per_credit`
  ([`2026-10-15-settlement-estimate-design.md`](2026-10-15-settlement-estimate-design.md),
  decision S2): the operator calibrates it from the owner's own traces with
  the fit in `crates/trace-commons-server/src/credit_estimate_fit.rs` and
  installs it in the settlement table. The value is operator configuration.
  It is not in the repository and this spec does not state it. The payout
  reads the rate from the installed table, so the estimate and the payout
  agree, and a rate epoch is a table version (section 4, "Rate").
- **(D3) Eligibility.** "users with near cloud accounts can be paid."
  Any account that links a NEAR AI cloud account is eligible, whatever its
  tenant class, by proving a live NEAR AI sign-in that owns the chosen org
  (section 5). Wallet-only accounts join by signing in with NEAR AI in the
  app. Payment is per account, not per tenant (section 6 says what this
  means for pooled and legacy tenants).
- **(D4) Timing.** "We should pay out users as quickly as possible."
  Each settlement triggers a payout pass, with a short retry tick behind it.
  There is no maturation window. Cumulative mode keeps every send
  exactly-once. Credit later reversed after it was paid is netted against
  that account's future credit and never clawed back, unless #1271 later
  offers a reversal (section 4, "Reversals"; section 9).
- **(D5) Link confirmation.** A passkey step-up confirms every link change:
  link, change and unlink are all confirmed on the step-up page with a
  passkey before the server saves anything. An account with no passkey must
  add one first. The zero-strong-authenticator carve-out does not apply to
  this route (section 5).
- **(D6) Parallel to Trace Credit.** "Near ai payout is parallel to trace
  credit." A NEAR AI payout does not consume, redeem or convert Trace
  Credit. The contributor keeps their Trace Credit, and whatever pays it
  out (the NEP-141 rail, a future redemption) is unaffected. There is no
  conversion record and no rule that blocks a second rail. This departs on
  purpose from the earlier principle that credit is redeemable by its holder
  only once: the owner chose to fund NEAR AI credit as a parallel award on
  the same settled credit, not as a redemption of it.
- **(D7) Shared organizations.** "Shared organizations can be paid."
  Allowed. The consent copy says other members of the org can spend the
  credit. No review flag: it would mean widening the cross-tenant
  enumeration role (section 4) to the org blind index, and nothing would act
  on it.

This spec covers paying Trace Commons contributors in NEAR AI inference
credit through the org-to-org transfer API that nearai/cloud-api PR #1271
added. It covers the commons side (a transfer client, the payout pass, crash
recovery and reconciliation), the contributor side (linking a NEAR AI
organization), eligibility, a staging test plan and the threat model.

It builds on, and does not repeat:

- [`2026-09-09-versioned-pipeline-design.md`](2026-09-09-versioned-pipeline-design.md),
  the authority for instruments, settlement legs and payout rails.
- [`2026-09-04-onboarding-funding-contract-design.md`](2026-09-04-onboarding-funding-contract-design.md),
  which set the rules for an account -> provider identity -> org mapping
  (l.154-159). Its redemption state machine (l.161-196) does not apply here
  (D6). Its premise that no provider operation offered idempotency or an
  operation lookup is no longer true after #1271.
- [`2026-10-15-settlement-estimate-design.md`](2026-10-15-settlement-estimate-design.md),
  which put the credit-to-dollar rate in operator hands
  (`usd_micros_per_credit`, decision S2). D2 makes it the payout rate too.

It supersedes the settlement-rail draft in #1415 (closed).

## What already exists, verified on 2026-10-21

Our tree is `origin/main` at f164d68f9. cloud-api is the #1271 merge commit
71bbb0be. Paths without a repository name are ours.

### Our settlement code

- **Settled credit is already split per account.** A finalized
  `trace_credit_settlement_batches` row (V2) carries `line_items_json`, one
  `TraceCreditAccountSettlementLineItem` per credit account
  (`crates/trace-commons-server/src/trace_corpus_storage.rs:773`):
  `credit_account_ref`, `credit_account_hash`,
  `settled_credit_delta_micros` (i64), `source_credit_event_ids`, and
  `near_payout_hold_reason`. The ref is the settlement key from
  `settlement_group_key` (`crates/trace-commons-server/src/db/mod.rs:2262`):
  `account:{account_id}` when the principal is linked to an account in
  `trace_account_principals` (V30), otherwise the bare principal.
  `settlement_key_account_id` recovers the account id. The pipeline writes
  one such line per batch (`finalize_pending_credit_batch`,
  `crates/trace-commons-server/src/versioned_pipeline.rs:16181`); the
  ingest settlement route writes many.
- **Accounts are per tenant.** `trace_accounts` is keyed
  `(tenant_id, account_id)` (V30). A pooled tenant can hold many accounts.
- **NEAR payout holds are about the wallet target.**
  `near_payout_hold_reason` is `none_enrolled` or
  `ambiguous_no_designation` (`PayoutHoldReason`, `db/mod.rs:2233`): the
  account has no NEAR wallet to pay, or several and none designated. The
  NEP-141 rail pays each line to that account's own wallet
  (`payout_target_for_line`, `versioned_pipeline.rs:17129`) and writes no
  outbox row for a held line.
- **Credit holds are a separate thing.** An open `trace_credit_holds` row
  (`released_at IS NULL`, keyed by `credit_account_ref`) keeps that
  principal's events out of settlement entirely
  (`trace-commons-ingest.rs:30415-30436`).
- **Reversals.** Revocation does not claw back settled credit
  (`docs/trace-commons-roadmap.md:36-38`); "the reversal action remains for
  operator-raised corrections". That action is a revocation-propagation
  item with a `CreditSettlement` target
  (`crates/trace-commons-server/src/trace_corpus_storage.rs:1956`). It
  writes a negative ledger row with `settlement_state = 'reversed'` and
  `external_ref = "revocation_credit_reversal:{credit_event_id}"`, naming
  the event it reverses (`trace-commons-ingest.rs:66873-66970`, prefix at
  l.26467). Settlement states are `pending`, `final`, `reversed`
  (`TraceCreditSettlementState`).
- **The seam.** `crates/trace-commons-gate-api/src/settlement.rs` (AGPL)
  defines `SettlementAdapter`; `SettlementRequest` carries no recipient and
  `operation_ref_hash` is one per run
  (`versioned_pipeline_bundle.rs:288`). This design does not use it
  (section 3).
- **A second payout seam exists for NEAR.** `NearPayoutAdapter`
  (`versioned_pipeline_credit.rs:262-276`) pays from `trace_near_credit_outbox`
  in `process_payouts` (`versioned_pipeline.rs:16348`) and
  `dispatch_near_settlements` (l.16629). Production never enables it.
  This design neither reads nor writes that outbox (D6).
- **Units.** Trace Credit has 6 decimals: 1 atomic unit = 1 microcredit.
  #1271 amounts are i64 nano-USD.

### The settlement table

- The operator installs it as a file named by
  `TRACE_COMMONS_CREDIT_SETTLEMENT_TABLE_PATH`, validated at boot with the
  protocol parser; an invalid file fails boot with
  `credit_settlement_table_invalid`
  (`crates/trace-commons-ingest/src/bin/trace_commons_ingest_internal/credit_estimate.rs:104-109`).
- `SettlementEstimateTable` (`crates/trace-commons-protocol/src/settlement_estimate.rs:98-111`)
  carries `version`, `pricing_version` (optional), `atomic_units_per_credit`,
  `usd_micros_per_credit` (optional; absent or zero draws nothing) and
  `published_at`. Tests and fixtures carry only synthetic rates.

### Our identity code

- **The commons does not know any contributor's NEAR AI org.**
  `IntrospectedUser` in `crates/trace-commons-server/src/near_ai_login.rs`
  keeps only `id` and `auth_provider`, and its doc comment forbids adding a
  field "without deciding it is anchorable". No table holds an org id.
- **The daemon knows one org.** Sign-in mints an inference key in the first
  active org from `GET /v1/organizations`
  (`crates/trace-commons-contributor/src/daemon/nearai_credential/api.rs:427`)
  and stores `organization_id` as non-secret metadata
  (`daemon/stored_cloud_credentials.rs:40-46`). It holds a refresh session
  (`near_ai_session`) and already re-reads org state with a fresh exchange
  (`near_ai_funding`, `daemon/nearai_credential/funding.rs:141`).
- **Sealing pattern.** `trace_near_account_anchors` stores a keyed blind
  index plus a sealed value and key refs (V61), sealed by
  `crates/trace-commons-server/src/near_account_identity.rs`
  (`seal`, `seal_login_subject`).
- **Strong sessions.** `AccountCtx::is_strong_session` is true for client
  kinds `passkey` and `near`
  (`crates/trace-commons-server/src/account_session.rs:143`). A native
  token (`tcn1_`) is weak (l.90-95).
  `require_authenticator_change_allowed`
  (`crates/trace-commons-ingest/src/bin/trace-commons-ingest.rs:22776`)
  passes a strong session, and passes any session on an account with zero
  strong authenticators. D5 needs a stricter gate (section 5).
- **Step-up page.** `docs/operator/native-step-up-page.md`: a browser page
  with sections chosen by `return=` (`add-passkey`, `remove-passkey`,
  `change-payout`). It "adds no API, and nothing comes back to the app".
- **Wallet and NEAR AI anchors differ.** A `nearai-` tenant is anchored on
  the NEAR AI login subject; a `near-` tenant on a NEAR wallet account
  (`near_account_identity.rs:450-463`).
- **Pinned hosts.** The daemon's `CLOUD_API_ORIGIN` is a compiled-in
  `https://cloud-api.near.ai` (`daemon/nearai_credential/api.rs:31`). The
  server's login introspection base is compiled in too
  (`NEAR_AI_API_BASE_URL`,
  `trace_commons_ingest_internal/near_provisioning.rs:996-999`), with a
  comment that a configurable host "would create the very thing the
  allowlist exists for".
- **Cross-tenant reads** use a narrow role assumed with `SET ROLE`
  (`docs/operator/register-stats-role.md`) or a separate pool
  (`docs/operator/login-resolver-role.md`), never `BYPASSRLS`.
- **Outbound secret patterns.** The NEAR AI scorer reads its key from env
  only (`versioned_pipeline_production/gate_env.rs:30-38`), sets
  `redirect(Policy::none())`, requires TLS or loopback and refuses an empty
  key. `reqwest` 0.12 and `secrecy` 0.10 are already server dependencies.
  **No new dependency is needed.**

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
- **Cumulative mode** needs `payout_program` and forbids an
  `Idempotency-Key`. Under the org locks the server sends
  `max(target - SUM(program), 0)`, where the sum covers sender + recipient
  + program across credentials and sources
  (`crates/database/src/repositories/credit_transfers.rs:121-163`). A zero
  delta returns 200 `{"amount":0,"no_op":true,"recipient_cumulative":<total>}`,
  writes no row, uses no quota and **returns no transfer id**. A positive
  delta returns 201 with `id` and the new program total. So a send with
  `recipient_cumulative: 0` is an exact read of the program total, taken
  under the org locks, for any active recipient.
- **Amount mode** needs an `Idempotency-Key` (<= 100 chars). Not used here.
- **Errors.** The body is `{"error":{"type":<code>,"message":..}}`
  (`crates/api/src/models.rs:1439-1449`). 403 covers `credential_refused`
  and `sender_not_enabled` (no policy row, or `enabled: false`;
  `credit_transfers.rs:116`, `models.rs:278`). 409 includes
  `capacity_overflow` (`credit_transfers.rs:272`). The status table is in
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
  `allowed_source_types`, `allowed_recipient_ids` (<= 1000; empty permits
  none, null permits any). `expires_at` must be in the future and at most
  366 days out. The create body also requires `name` (non-empty, at most
  200 bytes). Create, update and rotate need an owner session and
  `credentials.enabled`. Create returns the credential row (including `id`
  and `token_prefix`) plus the token, once. Rotation invalidates the old
  token at once.
- **Admin policy.** `PUT /v1/admin/organizations/{org_id}/policy`, system
  admin only, replaces the whole policy (`credentials.enabled`,
  `credits_transfer.enabled`, `max_sends_per_minute`, `constraints`, and a
  top-level `reason`). With `allowed_source_types` omitted, only `payment`
  and `staking_farm` are allowed. **A grant-funded sender must list
  `grant`.**
- **Read.** `GET .../credit/transfer/capacity`, `GET .../credit/transfers`
  (`direction`, `idempotency_key`, `limit` 1-100, cursor
  `before_time`+`before_id`; newest first; **no `payout_program` filter**)
  and `GET .../credit/transfers/{id}`. All need `credits:read`.
- **Recipient.** Any existing, active org. No opt-in. Credit arrives as
  `transfer` credit, cannot be re-sent, and pays any recipient overage first.
- **No reversal.** Nothing in #1271 takes a transfer back.
- **Finding a user's org.** `GET /v1/users/me` with the user's own session
  returns `organizations[]` with `id`, `name`, `role` and `is_active`,
  sorted by join time and capped at 100 (`crates/api/src/routes/users.rs:172-235`).
  There is no "personal" flag and no server-to-server lookup by user. Org
  names can carry part of an email address and are treated as personal data
  here; hence D1.

## 1. Goal and non-goals

**Goal.** A contributor account that has linked a NEAR AI organization
receives NEAR AI inference credit for the Trace Credit it settles, as soon
as it settles. The commons sends from its own reseller org with a scoped
credential. Every send is exactly-once, budget-capped and reconcilable
against #1271's transfer history.

**Funding model** (owner, 2026-10-19). The commons runs one reseller org.
NEAR AI funds it first with an admin grant. Later the commons buys payment
credits. USD only. The reseller sends; contributors receive `transfer`
credits in the org they chose.

**Non-goals.**

- Changing Trace Credit. A payout leaves the ledger, the batches and the
  NEP-141 rail exactly as they are (D6).
- Clawback. #1271 has no reversal. Over-payment is netted against the same
  account's later credit (section 4).
- Attributing what a recipient spends to the commons. #1271 defers it.
- Paying credit that settled to a bare principal with no account
  (section 6).

## 2. Staging setup (operator steps)

The owner does these once on staging, then again for production. Org ids,
tokens and the rate never go in the repository, in a PR, or in a log.

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
   Leave `allowed_recipient_ids` out at org level for open enrollment.
3. **Grant.** NEAR AI posts the grant to the reseller org. The sender must
   have an `organization_balance` row with zero legacy and unfunded amounts,
   or every send fails 409 `sender_not_reconciled`.
4. **Deployment check.** #1271's rollout note (`docs/credit-allocation.md:408-433`)
   needs a five-type `CREDIT_USAGE_ORDER` (including `transfer`), pinned with
   the allocation policy version, in the same revision as the transfer-aware
   image. Which order and version staging runs is not visible from the repo.
   Ask NEAR AI.
5. **Mint the payout credential** with an owner session:
   `POST /v1/organizations/{org_id}/credentials` with a `name` (a plain
   label such as "commons payouts, staging", nothing about a person),
   `permissions: ["credits:read", "credits:transfer"]`, `expires_at` at most
   90 days out on staging, and `constraints["credits:transfer"]` with
   `max_amount_per_transfer`, `max_total_amount` (below the org's),
   `allowed_source_types: ["grant"]` and, on staging,
   `allowed_recipient_ids` set to the test recipient orgs. Both permissions
   are needed: `credits:transfer` alone cannot read capacity or history.
6. **Hold the secret.** The token is shown once. Put it straight into the
   host's secret store. Ingest reads it from env only:
   - `TRACE_COMMONS_NEAR_AI_PAYOUT_CREDENTIAL` (held as `SecretString`,
     refused if empty or not `oc_` plus 64 hex);
   - `TRACE_COMMONS_NEAR_AI_PAYOUT_CREDENTIAL_ID` (the credential `id`; not
     secret, used only for its log hash);
   - `TRACE_COMMONS_NEAR_AI_PAYOUT_SENDER_ORG` (the reseller org UUID);
   - `TRACE_COMMONS_NEAR_AI_PAYOUT_SOURCE` (`grant` or `payment`);
   - `TRACE_COMMONS_NEAR_AI_PAYOUT_MODE` (`disabled`, `dry_run` or `live`;
     section 4, "Modes and the kill switch");
   - `TRACE_COMMONS_NEAR_AI_PAYOUT_ACCOUNT_CEILING_NANO` (section 4,
     "Ceilings").

   There is **no base-URL setting.** The payout host is the same compiled-in
   NEAR AI API base that login introspection uses (`NEAR_AI_API_BASE_URL`),
   switched only by the `near-ai-staging` build feature (section 8). The host
   that verified an org is the host that pays it.

   These names are proposals; none exists yet. Follow
   `docs/operator/commons-credential-storage.md` and
   `docs/operator/key-rotation.md`. Logs carry only `sha256:` of the
   credential `id` (not the `token_prefix`, which has 32 bits of entropy).
7. **Rate.** Nothing new. The operator's installed settlement table
   (`TRACE_COMMONS_CREDIT_SETTLEMENT_TABLE_PATH`) carries the rate (D2).
   Without a positive `usd_micros_per_credit` nothing accrues and nothing is
   sent.
8. **Payout program.** Set nothing in cloud-api; programs are just names. Our
   side derives them (section 4).

## 3. Placement

Payout is a separate pass beside `process_payouts`, sending in cumulative
mode. `trace_credit` keeps settling internally, unchanged. The pass reads
finalized settlement batches; it writes only its own tables.

A per-leg `SettlementAdapter` in amount mode was considered and not chosen.
A leg is per trace and per run, not per account; it carries no recipient;
and the withdrawal guard at `versioned_pipeline.rs:15139` would need a re-check first. A per-leg
cumulative adapter does not compose at all: a retry after a lost response
returns `no_op` with no transfer id, and two legs of one account settling
close together leave the lower target with no receipt of its own. The pass
gets "as quickly as possible" (D4) without any of that, because it is
triggered by each settlement (section 7).

Its trait is `NearAiCreditPayout`, not `NearPayoutAdapter`: that one is
on-chain, its `payout_eligible` is wired to rail `near`, and its outbox is
the NEP-141 rail's.

## 4. The transfer client and the payout pass

### Crate and licence

All of it lives in `crates/trace-commons-server` (AGPL): a client module
`near_ai_transfer.rs`, the pass, and storage in `db/`. The link request and
response types go in `crates/trace-commons-protocol` (permissive), beside
`inference_connection.rs`, because the daemon sends them. No permissive
crate gains a server or gate dependency. `tests/license_boundary.rs` must
stay unchanged.

### Client

`NearAiTransferClient` holds the sender org, source type and the credential
as `SecretString`. Its base URL is not a field anyone sets: it is the
compiled-in NEAR AI API base. `NEAR_AI_API_BASE_URL` moves from the ingest
bin into `trace-commons-server` so introspection and this client read one
constant (a test-only constructor may take a loopback mock URL). It copies
the scorer client's shape: TLS or loopback only, `redirect(Policy::none())`,
a fixed user agent, a 15 s timeout (cloud-api bounds the send transaction at
10 s, `credit_transfers.rs:51-53`), `bearer_auth`. It exposes three calls:
`send_cumulative`, `capacity` and `history_page`. It parses the error body's
`error.type` into a closed enum and keeps nothing else from an error.

### Recipients are per account

The unit of payment is a credit account, `(tenant_id, account_id)`, never a
tenant. A tenant can hold many accounts; each links its own org, or none.
Only the account itself can link, change or unlink (section 5), and only
its own credit goes to its own link.

### Payout program and link generation

The cumulative total is scoped to sender + recipient + program. If one
program name served every link, an account that moved from org X to org Y
would get a fresh total at Y, and a cumulative target would pay all its
history again. So:

- Each link has a **generation id** (`link_id`, UUID) and its own program:
  `tc1-` plus the first 32 hex of `sha256("nearai-payout-program-v1:" || link_id)`.
  It is opaque to the recipient and fits in 100 bytes.
- When a link closes, the pass first resolves it (section 5, "Change and
  unlink") and records `final_paid_nano` from #1271.
- The target for the account's live link is
  `owed_nano(account) - sum(final_paid_nano of the account's closed links)`.
- Relinking to an org used before still gets a new generation, so a new
  program. Nothing collides.

### Rate

D2: the rate is the installed settlement table's `usd_micros_per_credit`.

- **Epochs follow table versions.** At boot, after the table validates,
  ingest records the table's `(version, pricing_version,
  atomic_units_per_credit, usd_micros_per_credit)` in an append-only
  `trace_nearai_payout_rate_epochs` table with `observed_at` set by the
  database clock, if that `version` is not already there. If the `version`
  is there with a different rate or units, ingest refuses to start with
  `payout_rate_version_conflict`: a table version names one rate forever.
  Epochs are ordered by `observed_at`, which no one can set, so none can be
  backdated.
- **Which epoch prices a batch.** A finalized batch is priced at the epoch
  in force at the batch's `created_at` (the latest epoch observed at or
  before it). A batch older than the first epoch is priced at the first
  epoch. This is a pure function of stored rows, so it gives the same
  answer whenever accrual happens (at settlement, or later when an account
  first links).
- **No rate, no payout.** A table without a positive `usd_micros_per_credit`
  records no epoch. With no epoch at all, nothing accrues and the pass
  sends nothing. Nothing falls back to a default.
- **The number stays out of the repo.** Tests use synthetic rates, as the
  settlement-estimate tests do. The rate is not logged; logs carry the epoch
  `version` label only.

### Accrual

A new table, `trace_nearai_payout_accruals`, holds one row per settled
account line, priced once:

- Key `(tenant_id, settlement_batch_id, credit_account_hash)` for a credit
  row, and `(tenant_id, reversal_event_id)` for a reversal row (below).
- Columns: `account_id`, `kind` (`credit` or `reversal`),
  `micros NUMERIC(39,0)` (signed), `rate_version`, `nano NUMERIC(39,0)`
  (signed), `created_at`.
- The pass inserts with `ON CONFLICT DO NOTHING`, so re-running accrual is
  harmless and a row is never re-priced. `nano =
  floor(micros * usd_micros_per_credit * 1000 / atomic_units_per_credit)`,
  in `u128`, checked; a reversal row uses the same formula on its absolute
  value and is negated, so it takes back exactly what its credit added.
- **Which lines accrue.** Every line of every `finalized` batch whose
  `credit_account_ref` names an account (`settlement_key_account_id` is
  `Some`) and whose `settled_credit_delta_micros` is positive. A line keyed
  to a bare principal accrues nothing (section 6). `dry_run` and `failed`
  batches never accrue.
- **NEAR wallet holds do not block this rail.** `near_payout_hold_reason`
  says the account has no NEAR wallet to pay. This rail does not pay a
  wallet, so the label is not a reason to withhold NEAR AI credit; the
  line accrues to its own account, and to no one else. What does hold this
  rail is in "Holds" below.
- **Owed** for an account is `sum(nano)` over its accrual rows. It is never
  read from `trace_credit_ledger` sums or `account_credit_totals` (a display
  path that sums `f32`).

### Reversals

- A reversal row (`settlement_state = 'reversed'`, negative `points_delta`)
  names the event it reverses in its `external_ref`. The pass finds the finalized
  batch line whose `source_credit_event_ids` holds that event, and writes a
  `reversal` accrual for the same account at that line's `rate_version`,
  for the reversed event's own amount.
- If the reversed event was never in a finalized batch, nothing was accrued
  and nothing is written. If the line cannot be found for a reversal of a
  settled event, the account is held as `payout_reversal_unpriced`.
- **Netting, not clawback.** If owed drops below what the live link's
  program has already received, the target is below the program total, so
  the pass sends nothing. The account's later credit fills the gap before
  anything more is sent. If the account never earns again, the difference
  is a loss to the commons. Nothing is taken back unless #1271 adds a
  reversal, and that would need its own design.
- Only a reversal row lowers owed. Per the roadmap (l.36-38) that is an
  operator-raised correction, not an automatic result of a revocation or a
  withdrawal, which leave settled Trace Credit, and so owed, as they are.

### Holds

A held account accrues as usual and gets no send until the hold clears.
Holds are per account and never move credit to another account.

- `payout_account_credit_hold`: an open `trace_credit_holds` row
  (`released_at IS NULL`) for any principal linked to the account in
  `trace_account_principals`, or for its `account:` settlement key.
  Settlement already keeps a held principal's new events out of batches;
  this check stops sends of credit that settled before the hold.
- `payout_account_closed`: `trace_accounts.closed_at` is set.
- `payout_reversal_unpriced`, `payout_link_unreadable`,
  `payout_amount_out_of_range`, `payout_over_ceiling`: below.

### Ceilings

There is no pulled statement for anyone to fetch; the commons only pushes.
Every amount that leaves is bounded three times, and each bound is checked
before the line is written `pending`:

- **Per send:** the target is at most `program_total + max_amount_per_transfer`
  (read from `capacity`), so back pay steps up over successive passes.
- **Per account, lifetime:** a target above
  `TRACE_COMMONS_NEAR_AI_PAYOUT_ACCOUNT_CEILING_NANO` writes no line and
  holds the account as `payout_over_ceiling`. An operator lifts it by
  raising the setting; nothing is sent above it in the meantime.
- **Whole rail, lifetime:** the credential's `max_total_amount`, enforced by
  cloud-api under its own lock, and our `capacity` probe before the pass.

`u128` arithmetic, checked; a result above `i64::MAX` holds the account as
`payout_amount_out_of_range`.

### Modes and the kill switch

`TRACE_COMMONS_NEAR_AI_PAYOUT_MODE`:

- `disabled`: no accrual, no pass, no link probe. The link route answers
  `payout_unavailable`.
- `dry_run`: accrual runs (it moves no money and makes later live sends
  match). The pass computes each target and records a count-only report
  (accounts that would be sent to, total nano, holds by label). It makes
  **no POST to #1271 at all**, not even a zero read; it may call `capacity`.
  No line is written.
- `live`: as specified below.

The kill switch is runtime, not boot-time: an admin route
`POST /v1/admin/nearai-payout/halt` sets a row in
`trace_nearai_payout_control` (with an audit row: actor hash, time, label),
and `.../resume` clears it. The pass reads it before each send, under the
single-flight lock, and stops at once when it is set. Lines already
`pending` stay `pending`; recovery resumes them after `resume`. Readiness
reports `payout_halted`.

### State and storage

New tables, RLS forced, tenant predicates through
`trace_current_tenant_id()`: the accruals above, and

- `trace_nearai_payout_links`: `tenant_id`, `account_id`, `link_id`, `state`
  (`waiting`, `active`, `closing`, `closed`), `sealed_org_id JSONB`,
  `org_index` (keyed blind index), `org_key_ref`, `index_pepper_ref`,
  `subject_index` (keyed blind index of the NEAR AI subject that proved
  the org, section 5), `payout_program`, `final_paid_nano NUMERIC(20,0)`
  (set on close), `unlink_requested_at`, `version`, timestamps. Three
  partial unique indexes allow at most one `active`, one `waiting` and one
  `closing` row per `(tenant_id, account_id)`. There is no uniqueness on
  `org_index`: many accounts may name one org (D7), so no org can be locked
  away from its real owner by an earlier link.
- `trace_nearai_payout_lines`: `tenant_id`, `account_id`, `line_id` (UUID),
  `link_id`, `target_nano`, `owed_through_accrual_count`, `state`
  (`pending`, `confirmed`, `no_op`, `held`, `failed`), `transfer_id_hash`,
  `delta_nano`, `attempts`, `last_error_label`, timestamps. A partial unique
  index on `(link_id) WHERE state = 'pending'` (a link has at most one
  unresolved line), and one on `(transfer_id_hash) WHERE transfer_id_hash IS
  NOT NULL` (one transfer answers one line).
- `trace_nearai_payout_audit`: append-only, hash-only. One row per link
  request, confirmation, change, unlink, close, operator release, hold set
  or cleared, halt and resume: action, actor ref hash, link id hash, prior
  and new state, label, time. Nothing is updated in place without a row
  here.
- `trace_nearai_payout_rate_epochs` and `trace_nearai_payout_control` are
  not tenant data. They get forced RLS with a read-all policy, and writes
  only through a narrow role used by the boot path and the admin route.
- **Enumeration.** The pass and the drill need the tenants with an
  `active` or `closing` link, which no tenant context can see under forced
  RLS. A narrow role, `trace_nearai_payout_enumerator`, with a permissive
  `SELECT` policy on `trace_nearai_payout_links (tenant_id, state)` only,
  assumed with `SET ROLE` for that one query, as
  `docs/operator/register-stats-role.md` does. It can read nothing else.
  Every per-tenant read and write after that runs under that tenant's
  `trace_current_tenant_id()`.
- **Migration number:** the next free one at plan time. V129 was reserved by
  #1415, which is closed; check the repo's numbering convention before
  reusing it.

### One pass

0. **Single flight.** The pass takes a session-level advisory lock on a
   fixed key (`pg_try_advisory_lock(hashtextextended('nearai-payout-pass-v1',
   0))`, the hashing at `versioned_pipeline.rs:2177`) on a connection it
   pins for the whole run, and releases it with `pg_advisory_unlock` on
   every exit path; a crash drops the connection and the lock. If the lock
   is held, the trigger sets a "run again" flag and returns (section 7).
   Every link state change (`active` to `closing`, `closing` to `closed`,
   `waiting` to `active`) is made by the pass while holding it. The routes
   only record requests.
1. Enumerate tenants (above). Per tenant, under its context:
   a. **Accrue** every finalized batch line and reversal not yet accrued.
   b. **Apply requests**: move a live link with `unlink_requested_at`, or
      with a confirmed `waiting` successor, to `closing`, and resolve it
      (section 5).
   c. For each account with an `active` link: skip it if it is held, or if
      its link has a `pending` line (that line goes to crash recovery
      first). Read the link's program total with a `recipient_cumulative: 0`
      send. Compute the target (owed minus closed links' final totals,
      capped per send). Skip unless the target exceeds the program total.
      Check the account ceiling.
2. Write the line `pending` with its `target_nano`, committed, before any
   network call.
3. Probe `capacity` once per pass. If `sending_enabled`,
   `sender_reconciled` or `credential_can_transfer` is false, stop the pass
   with that label. Pace sends to `sends_remaining_in_window` and to a local
   limit below `max_sends_per_minute`.
4. Re-read the kill switch. Send `recipient_cumulative = target_nano` with
   the link's program. Memo empty. Metadata
   `{"tc": "<sha256 of 'nearai-payout-line-v1:' || line_id>"}`.
5. Record the outcome:
   - 201: `confirmed`, `transfer_id_hash = sha256("nearai-transfer-v1:" || id)`,
     `delta_nano = amount`. If the returned program total is not the
     target, hold as `payout_program_total_mismatch`.
   - 200 `no_op`: if `recipient_cumulative >= target`, run recovery (below);
     otherwise hold as `payout_program_total_mismatch`.

### Crash recovery

A crash between step 2 and step 5 leaves a `pending` line whose send may or
may not have committed. Because a link has at most one pending line and the
pass is single-flight, nothing else was written or sent for that link in
between. The next pass re-sends the **same** target first:

- If the first send never committed, it is sent now (201).
- If it did, the reply is `no_op`. The pass pages
  `GET .../credit/transfers?direction=outgoing` and filters client-side for
  this recipient, this program and `recipient_cumulative == target_nano`,
  skipping any transfer whose id hash is already on another line. History
  has no program filter, and the cursor is still returned on the last page,
  so the pass stops on a short page.
- If no row matches, hold the line as `payout_recovery_unmatched`. Under the
  one-pending-line rule this cannot happen without a bug or an outside send
  on our program.

### Error mapping

| #1271 reply | Line state | Notes |
|---|---|---|
| 429 `transfer_rate_limited`, 503 `transfer_unavailable` / `accounting_busy`, timeout | stays `pending` | retried next pass, same target |
| 409 `insufficient_posted_capacity`, `sender_not_reconciled` | pass stops, `held` | operator: fund or reconcile the sender |
| 409 `credential_transfer_limit_exceeded`, `organization_transfer_limit_exceeded` | `held` | budget or recipient restriction; compare with `capacity` |
| 409 `capacity_overflow` | `held` | the recipient's incoming total would overflow `i64`; operator review |
| 404 `organization_unavailable` | `held`, link marked `recipient_unavailable` | contributor must relink |
| 403 `credential_refused` | pass stops, fail closed | credential malformed, expired, revoked or wrong |
| 403 `sender_not_enabled` | pass stops, fail closed | no policy row, or transfers disabled; ask NEAR AI |
| 400 any | `failed` | a bug on our side; page the operator |

A 403 or 401 on **any** call, read or send, stops the pass and changes no
row: it is a fault in our credential, not in an account. No account or link
is held, frozen or closed because of it.

### Fail-closed rules

- Mode `dry_run` or `live` with any of the payout env values missing or
  malformed: ingest refuses to start with `near_ai_payout_config_missing`.
- The payout host is compiled in. A `near-ai-staging` build switches it for
  introspection and payout together, is refused as a release build, and
  reports itself in readiness. No env value names a host.
- Readiness reports the rail as `near_ai` with `mode`, `halted`,
  `rate_epoch` (the version label only) and `qualified`. `qualified` is true
  only in `live`, outside `near-ai-staging`, with a rate epoch in force and
  a startup `capacity` probe answering `sending_enabled: true`.
- A link whose sealed org id fails to open: hold `payout_link_unreadable`.

### Hash-only logging

Logs and audit rows carry labels and `sha256:` values only: line, link,
transfer, program and credential id hashes, amounts in nano-USD, and the
rate epoch's version label. Never the recipient or sender org UUID, the org
name, the token, the memo, the metadata, the tenant or account id, or the
rate. This follows `docs/operator/hash-only-logging.md`.

## 5. Linking an account to a NEAR AI organization

### Who can link

Any account whose person can sign in to NEAR AI in the app and owns the
org they choose (D3). The proof is a fresh NEAR AI access token whose
`/users/me` lists the org as active and owned. The authority to direct the
account's credit is a passkey step-up on that account (D5).

| Account class | NEAR AI subject on file | Check at link time |
|---|---|---|
| `nearai-` tenant | yes (the tenant anchor) | `/users/me` `id` must match the anchor, else `subject_mismatch` |
| `near-` tenant (wallet) | no | the person signs in with NEAR AI in the app; the subject is recorded as `subject_index` on the link |
| passkey-origin account, bound or not | per its binding, or none | as the two rows above |
| account in a legacy or pooled tenant | no | as the wallet row |

Recording `subject_index` lets an operator see that one NEAR AI subject
linked several accounts, without opening anything. It is not an anchor and
grants nothing.

### Verification on the server, not the client

1. The daemon exchanges its refresh token for a fresh access token
   (`session::exchange`). A wallet-only account first signs in with NEAR AI
   in the app.
2. It calls `/v1/users/me` itself and shows the active orgs where the user
   is owner, by name (D1). Nothing is preselected when there is more than
   one; with exactly one it still needs a confirm. The name stays on the
   device.
3. On confirm, the daemon starts a **link request**: it sends the chosen org
   id, the fresh NEAR AI access token, an `idempotency_key` and
   `expected_current_version` to ingest over its own (weak) session.
4. Ingest calls `GET /v1/users/me` once with that token at the compiled-in
   base, then drops the token. It reads the response into a new
   single-purpose struct holding `id` and `organizations[]{id, role,
   is_active}` only; it does not deserialize `name`. It applies the subject
   check from the table above, and checks that the org is in the list,
   active, with `role == "owner"`.
5. Ingest sends a liveness probe: `recipient_cumulative: 0` on the fixed
   program `tcprobe-v1` (outside the `tc1-` namespace, so the drill never
   counts it; a zero target never writes a row). 200 `no_op` passes. 404
   refuses as `near_ai_org_unavailable`. 409
   `credential_transfer_limit_exceeded` refuses as
   `near_ai_org_not_permitted` (expected on a closed pilot list). 403
   refuses as `payout_unavailable`.
6. Ingest stores a `pending_confirmation` request: sealed org id, subject
   index, the account it belongs to, a 5-minute expiry, and an opaque
   ceremony handle returned to the daemon. **Nothing changes yet.**
7. The daemon opens the step-up page with `return=confirm-payout-org` (a
   fourth section). The person signs in **with a passkey**; the page shows
   "the organization you chose in the app" (the server does not hold the
   name) and a confirm button. Confirming writes the `waiting` link (or the
   unlink request) and the audit row. Expiry, a second request, or a
   different account discards it.

### The passkey gate

A new gate, `require_passkey_step_up`, guards the confirm call. It passes
only a session whose `client_kind` is `passkey`, minted within the step-up
window, on the account that started the request. It has no
zero-authenticator carve-out: an account with no passkey gets
`passkey_required`, and the app sends the person to the step-up page's
`add-passkey` section first. A NEAR wallet (`near`) session does not pass:
D5 says passkey. `require_authenticator_change_allowed` is not changed; the
existing NEAR payout redirect keeps its rules.

### The routes

`POST /v1/account/near-ai/payout-org/requests` (start, step 3), copying the
inference-connection stack
(`crates/trace-commons-contributor/src/inference_connection.rs:190-230`,
`trace_commons_ingest_internal/inference_connection.rs`):

- account session only; `DeviceBearer` is refused with 403;
- body `deny_unknown_fields`: `action` (`link` or `unlink`),
  `organization_id` and `near_ai_access_token` (link only),
  `idempotency_key` (non-nil UUID), `expected_current_version`;
- outcomes: `pending_confirmation` (with the handle), `idempotency_conflict`,
  `version_conflict`, `subject_mismatch`, `org_not_owned`,
  `near_ai_org_unavailable`, `near_ai_org_not_permitted`,
  `payout_unavailable`, `passkey_required`.

`POST /v1/account/near-ai/payout-org/requests/{handle}/confirm`, behind
`require_passkey_step_up`, called only by the step-up page. Outcomes:
`confirmed`, `expired`, `not_found`.

`GET /v1/account/near-ai/payout-org` returns `{linked, org_ref, state,
version, request_state}`, never the id or a name. `org_ref` is
`sha256("nearai-payout-org-ref-v1:" || idempotency_key || ":" || org_id)`;
the daemon keeps the key and the org id together, so it can recompute the
ref and check the server holds the same org without the server echoing it.

### Storage

The send needs the raw UUID, so hash-only storage is impossible. The org id
is sealed with the machinery behind `sealed_account_name`, with a keyed
blind index and key and pepper refs for rotation. The key is not in the
database.

### Change and unlink

- **Change** confirms a new generation as `waiting`. The next pass, under
  its lock, moves the old link to `closing` first.
- **Closing.** The pass sends nothing new to a closing link and never
  re-sends to it. It resolves every `pending` line on it through reads
  only: a `recipient_cumulative: 0` read of the program total and, for the
  transfer id, history paging. A line whose target the total reached is
  `confirmed`; one it did not reach is `no_op`. Only then does it write
  `final_paid_nano` (the read total), set the old link `closed` and the
  waiting one `active`, in one transaction, with an audit row.
- **Old org gone.** If the zero read returns 404, the pass takes the program
  total from history alone. If history cannot be read either, the link
  stays `closing`, readiness shows `payout_link_close_blocked`, and the
  pass retries.
- **Unlink** confirms `unlink_requested_at` on the live link; the pass
  closes it with no successor. Credit keeps accruing and is sent when the
  account links again.
- **Operator release.** `POST /v1/admin/nearai-payout/links/{link_id_hash}/release`
  moves a link to `closing` (for example after a contributor reports lost
  access to the org) with an audit row naming the actor hash and a reason
  label. The account then links again through the passkey flow.

### Credit earned before linking

Nothing extra. Accrual covers every settled line of the account whether or
not it was linked, priced at each batch's own epoch. The first target is
everything owed minus what prior links received, capped per send, so back
pay steps up over a few passes.

### Consent copy (draft, for owner review)

New keys in `crates/trace-commons-contributor/src/consent_copy.rs`, one each
for title, body, button and accessibility name, per `docs/ui-copy-guide.md`.
**All text below is draft and needs owner review.**

- Title: "Get paid in NEAR AI credit"
- Body: "The commons will send NEAR AI credit to {org name} for traces you
  contribute, as soon as their credit settles. This is on top of your Trace
  Credit, which you keep. Credit sent to {org name} stays there and can't be
  moved. Other members of {org name} can spend it too. If {org name} owes
  NEAR AI for past use, new credit pays that first. You'll confirm with your
  passkey."
- Scope line beside the button: "Sends credit to {org name} only."
- Button: "Continue with passkey"
- Secondary: "Not now"
- No passkey yet: "Add a passkey to choose where NEAR AI credit goes."
- Unlink button: "Stop sending credit"; body: "Credit you earn is kept and
  sent when you choose an organization again. You'll confirm with your
  passkey."
- Status: "Credit goes to {org name}" / "No organization chosen" /
  "Waiting for passkey confirmation" / "Organization unavailable. Choose
  another."

The confirm step names the org shown, and a `connection_revision`-style
check (as `near_ai_funding` does) refuses a click if the org changed since
it was displayed. macOS is the lead shell; the others follow it.

## 6. Eligibility

D3, per account:

- **Eligible:** any open account (`trace_accounts.closed_at IS NULL`) that
  has completed the link in section 5. Tenant class does not matter.
- **What is paid:** only credit that settled on a line keyed
  `account:{account_id}`. The NEAR wallet hold label on a line does not
  matter (section 4, "Accrual"); a credit hold does (section 4, "Holds").
- **Pooled tenants** (`trace_legacy_invite_pooled_tenants`, V81). Each
  account in a pooled tenant links and is paid for its own lines only. No
  account is paid for another's credit, and nothing is paid at tenant level.
  Credit that settled to a bare principal (a member who had no account when
  the batch was written) has no owner and is not paid. Batch lines are
  never re-keyed, so linking that principal to an account later does not
  make old lines payable.
- **Legacy `tenant-...` tenants:** the same as pooled. The rule that a
  legacy tenant must first link to a NEAR account (V81) does not apply to
  this rail; the NEAR AI proof replaces it.
- **Wallet (`near-`) tenants:** eligible after the person signs in with
  NEAR AI in the app.

The predicate lives in one server function, checked at link time and on
every send.

## 7. Schedule

D4: as soon as credit settles.

- **Trigger.** Every code path that commits a finalized settlement batch
  (the pipeline's `finalize_pending_credit_batch` and the ingest settlement
  route) notifies the payout worker after its transaction commits (an
  in-process `tokio::sync::Notify`; no payload, no tenant id). The worker
  runs a pass. A notify while a pass runs sets the "run again" flag, so a
  burst of settlements becomes at most one extra pass.
- **Retry tick.** The worker also runs every 60 s (proposed). This catches a
  notify lost to a crash, retries `pending` lines, and finishes closing
  links. A pass with nothing to do costs one enumeration query.
- **Coalescing.** One send per account per pass covers everything accrued
  since the last one, so the 60/min quota is per account-pass, not per
  trace.
- **No minimum send.** Owner asked for speed. A floor can be added later
  without loss, since cumulative mode carries the remainder.

At 60 sends a minute a pass serves 3,600 accounts an hour, above the pilot.
The quota can be raised by policy.

## 8. Staging test plan and reconciliation drill

### Known staging gap

The daemon's cloud-api origin and the server's NEAR AI base are compiled in
as production. Plan: a build-time feature (`near-ai-staging`) that switches
both, refused in a release build and reported in readiness. Until it
exists, test the client against a mock #1271 server, and test the link flow
against production with a closed recipient list.

### Steps

1. Section 2 setup with `allowed_recipient_ids` set to two test orgs.
2. Unit: rate-epoch selection by batch time, the version-conflict refusal,
   pricing and floor, reversal priced at its line's epoch, `u128` to `i64`
   bounds, the three ceilings, error mapping, program derivation, target
   after a closed link.
3. Mock #1271 server: 201; `no_op` then history recovery; lost response;
   429 pacing; each 409 hold; both 403 causes stop the pass and change no
   row; a 403 on a read changes no row; a second pass refused while the
   first holds the lock; a notify during a pass causes exactly one more; a
   link with a pending line gets no new line; closing resolves by reads
   only; closing after a 404 completes from history; `dry_run` makes no
   POST; the kill switch stops the pass before the next send.
4. PostgreSQL: two accounts in one tenant, one linked and one not; the
   unlinked one's credit is accrued but sent nowhere. A credit hold on one
   account stops its sends and not the other's. A line keyed to a bare
   principal accrues nothing. RLS on every new table.
5. Staging, live: two accounts, one settlement each. Expect one send per
   account within a pass of settlement.
6. Kill ingest after step 2 of a pass and before step 5. Re-run. Expect one
   transfer, not two.
7. Change org for one account. Expect the closing link settled, then the new
   program starting at owed minus paid.
8. Operator reversal after a payout. Expect no send until new credit
   exceeds the gap.
9. Exhaust `max_total_amount`; revoke the credential; deactivate a recipient
   org; deactivate an org while its link is `closing`. Expect a hold, a
   stopped pass, `recipient_unavailable`, and a completed close.

### Reconciliation drill

`POST /v1/admin/nearai-payout-reconcile-drill`, following
`docs/operator/drills.md`: admin bearer, `ready` plus label-only
`blocking_gaps`, hash-only evidence, wired into `rollout-smoke/evidence`.
It is read-only: it never holds, freezes or changes a row. It pages
outgoing history and compares, per link: the sum of our confirmed
`delta_nano` against the program total; every confirmed `transfer_id_hash`
against a history row; and no history row in a `tc1-` program that we do
not hold. Gaps: `payout_history_unmatched`, `payout_program_total_mismatch`,
`payout_line_missing_transfer`. It also reports, as counts, accounts whose
owed is below their paid total (netting in progress). cloud-api's admin
reconciliation route is system admin only; NEAR AI can run it.

## 9. Threat model

- **Stolen credential.** Bounded by `max_amount_per_transfer`,
  `max_total_amount` (lifetime, survives rotation), expiry (90 days
  proposed), and source restriction. For open enrollment the caps and
  expiry are the defence. Response: halt, revoke in the UI, rotate, update
  the env, restart, resume.
- **Stolen database.** Org ids are sealed; the key and the credential are
  not in the database.
- **Double pay.** Cumulative targets converge, so retries and re-runs cannot
  overpay one link. Per-link programs stop a relink from paying history
  twice. The pass is single-flight, a link has at most one pending line,
  and a link closes only after its pending lines are resolved by reads.
  Accruals are unique per batch line and per reversal, so a batch is priced
  once.
- **Paid, then reversed (D4).** With no maturation window, credit can be
  sent and later reversed by an operator. The NEAR AI credit cannot be
  recovered. The account's later credit is netted first; an account that
  never earns again leaves the commons with the loss. Upload, settle,
  cash out and leave is therefore possible for anything that passes the
  gates and is only later found to be bad. Limits on that exposure: the
  account ceiling, the credential's lifetime cap, and the existing gates
  before settlement. This is the owner's explicit choice.
- **Paid twice for one trace (D6).** The same settled credit can be paid as
  NEAR AI credit and also as NEP-141 Trace Credit (or redeemed later). That
  is the owner's design, not a race: each rail keys its own exactly-once
  record off the same finalized line, and neither reads the other's.
- **Wrong recipient.** The org must be in the person's own `/users/me`,
  active and owned, checked by the server with the person's own token
  against the host that pays. Only a passkey session on the same account
  can confirm. A wrong send cannot be reversed.
- **Stolen device.** The daemon's refresh token and `tcn1_` token can start
  a link request, but cannot confirm it without the account's passkey.
- **Role lost after linking.** The server cannot re-check org ownership
  without the person's token, so a link stays valid if the person later
  loses the owner role. Credit then goes to an org they no longer own. The
  person can unlink; the operator can release. Because `org_index` is not
  unique, the org's real owners can always link it themselves.
- **Shared org (D7).** Credit sent to an org with other members is
  spendable by all of them. The consent copy says so.
- **Recipient debt.** Incoming transfer credit first pays any overage the
  recipient org owes. The consent copy says so.
- **Sender out of funds.** 409 `insufficient_posted_capacity` stops the pass.
  Accruals stay owed. Readiness reports it.
- **Rounding.** Floor per batch line; at most one nano-USD per line.
- **Metadata leak.** Memo empty, metadata an opaque hash.
- **Org name.** The server never deserializes, stores or logs it.
- **Rate.** Not in the repo, not in logs; only the epoch version label.
  The settlement table is served to clients, so the rate is visible to
  them, as S2 already accepts.
- **Enumeration role.** Reads only `tenant_id` and `state` of the link
  table. A leaked connection under it reveals which tenants have a linked
  account, nothing about whom to.

## 10. Delivery plan

Small stacked PRs. Each passes the repo checks on its own.

1. **Protocol types** for link request, confirm and status (permissive).
2. **Migration** for links, lines, accruals, audit, rate epochs and
   control, with RLS tests.
3. **Rate epochs** at boot from the installed table, with the conflict
   refusal.
4. **Transfer client** with the mock #1271 server and the error table.
5. **Accrual** from finalized batches and reversals, per account.
6. **Link request and confirm routes**, the passkey gate, the server-side
   `/users/me` check, the liveness probe, audit, and the step-up page's
   fourth section.
7. **Daemon and macOS** link flow and copy keys (draft copy behind a
   setting until the owner signs it off).
8. **Payout pass** behind `dry_run`, with crash points, the single-flight
   lock, the enumeration role, holds, ceilings and the kill switch; then
   the settlement notify and the retry tick.
9. **Reconciliation drill** and readiness wiring.
10. **Staging feature** for the pinned hosts, then the staging run.
11. **Operator runbook** under `docs/operator/`, listed in its README.

## 11. Lessons from #1415

The review of #1415 (2026-10-20) found fourteen problems in the earlier
settlement-rail draft. Each one, and where this design handles it:

1. **Whole batch paid to one tenant-level org; payout holds skipped.**
   Recipients, links and accruals are per account, `(tenant_id,
   account_id)`, and accrue per batch line from `line_items_json`
   (section 4, "Recipients are per account", "Accrual"). Only the account
   can link its own org (section 5). Holds that apply to this rail are
   checked per account (section 4, "Holds"). The NEP-141 wallet hold label
   is deliberately not one of them, because it is about the wallet target;
   it never sends a held account's credit to anyone else.
2. **One-payout-path triggers race (check then insert, no shared lock).**
   There is no cross-rail exclusivity to race on: D6 makes this rail
   parallel to Trace Credit, so it never reads or writes the NEP-141
   outbox, and there are no triggers. Inside this rail, exactly-once rests
   on unique keys written with `ON CONFLICT DO NOTHING` (accruals), the
   single-flight lock, one pending line per link, and cumulative targets
   (section 4). If the owner ever makes the rails exclusive, the chosen
   rail must be recorded on the batch line under `SELECT ... FOR UPDATE` of
   the batch row, as the review proposed, not by triggers.
3. **Pull mode serves totals above the ceiling or not yet checked.** No
   pull mode and no signed statement: the commons only pushes. Every bound
   is checked before a line is written `pending` (section 4, "Ceilings"),
   and cloud-api enforces the credential's caps under its own lock.
4. **No dry run in pull mode; kill switch does not reach serving.** Mode is
   `disabled`, `dry_run` or `live`; `dry_run` makes no POST at all. The
   kill switch is a runtime row read before every send, not a boot setting
   (section 4, "Modes and the kill switch").
5. **Award period ignores policy, so a policy change reprices the period.**
   There is no period sum. Each batch line is priced once, at the epoch of
   the table version in force at the batch's time, and stored; table
   versions cannot change their rate (section 4, "Rate", "Accrual").
6. **Reconcile can falsely freeze a ledger.** Reconciliation is a read-only
   drill that never freezes or changes a row (section 8). Crash recovery
   runs inside the single-flight pass, so no publish and reconcile overlap.
7. **A read-key 403 freezes every ledger.** A 401 or 403 on any call stops
   the pass and changes no row (section 4, "Error mapping").
8. **Accrual reads the recipient without a lock.** Routes only record
   requests; every link state change is made by the pass under its lock,
   and accrual is not tied to a recipient at all (it is per account, and
   the link only decides where the cumulative total goes) (section 4,
   "One pass").
9. **Award period per recipient, not per account.** Owed is per account
   across all its links: the target subtracts every closed link's final
   total (section 4, "Payout program and link generation"). Pricing is
   linear per line, so alternating orgs gains nothing.
10. **An org is checked once and then locked forever.** `org_index` is not
    unique, so no link can lock an org away from its owners. The account
    can unlink, and the operator has an audited release (section 5,
    "Change and unlink"). Re-checking the role before each send is not
    possible without the person's token; section 9 records the residual
    risk.
11. **Unfreeze and rebind leave no audit trail.** `trace_nearai_payout_audit`
    is append-only and gets a row for every request, confirmation, change,
    unlink, close, release, hold, halt and resume (section 4, "State and
    storage"). There is no unfreeze.
12. **HTTPS check bypassed with a userinfo URL.** There is no base-URL
    setting; the host is a compiled-in constant shared with login
    introspection (section 2, step 6).
13. **The "generic" ledger is hard-wired to NEAR AI.** This design claims no
    generality: its tables are NEAR AI's. A future rail gets its own
    tables, keyed off the same finalized batch lines.
14. **Stale branch and citations.** Citations here are against
    `origin/main` f164d68f9, with the ingest binary at its new path
    (`crates/trace-commons-ingest`). The migration number is picked at
    plan time (section 4, "State and storage").

## 12. Still open

None of these is an owner decision about the design; each is a value or a
fact to get before build.

1. **Account ceiling** value, **retry tick** interval, and **credential
   expiry**.
2. **Staging API host:** `cloud-stg-api.near.ai` or `cloud-stg.near.ai`.
3. **Staging deployment flags** (`CREDIT_ALLOCATION_POLICY_VERSION`,
   `CREDIT_USAGE_ORDER`): unknown; ask NEAR AI.
4. **History filter.** No `payout_program` filter; recovery and the drill
   page all outgoing history. Fine at pilot scale; ask NEAR AI for a filter
   before it is not.
5. **Restriction vs budget.** One error code covers both; we infer from
   `capacity`.
6. **Bare-principal credit** (section 6): unpaid by this design. If the
   owner wants it paid once the principal joins an account, that needs a
   re-keying rule for old batch lines.
7. **Spending attribution.** Deferred in #1271.
