# Parallel payout rails, starting with NEAR AI credit -- design

Date: 2026-10-21. Owner: Zaki. Status: draft, nothing built. The owner
settled every design decision on 2026-10-21 (below). What is still open,
and the defaults the owner may override, are listed in section 12. Section
11 maps each point of the review of the earlier settlement-rail draft
(#1415) to where this design handles it, per rail. The implementation plan
is [`../plans/2026-10-21-nearai-credit-payout.md`](../plans/2026-10-21-nearai-credit-payout.md).

The payout flow is a set of **parallel settlement rails**, each settled from
the same award (D8). Trace Credit is the award and the first rail. NEAR AI
credit is the first external rail and the only one this design builds.
Later rails (a NEP-141 Trace Credit token, BAT) plug into the same
interface (section 3) and are not designed here.

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

  **Superseded by the owner on 2026-10-23.** "We aren't using passkeys at
  all in the app right now." The owner chose **signed-in session only**:
  link, change and unlink are authorized by the contributor's normal
  authenticated app session (the account session the daemon already uses
  with the commons), with no passkey step-up, no step-up page section, and
  nothing that requires the account to have a passkey. The server still
  verifies org membership with NEAR AI (`/users/me`, using the
  contributor's NEAR AI access token, handed over for that check and never
  stored), keeps the org-changed-since-shown (`connection_revision`-style)
  check, and audits every link, change and unlink. The owner accepted the
  trade-off in the option's own words: "anyone holding the app session
  could redirect future payouts." Section 5's passkey gate and step-up
  page section were replaced when the link routes were built (task 6, see
  section 5, "Who confirms"); the passkey consent lines are struck there
  and their replacements are drafted with the macOS picker (task 8). Where
  any text still says passkey, this note governs.
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
- **(D8) Many rails, settled in parallel.** "our general model is that we
  should imagine a future of multiple settlement in the payout flow. Our
  current settlement is trace credit and near ai but there can and will be
  more." So the payout flow is N rails behind one interface (section 3).
  Each enabled rail pays its own instrument from the same finalized award,
  independently: a failure, hold or halt on one rail never blocks or delays
  another. Every exactly-once, crash-recovery, ceiling, kill-switch and
  link-confirmation rule in this design is stated per rail. D6 is the first instance
  of this rule.
- **(D9) One staging build of the whole NEAR AI path.** "What we need is a
  stageing build of the sign in, settlement and private inference system"
  (2026-10-21). A `near-ai-staging` build points every NEAR AI host at
  staging together: the daemon's sign-in and credential API, Private AI
  inference and its receipt endpoint, the billing dashboard links, and the
  server's transfer client. No part of one build talks to staging while
  another part talks to production, so a staging sign-in mints a key that
  staging inference accepts. Attestation stays on and fails closed: the
  staging build carries staging's own gateway and receipt-signer pins,
  never a switch that skips verification. The build reports itself as
  staging in the daemon's status, and the release check refuses the feature
  in any shipped build.

This spec covers the rail interface and the payout pass that drives every
external rail, and then the first external rail: paying Trace Commons
contributors in NEAR AI inference credit through the org-to-org transfer
API that nearai/cloud-api PR #1271 added. For that rail it covers the
commons side (a transfer client, crash recovery and reconciliation), the
contributor side (linking a NEAR AI organization), eligibility, a staging
test plan and the threat model.

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

- **2026-10-21: pay credit earned before an account existed.** "we should
  pay out credit that settled before had an account when they create an
  account." A bare-principal line accrues unowned and is paid to the first
  account its principal is linked to (section 4, "Which lines accrue").
- **2026-10-21: no reversals on NEAR AI.** #1271 has no transfer reversal
  ("Reversals and automatic accounting repair are outside this PR"), so a
  sent credit cannot be recalled; reversed credit is netted against later
  credit (section 4, "Reversals").

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

**Goal.** One payout pass drives any number of external rails, each paying
its own instrument from the same settled award (D8). The first rail: a
contributor account that has linked a NEAR AI organization receives NEAR AI
inference credit for the Trace Credit it settles, as soon as it settles.
The commons sends from its own reseller org with a scoped credential. Every
send, on every rail, is exactly-once, budget-capped and reconcilable
against that rail's own external history (for NEAR AI, #1271's transfer
history).

**Funding model** (owner, 2026-10-19). The commons runs one reseller org.
NEAR AI funds it first with an admin grant. Later the commons buys payment
credits. USD only. The reseller sends; contributors receive `transfer`
credits in the org they chose.

**Non-goals.**

- Changing Trace Credit. A payout leaves the ledger, the batches and the
  NEP-141 rail exactly as they are (D6). Trace Credit is a rail in the
  sense of section 3, but its settlement is the existing code: nothing here
  refactors `trace_credit_ledger`, `trace_credit_settlement_batches`,
  `trace_near_credit_outbox`, `NearPayoutAdapter`, `process_payouts` or
  `SettlementAdapter` onto the new interface.
- Designing any rail other than NEAR AI. A NEP-141 Trace Credit token rail
  and a BAT rail are named as future rails only; how each proves a
  recipient and answers the interface's reads is open (section 12).
- Clawback. #1271 has no reversal. Over-payment is netted against the same
  account's later credit (section 4).
- Attributing what a recipient spends to the commons. #1271 defers it.

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
   `allowed_source_types` naming the source the sender is funded from
   (see step 6) and, on staging,
   `allowed_recipient_ids` set to the test recipient orgs. Both permissions
   are needed: `credits:transfer` alone cannot read capacity or history.
6. **Hold the secret.** The token is shown once. Put it straight into the
   host's secret store. Ingest reads it from env only:
   - `TRACE_COMMONS_NEAR_AI_PAYOUT_CREDENTIAL` (held as `SecretString`,
     refused if empty or not `oc_` plus 64 hex);
   - `TRACE_COMMONS_NEAR_AI_PAYOUT_CREDENTIAL_ID` (the credential `id`; not
     secret, used only for its log hash; optional since 2026-10-31: unset,
     log lines carry `credential=unset`, while a set value must be a UUID);
   - `TRACE_COMMONS_NEAR_AI_PAYOUT_SENDER_ORG` (the reseller org UUID);
   - `TRACE_COMMONS_NEAR_AI_PAYOUT_SOURCE` (`payment`, `grant` or
     `staking_farm`; required, **no default**). It names the credit the
     sender org sends from, which depends on how that org was funded and on
     what its policy and the credential allow. Verified on staging on
     2026-10-23: the sender org is funded with paid credit, its policy
     allows `payment` and `staking_farm`, and the working credential allows
     `payment` only, so staging runs `payment`. The capacity probe reports a
     maximum per source (0 for a source the credential may not use) and
     carries fields the client does not read, which it ignores;
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

## 3. Placement and the rail interface

### The award and the rails

The **award** is the settled Trace Credit on a finalized batch line
(`TraceCreditAccountSettlementLineItem`, one per credit account per batch).
A **rail** is one way that award is paid out. Every rail reads the same
finalized lines; none reads another rail's records, and none consumes the
award (D6, D8).

| Rail | Instrument | Recipient | Settled by | Status |
|---|---|---|---|---|
| `trace_credit` | `trace_credit`, 6 decimals | the credit account itself | the existing ledger write at settlement; NEP-141 payout by the existing outbox | exists, unchanged |
| `near_ai` | `near_ai_usd`, 9 decimals (1 atomic unit = 1 nano-USD) | a NEAR AI org the account links (section 5) | the payout pass, through #1271 | this design |
| a NEP-141 Trace Credit token, BAT, ... | per rail | per rail | the payout pass | future; not designed (section 12) |

`trace_credit` is a rail in name and in readiness reporting only. Its
"send" is the batch line itself; the payout pass never drives it. The
interface below is for **external** rails, of which `near_ai` is the
first.

### Rail ids

A `RailId` is a validated newtype with the same rule as `InstrumentId`
(1-64 lowercase ASCII letters, digits, `_`, `-`, `.`). The NEAR AI rail is
`near_ai`. It is never `near`: `SettlementAdapter::payout_rail()` already
returns `near` for the on-chain NEP-141 outbox rail, and readiness reports
that label. The two must not be confused in logs, readiness or storage.

### The interface

The contract types and the trait live in `crates/trace-commons-gate-api`
(AGPL), beside `SettlementAdapter`, in a new `payout_rail.rs`. They reuse
`InstrumentId`, `InstrumentDescriptor` and `AtomicUnits` from
`pipeline.rs`. The pass, its storage and the NEAR AI implementation live in
`crates/trace-commons-server` and hold every rail as
`Arc<dyn PayoutRail>`, never a concrete type, per the repo's seam rule.

A rail declares:

- **Identity:** `rail_id`, an `adapter_identity` label for hash-only
  reports, and `production_qualified()` (false by default; readiness fails
  closed on false).
- **Instrument:** an `InstrumentId` and its `InstrumentDescriptor`
  (`decimals` included). Every amount the pass handles for this rail is
  `AtomicUnits` (`u128`) in this instrument; storage is `NUMERIC(39,0)`.
  A rail whose wire is narrower (#1271 takes `i64`) narrows at its own
  boundary and refuses out of range there (section 4,
  `payout_amount_out_of_range`).
- **Rate source:** a `RailRate { version, numerator, denominator }`,
  meaning `rail_units = floor(award_atomic_units * numerator /
  denominator)`, `u128`, checked. The server builds it at boot from the
  rail's configured source and records it as a rate epoch per
  `(rail_id, version)` (section 4, "Rate"). For `near_ai` the source is the
  installed settlement table (D2): numerator `usd_micros_per_credit *
  1000`, denominator `atomic_units_per_credit`. A future rail may use a
  price feed; it still yields one immutable rational per version.
- **Recipient-link kind:** a safe label (`near_ai_org` for this rail)
  naming what a link holds and which proof verifier the link route runs
  (section 5). The recipient itself is sealed in storage and handed to the
  rail as an opaque value for the length of one call.
- **Exactly-once send.** A send is identified by its **line**: a line ref
  hash, the link's program, a `target_total` (everything this link's
  program should have received once the line lands) and the `delta` (the
  target minus our confirmed total on the program). The contract: **a
  repeated `send` for the same line never repeats the effect.** It returns
  `Committed { receipt_hash, delta, total_after }` or
  `AlreadyAtOrAbove { total }`. Each rail meets the contract with its own
  mechanism: `near_ai` sends `recipient_cumulative = target_total` and
  ignores `delta`; an amount-mode rail sends `delta` under an idempotency
  key derived from the line ref; an on-chain rail would carry the line ref
  in a memo or nonce.
- **Reads:** `read_total(recipient, program)` (the external total on this
  program, exact), `find_receipt(line)` (the external record that answers
  this line, or none), and `history(recipient, program)` for the
  reconciliation drill. A rail that cannot answer these reads cannot meet
  the recovery and close rules below, and does not qualify.
- **Capacity:** `preflight()` returns whether sending is enabled, the
  per-send maximum, and the sends left in the current window.
- **Recipient liveness:** `probe_recipient(recipient)`, used at link time.
- **Refusal mapping:** every rail error maps into one closed enum,
  `RailRefusal`, each variant carrying a safe label: `Retry` (line stays
  `pending`), `StopPass` (stop this rail's pass, change no row; any
  credential fault), `HoldLine`, `RecipientUnavailable` (hold, mark the
  link), `Failed` (a bug on our side). The pass acts on the variant only,
  never on a rail's raw error. Section 4's table is the `near_ai` mapping.

### Independence

Each enabled rail runs as its own worker task with its own single-flight
advisory lock (keyed on `payout-pass-v1:{rail_id}`), its own mode setting,
its own control row (kill switch), its own capacity probe, its own ceilings
and its own readiness entry. A `StopPass`, a halt, a hold, an exhausted
budget or a crash in one rail's pass is invisible to every other rail: no
shared lock, no shared loop, no shared row. A settlement wakes every rail's
worker (section 7).

### Why not `SettlementAdapter`

`SettlementAdapter` performs the award instrument's own external effect,
per settlement leg, per pipeline run, inside the pipeline's settle step. A
leg is per trace and per run, not per account; it carries no recipient;
and the withdrawal guard at `versioned_pipeline.rs:15139` would need a
re-check first. A per-leg cumulative adapter does not compose at all: a
retry after a lost response returns `no_op` with no transfer id, and two
legs of one account settling close together leave the lower target with no
receipt of its own. So the two seams stay separate: `SettlementAdapter`
for awarding, `PayoutRail` for paying an award out per account. The pass
gets "as quickly as possible" (D4) without any of that, because it is
triggered by each settlement (section 7).

`PayoutRail` is also not `NearPayoutAdapter`: that one is on-chain, its
`payout_eligible` is wired to rail `near`, and its outbox is the NEP-141
rail's. This design neither reads nor writes that outbox.

### What makes the interface real

A second rail is implemented in tests (section 8): an amount-mode,
idempotency-keyed rail with an 18-decimal instrument of kind other than
`credit_account`, and its own rate. The generic pass, storage and drill
run against it and against `near_ai` with the same code, and a test runs
both rails at once with one stopped. An interface that only ever met
#1271's cumulative mode would not pass that test.

## 4. The payout pass and the NEAR AI rail

Everything in this section is per rail unless it names `near_ai`. Where a
rule names `near_ai` or #1271, it is that rail's implementation of a
generic rule from section 3.

### Crate and licence

The rail contract (`payout_rail.rs`) is in `crates/trace-commons-gate-api`
(AGPL). The pass, the storage in `db/`, the rate-epoch recorder and the
NEAR AI rail (`near_ai_transfer.rs`, the client, and `near_ai_rail.rs`,
its `PayoutRail` implementation) live in `crates/trace-commons-server`
(AGPL). Routes, readiness and the drill live in `crates/trace-commons-ingest`
(AGPL). The link request and response types go in
`crates/trace-commons-protocol` (permissive), beside
`inference_connection.rs`, because the daemon sends them; they carry the
rail id as a plain string, so protocol never depends on gate-api. No
permissive crate gains a server or gate dependency.
`tests/license_boundary.rs` must stay unchanged.

### The NEAR AI client

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
`near_ai_rail.rs` wraps it as `PayoutRail`: `send` is `send_cumulative`
with `target_total` narrowed to `i64`; `read_total` is a
`recipient_cumulative: 0` send; `find_receipt` and `history` page
`history_page`; `preflight` is `capacity`; `probe_recipient` is the
`tcprobe-v1` zero send (section 5). The error table below is its
`RailRefusal` mapping.

### Recipients are per account, per rail

The unit of payment is a credit account, `(tenant_id, account_id)`, never a
tenant, on one rail. A tenant can hold many accounts; each account links
its own recipient on each rail, or none. Only the account itself can link,
change or unlink (section 5), and only its own credit goes to its own link.
Linking or unlinking on one rail changes nothing on another.

### Program and link generation

Every external total is scoped to one link generation, called its
**program**. For `near_ai`, #1271 scopes a cumulative total to sender +
recipient + program; if one program name served every link, an account
that moved from org X to org Y would get a fresh total at Y, and a
cumulative target would pay all its history again. The rule is generic:

- Each link has a **generation id** (`link_id`, UUID) and its own program,
  derived by the rail. For `near_ai`: `tc1-` plus the first 32 hex of
  `sha256("nearai-payout-program-v1:" || link_id)`. It is opaque to the
  recipient and fits in 100 bytes.
- When a link closes, the pass first resolves it (section 5, "Change and
  unlink") and records `final_paid` (rail atomic units, `NUMERIC(39,0)`)
  from the rail's `read_total`.
- The target for the account's live link on a rail is
  `owed(account, rail) - sum(final_paid of the account's closed links on
  that rail)`.
- Relinking to a recipient used before still gets a new generation, so a
  new program. Nothing collides.

### Rate

Each rail has its own rate source (section 3) and its own epochs. For
`near_ai`, D2: the rate is the installed settlement table's
`usd_micros_per_credit`.

- **Epochs follow source versions, per rail.** At boot, for each enabled
  rail, ingest asks the rail's rate source for its `RailRate` and records
  `(rail_id, version, numerator, denominator)` in an append-only
  `trace_payout_rate_epochs` table with `observed_at` set by the database
  clock, if that `(rail_id, version)` is not already there. For `near_ai`
  the version is the settlement table's `version` (with `pricing_version`
  kept beside it as a label). If `(rail_id, version)` is there with a
  different numerator or denominator, ingest refuses to start with
  `payout_rate_version_conflict`: a version names one rate forever. Epochs
  are ordered by `observed_at`, which no one can set, so none can be
  backdated. One rail's conflict refuses boot as a whole, because it is a
  configuration fault, not a runtime one.
- **Which epoch prices a batch.** On each rail, a finalized batch is priced
  at that rail's epoch in force at the batch's `created_at` (the latest
  epoch observed at or before it). A batch older than the rail's first
  epoch is priced at the first epoch. This is a pure function of stored
  rows, so it gives the same answer whenever accrual happens (at
  settlement, or later when an account first links).
- **No rate, no payout.** A source with no positive rate (for `near_ai`, a
  table without a positive `usd_micros_per_credit`) records no epoch. With
  no epoch at all, that rail accrues nothing and sends nothing. Nothing
  falls back to a default. Other rails are unaffected.
- **The number stays out of the repo.** Tests use synthetic rates, as the
  settlement-estimate tests do. The rate is not logged; logs carry the
  rail id and epoch `version` label only.

### Accrual

A new table, `trace_payout_accruals`, holds one row per rail per settled
account line, priced once:

- Key `(tenant_id, rail_id, settlement_batch_id, credit_account_hash)` for
  a credit row, and `(tenant_id, rail_id, reversal_event_id)` for a
  reversal row (below).
- Columns: `account_id`, `kind` (`credit` or `reversal`),
  `award_units NUMERIC(39,0)` (signed, Trace Credit atomic units),
  `rate_version`, `rail_units NUMERIC(39,0)` (signed, the rail's atomic
  units), `instrument_id`, `created_at`.
- The pass inserts with `ON CONFLICT DO NOTHING`, so re-running accrual is
  harmless and a row is never re-priced. `rail_units =
  floor(award_units * numerator / denominator)` at the epoch, in `u128`,
  checked; a reversal row uses the same formula on its absolute value and
  is negated, so it takes back exactly what its credit added. For
  `near_ai` this is `floor(micros * usd_micros_per_credit * 1000 /
  atomic_units_per_credit)` nano-USD.
- **Which lines accrue.** Every line of every `finalized` batch whose
  `settled_credit_delta_micros` is positive, whether it is keyed to an
  account or to a bare principal (owner decision, 2026-10-21: credit earned
  before the person had an account is paid once they have one).
  - **Owner of an accrual.** Each accrual row carries `account_id`. A line
    keyed `account:{account_id}` gets it at accrual. A line keyed to a bare
    principal accrues with `account_id` NULL; the accrual pass fills it in
    the first time `trace_account_principals` (V30) links that principal to
    an account, with `UPDATE ... SET account_id = $1 WHERE account_id IS
    NULL`, and writes an audit row. The owner is set once and never
    changes, so a principal that later moves to another account cannot be
    paid twice: its old credit stays with the account that first claimed
    it. `owed(account, rail)` sums accruals by `account_id`; an unowned
    accrual is owed to nobody yet.
  - **Ownership is not a payout redirect.** Claiming happens only through
    the existing principal-to-account link, which has its own
    authentication; no payout route can set `account_id`. The payout pass
    sends nothing extra for a claim beyond what `owed` now includes, so a
    newly claimed backlog goes out on the next pass under the same
    per-send, per-account and credential ceilings.
  `dry_run` and `failed` batches never accrue. Each rail accrues the same
  lines independently; a rail with no epoch yet accrues them later, at the
  epoch rule above.
- **NEAR wallet holds do not block the `near_ai` rail** (a default,
  section 12). `near_payout_hold_reason` says the account has no NEAR
  wallet to pay. `near_ai` does not pay a wallet, so the label is not a
  reason to withhold NEAR AI credit; the line accrues to its own account,
  and to no one else. A future rail that does pay a NEAR wallet (the
  NEP-141 token rail) would treat the label as a hold. What does hold a
  rail is in "Holds" below.
- **Owed** for an account on a rail is `sum(rail_units)` over its accrual
  rows on that rail. It is never read from `trace_credit_ledger` sums or
  `account_credit_totals` (a display path that sums `f32`).

### Reversals

- A reversal row (`settlement_state = 'reversed'`, negative `points_delta`)
  names the event it reverses in its `external_ref`. On each rail, the pass
  finds the finalized batch line whose `source_credit_event_ids` holds that
  event, and writes a `reversal` accrual for the same account at the rate
  version that rail priced that line at, for the reversed event's own
  amount.
- If the reversed event was never in a finalized batch, nothing was accrued
  and nothing is written. If the line cannot be found for a reversal of a
  settled event, the account is held on that rail as
  `payout_reversal_unpriced`.
- **Netting, not clawback** (a default, section 12). If owed on a rail
  drops below what the live link's program has already received, the
  target is below the program total, so the pass sends nothing on that
  rail. The account's later credit fills the gap before anything more is
  sent. If the account never earns again, the difference is a loss to the
  commons, bounded by the account ceiling and the rail's lifetime cap
  ("Ceilings"). Nothing is taken back on any rail. For `near_ai`, that
  stays so unless #1271 adds a reversal, which would need its own design.
- Only a reversal row lowers owed. Per the roadmap (l.36-38) that is an
  operator-raised correction, not an automatic result of a revocation or a
  withdrawal, which leave settled Trace Credit, and so owed, as they are.

### Holds

A held account accrues as usual and gets no send on that rail until the
hold clears. Holds are per account and per rail, and never move credit to
another account. The first two holds below derive from shared account
state, so they hold every rail at once; each rail still checks them
itself. The rest are recorded per rail and hold only that rail.

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
On every rail, every amount that leaves is bounded three times, and each
bound is checked before the line is written `pending`:

- **Per send:** the target is at most `program_total + per_send_max` from
  the rail's `preflight` (for `near_ai`, `max_amount_per_transfer` from
  `capacity`), so back pay steps up over successive passes.
- **Per account, lifetime, per rail:** a target above the rail's account
  ceiling (for `near_ai`,
  `TRACE_COMMONS_NEAR_AI_PAYOUT_ACCOUNT_CEILING_NANO`) writes no line and
  holds the account on that rail as `payout_over_ceiling`. An operator
  lifts it by raising the setting; nothing is sent above it in the
  meantime. A rail with no ceiling configured refuses to start in `dry_run`
  or `live`.
- **Whole rail, lifetime:** enforced by the external side where it can be
  (for `near_ai`, the credential's `max_total_amount`, enforced by
  cloud-api under its own lock), and by our `preflight` probe before the
  pass.

`u128` arithmetic, checked, in the rail's atomic units. A result the rail's
wire cannot carry (for `near_ai`, above `i64::MAX`) holds the account as
`payout_amount_out_of_range`.

### Modes and the kill switch

Each rail has its own mode setting; for `near_ai` it is
`TRACE_COMMONS_NEAR_AI_PAYOUT_MODE`. Every mode defaults to `disabled`.

- `disabled`: no accrual, no pass, no link probe on that rail. Its link
  route answers `payout_unavailable`.
- `dry_run`: accrual runs (it moves no money and makes later live sends
  match). The pass computes each target and records a count-only report
  (accounts that would be sent to, total rail units, holds by label). It
  makes **no write call to the rail at all**, not even a zero read (for
  `near_ai`, no POST to #1271); it may call `preflight`. No line is
  written.
- `live`: as specified below.

The kill switch is runtime, not boot-time, and per rail: an operator route
`POST /v1/admin/payout-rails/{rail_id}/halt` sets that rail's row in
`trace_payout_control` (with an audit row: rail id, actor hash, time,
label), and `.../resume` clears it. The rail's pass reads it before each
send, under its single-flight lock, and stops at once when it is set.
Lines already `pending` stay `pending`; recovery resumes them after
`resume`. Readiness reports `payout_halted` for that rail. Halting one rail
does not touch another; there is no global halt (an operator halts each
rail). The switch is rail-wide, so it is not a tenant control: halt, resume
and hold clearing accept only the deployment's payout control token
(`TRACE_COMMONS_PAYOUT_CONTROL_TOKEN`, stored as a hash, at least 32
characters), never a tenant token, a tenant admin's included, and refuse
every caller (`payout_control_unconfigured`) while it is unset. A rail in
`live` cannot run without it: boot refuses
(`payout_control_token_unconfigured`) when any rail is `live` and the token
is unset, so a live rail always has a reachable kill switch. `dry_run`
sends nothing and boots without it, logging the same label. Audit and
control rows name the operator by a hash of the token's hash.

Holds an operator clears. A hold about one account (recovery unmatched,
program total mismatch, link unreadable, amount out of range, owed
inconsistent, a `HoldLine` or `Failed` refusal on its line, an unpriced
reversal) stays until an operator clears it with the same token:
`GET /v1/admin/payout-rails/{rail_id}/tenants/{tenant_id}/holds` lists that
tenant's open holds on the rail by account id hash and label, and
`POST .../holds/clear` with `{account_id_hash, label}` clears one, with a
`hold_cleared` audit row naming the operator by hash. The next pass looks at
the account afresh. The ceiling hold also lifts itself once owed fits the
ceiling. A refusal caused by the sender's own limits (credential or org
transfer limits, rate limits, accounting busy, insufficient posted capacity,
an unreconciled sender) never holds an account: it retries or stops the
pass.

### State and storage

One set of tables serves every rail, each row carrying `rail_id`. New
tables, RLS forced, tenant predicates through `trace_current_tenant_id()`:
the accruals above, and

- `trace_payout_links`: `tenant_id`, `account_id`, `rail_id`, `link_id`,
  `recipient_kind` (the rail's label, `near_ai_org` here), `state`
  (`waiting`, `active`, `closing`, `closed`, plus the mark
  `recipient_unavailable`), `sealed_recipient JSONB`, `recipient_index`
  (keyed blind index), `recipient_key_ref`, `index_pepper_ref`,
  `subject_index` (keyed blind index of whatever proved the recipient; for
  `near_ai` the NEAR AI subject, section 5), `program`, `final_paid
  NUMERIC(39,0)` (rail atomic units, set on close), `unlink_requested_at`,
  `version`, timestamps. Two partial unique indexes allow at most one
  `waiting` row, and at most one row that is `active` or `closing`, per
  `(tenant_id, account_id, rail_id)`. Active and closing share a slot
  because a new link's target leaves out what a still-closing link was
  paid: a link goes active only once its predecessor is closed, in the
  same transaction. There is no uniqueness on `recipient_index`: many
  accounts may name one org (D7), so no recipient can be locked away from
  its real owner by an earlier link.
- `trace_payout_lines`: `tenant_id`, `account_id`, `rail_id`, `line_id`
  (UUID), `link_id`, `target_total NUMERIC(39,0)`,
  `owed_through_accrual_count`, `state` (`pending`, `confirmed`, `no_op`,
  `held`, `failed`), `receipt_hash`, `delta NUMERIC(39,0)`, `attempts`,
  `last_error_label`, timestamps. A partial unique index on `(link_id)
  WHERE state = 'pending'` (a link has at most one unresolved line), and
  one on `(rail_id, receipt_hash) WHERE receipt_hash IS NOT NULL` (one
  external record answers one line).
- `trace_payout_link_requests`: the pending link or unlink requests of
  section 5 (rail id, sealed recipient, subject index, ceremony handle
  hash, idempotency key, expiry), per account and rail.
- `trace_payout_holds`: open and cleared holds per `(tenant_id,
  account_id, rail_id)` with a label; cleared rows stay.
- `trace_payout_audit`: append-only, hash-only. One row per link request,
  confirmation, change, unlink, close, operator release, hold set or
  cleared, halt and resume: rail id, action, actor ref hash, link id hash,
  prior and new state, label, time. Nothing is updated in place without a
  row here.
- `trace_payout_rate_epochs` and `trace_payout_control` (one row per rail)
  are not tenant data. They get forced RLS with a read-all policy, and
  writes only through a narrow role used by the boot path and the admin
  routes.
- **Enumeration.** Each rail's pass and the drill need the tenants with a
  `waiting`, `active` or `closing` link on that rail, which no tenant
  context can see under forced RLS. `waiting` is included because every
  confirmed link is written `waiting` (section 5, step 7) and only the pass
  makes it `active`: a tenant whose only link is waiting (every first link,
  or a successor left by a crash) must still get a pass. A narrow role, `trace_payout_enumerator`, with a
  permissive `SELECT` policy on `trace_payout_links (tenant_id, rail_id,
  state)` only, assumed with `SET ROLE` for that one query, as
  `docs/operator/register-stats-role.md` does. It can read nothing else.
  Every per-tenant read and write after that runs under that tenant's
  `trace_current_tenant_id()`.
- **Migration number:** the next free number above the highest on `main`
  when the schema PR is written, never V129 (reserved by #1415, which is
  closed, and possibly applied to someone's test database, where the
  runner would silently skip a reuse). The plan says how to check.

### One pass, per rail

Each step below runs inside one rail's pass. Rails run these steps
concurrently and never wait on each other.

0. **Single flight, per rail.** The rail's pass takes a session-level
   advisory lock on `pg_try_advisory_lock(hashtextextended('payout-pass-v1:'
   || rail_id, 0))` (the hashing at `versioned_pipeline.rs:2177`) on a
   connection it pins for the whole run, and releases it with
   `pg_advisory_unlock` on every exit path; a crash drops the connection and
   the lock. If the lock is held, the trigger sets that rail's "run again"
   flag and returns (section 7). Every link state change on the rail
   (`active` to `closing`, `closing` to `closed`, `waiting` to `active`) is
   made by its pass while holding the lock. The routes only record
   requests.
1. Enumerate tenants with a link on this rail (above). Per tenant, under
   its context:
   a. **Accrue** every finalized batch line and reversal not yet accrued on
      this rail.
   b. **Apply requests**: move a live link with `unlink_requested_at`, or
      with a confirmed `waiting` successor, to `closing`, and resolve it
      (section 5).
   c. For each account with an `active` link: skip it if it is held on this
      rail, or if its link has a `pending` line (that line goes to crash
      recovery first). Read the link's program total with `read_total`
      (for `near_ai`, a `recipient_cumulative: 0` send). Compute the target
      (owed minus closed links' final totals, capped per send). Skip unless
      the target exceeds the program total. Check the account ceiling.
2. Write the line `pending` with its `target_total` and `delta`, committed,
   before any network call.
3. Call `preflight` once per pass. If sending is not enabled (for
   `near_ai`: `sending_enabled`, `sender_reconciled` or
   `credential_can_transfer` is false), stop this rail's pass with that
   label. Pace sends to the sends left in the window and to a local limit
   below the rail's quota (for `near_ai`, `max_sends_per_minute`).
4. Re-read this rail's kill switch. Call `send` for the line. For
   `near_ai`: `recipient_cumulative = target_total` with the link's
   program, memo empty, metadata
   `{"tc": "<sha256 of 'nearai-payout-line-v1:' || line_ref_hash>"}`. The
   rail contract hands a rail the line's reference hash, never its id.
5. Record the outcome:
   - `Committed`: `confirmed`, `receipt_hash`, `delta` as the rail reports
     it. If the rail reports `total_after` and it is not the target, hold
     as `payout_program_total_mismatch`. For `near_ai`, `Committed` is a
     201, `receipt_hash = sha256("nearai-transfer-v1:" || id)`, `delta =
     amount`.
   - `AlreadyAtOrAbove { total }`: if `total >= target`, run recovery
     (below); otherwise hold as `payout_program_total_mismatch`. For
     `near_ai` this is the 200 `no_op`.
   - A `RailRefusal`: as the variant says (section 3); the `near_ai` table
     is below.

### Crash recovery, per rail

A crash between step 2 and step 5 leaves a `pending` line whose send may or
may not have committed. Because a link has at most one pending line and the
rail's pass is single-flight, nothing else was written or sent for that
link in between. The next pass of that rail calls `send` for the **same**
line first, which the rail contract makes safe:

- If the first send never committed, it is sent now (`Committed`).
- If it did, a cumulative rail answers `AlreadyAtOrAbove` and a keyed rail
  answers `Committed` with the first receipt. On `AlreadyAtOrAbove` the
  pass calls `find_receipt`. For `near_ai`, that pages
  `GET .../credit/transfers?direction=outgoing` and filters client-side for
  this recipient, this program and `recipient_cumulative == target_total`,
  skipping any transfer whose id hash is already on another line. History
  has no program filter, and the cursor is still returned on the last page,
  so it stops on a short page.
- If no record matches, hold the line as `payout_recovery_unmatched`. Under
  the one-pending-line rule this cannot happen without a bug or an outside
  send on our program.

### `near_ai` refusal mapping

| #1271 reply | `RailRefusal` | Line state | Notes |
|---|---|---|---|
| 429 `transfer_rate_limited`, 503 `transfer_unavailable` / `accounting_busy`, timeout | `Retry` | stays `pending` | retried next pass, same target |
| 409 `insufficient_posted_capacity`, `sender_not_reconciled` | `StopPass` | stays `pending` | operator: fund or reconcile the sender |
| 409 `credential_transfer_limit_exceeded`, `organization_transfer_limit_exceeded` (on a send or a zero read) | `StopPass` | stays `pending` | a limit on our own credential or org (budget, source, or recipient list, which the link probe already checked); never the recipient's standing, so no account is held. Raise the limit; the next pass resends the line |
| 409 `capacity_overflow` | `HoldLine` | `held` | the recipient's incoming total would overflow `i64`; operator review |
| 404 `organization_unavailable` on a send | `RecipientUnavailable` | `held`, link marked `recipient_unavailable` | contributor must relink; #1271 raises this code for the recipient only |
| 404 with any other code, or no JSON body | `StopPass` (`near_ai_route_unavailable`) | unchanged | a wrong route, a proxy page or a host without #1271; never marks a link |
| 2xx on a send whose body fails the shape check | `Retry` | stays `pending` | money may have moved; the resend answers `AlreadyAtOrAbove` and recovery finds the receipt |
| 403 `credential_refused` | `StopPass` | unchanged | credential malformed, expired, revoked or wrong |
| 403 `sender_not_enabled` | `StopPass` | unchanged | no policy row, or transfers disabled; ask NEAR AI |
| 400 any | `Failed` | `failed` | a bug on our side; page the operator |
| anything else on a send | `Failed` | `failed` | closed fallthrough, never a guess |
| on a read (`capacity`, history) or the probe: anything the rows above do not map | `StopPass` | unchanged | there is no line to fail; also a `capacity` reply without the configured source, a history record from another sender, and a history cursor that does not move strictly older |

A `StopPass` stops that rail's pass and changes no row. In particular a 403
or 401 on **any** `near_ai` call, read or send, is a fault in our
credential, not in an account: no account or link is held, frozen or closed
because of it, and no other rail notices.

### Fail-closed rules

- A rail in `dry_run` or `live` with any of its settings missing or
  malformed refuses ingest's start, with a per-rail label (for `near_ai`,
  `near_ai_payout_config_missing`). A rail in `disabled` needs no settings.
- An unknown rail id anywhere (a mode setting, a route segment, a stored
  row) is refused, never defaulted.
- The `near_ai` host is compiled in. A `near-ai-staging` build switches it
  for introspection, payout and the daemon together, and reports itself in
  readiness. A CI check (beside `scripts/ci/check-release-credential-store.sh`,
  with its own self-test) refuses any release workflow or shipped manifest
  that enables the feature. No env value names a host.
- Readiness reports one entry per rail, `trace_credit` included, with
  `rail_id`, `mode`, `halted`, `rate_epoch` (the version label only) and
  `qualified`. For `near_ai`, `qualified` is true only in `live`, outside
  `near-ai-staging`, with a rate epoch in force and a startup `capacity`
  probe answering `sending_enabled: true`. One rail not qualified does not
  make another unqualified.
- A link whose sealed recipient fails to open: hold that rail as
  `payout_link_unreadable`.

### Hash-only logging

Logs and audit rows carry labels and `sha256:` values only: rail id, line,
link, receipt, program and credential id hashes, amounts in the rail's
atomic units, and the rate epoch's version label. Never a recipient (for
`near_ai`, the recipient or sender org UUID or the org name), a token, a
memo, metadata, the tenant or account id, or the rate. This follows
`docs/operator/hash-only-logging.md`.

## 5. Linking an account to a recipient

Linking is generic in shape and per rail in proof. The routes, the request
table, the session rule and the audit are one mechanism with a rail
segment. What proves the recipient is the rail's **recipient-proof
verifier**, chosen by the rail's recipient-link kind; the body carries a
per-rail proof, a tagged enum keyed by rail id. Only `near_ai` is accepted
now; any other rail id is refused as `payout_rail_unknown`. The rest of
this section is the `near_ai` verifier and the shared mechanism.

### Who can link (`near_ai`)

Any account whose person can sign in to NEAR AI in the app and owns the
org they choose (D3). The proof is a fresh NEAR AI access token whose
`/users/me` lists the org as active and owned. The authority to direct the
account's credit is the account's signed-in session (D5 as superseded on
2026-10-23; see "Who confirms" below).

| Account class | NEAR AI subject on file | Check at link time |
|---|---|---|
| `nearai-` tenant, or any account with a `near_ai_login` anchor | yes (the anchor) | `/users/me` `id` must match the anchor, else `subject_mismatch` |
| `near-` tenant (wallet) | no | the person signs in with NEAR AI in the app; the subject is recorded as `subject_index` on the link |
| passkey-origin account, bound or not | per its binding, or none | as the two rows above |
| account in a legacy or pooled tenant | no | as the wallet row |

Recording `subject_index` lets an operator see that one NEAR AI subject
linked several accounts, without opening anything. It is not an anchor and
grants nothing; it is keyed under its own domain, so its value is never the
login anchor's and a link row cannot be joined to a tenant's anchor.

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
   `organization_unavailable` refuses as `near_ai_org_unavailable`. 409
   `credential_transfer_limit_exceeded` refuses as
   `near_ai_org_not_permitted` (expected on a closed pilot list). 403
   refuses as `payout_unavailable`. Any other reply is ours, not the
   contributor's, and refuses as `payout_unavailable` without blaming the
   org: 409 `organization_transfer_limit_exceeded` (our org's policy
   refusing the source or the recipient), a 404 with another code, or a
   reply we cannot place. #1271 runs its policy checks, source and
   `allowed_recipient_ids` included, before the no-op return and checks no
   budget at a zero target (cloud-api `check_send_policy`), so the probe
   does see a closed list; an inactive recipient left off a closed list
   answers 409, not 404, because activity is checked after policy.
6. Ingest stores a `pending_confirmation` request: sealed org id, subject
   index, recipient ref, the account it belongs to, a 5-minute expiry, and
   an opaque handle returned to the daemon. **Nothing changes yet.** The
   handle is keyed over the tenant, account, rail and idempotency key, so a
   replayed start answers the same handle.
7. The app shows the consent for "the organization you chose" and, on the
   person's click, the daemon calls confirm with the handle over the same
   account session. Confirming writes the `waiting` link (or the unlink
   mark) and its audit rows in one transaction. Expiry, a newer request on
   the same account and rail, a different account, or a link that changed
   since the request discards it.

(Until 2026-10-23 step 7 sent the person to a step-up page to confirm with
a passkey. The owner superseded that; see below.)

### Who confirms (D5, superseded 2026-10-23)

The owner: "We aren't using passkeys at all in the app right now." The
contributor's signed-in account session confirms link, change and unlink
on every rail. Any account session passes: a native token (`tcn1_`) or a
same-origin cookie session, weak or strong. A device bearer is not an
account session and is refused with 403. There is no passkey gate, no
`require_passkey_step_up`, no step-up page section, and nothing requires
the account to have a passkey; `require_authenticator_change_allowed` and
the NEAR wallet payout redirect keep their own rules.

The API keeps two calls (start, then confirm) so the app can show the
consent between them, and so that nothing is written until the person
confirms. What stands behind a link without a passkey:

- the server's own ownership check with NEAR AI (steps 4 and 5), with the
  person's token, against the host that pays;
- the "changed since shown" check: the status route reports a `version`
  per account and rail that counts every link the account was given and
  every unlink it confirmed (only the person's changes move it; the pass
  activating or closing a link does not). Start and confirm both refuse
  `version_conflict` unless the request's `expected_current_version`
  equals it, and start checks it before the token leaves the machine;
- an audit row for every request (`link_requested`, `unlink_requested`)
  and every confirmation (`link_confirmed`, and `link_created` for a new
  link), each naming the account and link by hash only.

The owner accepted the trade-off in the option's words: "anyone holding
the app session could redirect future payouts."

### The routes

`POST /v1/account/payout-rails/{rail_id}/link-requests` (start, step 3),
copying the inference-connection stack
(`crates/trace-commons-contributor/src/inference_connection.rs:190-230`,
`trace_commons_ingest_internal/inference_connection.rs`):

- account session only (any kind, weak included); `DeviceBearer` is
  refused with 403; a cookie session only from the same origin;
- `rail_id` must name a rail whose mode is not `disabled`, else
  `payout_unavailable`; an unknown id is `payout_rail_unknown`;
- body `deny_unknown_fields`: `action` (`link` or `unlink`),
  `idempotency_key` (non-nil UUID), `expected_current_version`, and for
  `link` a `proof` tagged by rail. For `near_ai` the proof is
  `{"rail": "near_ai", "organization_id", "near_ai_access_token"}`;
- outcomes: `pending_confirmation` (with the handle, expiry and recipient
  ref), `idempotency_conflict`, `version_conflict`, `payout_not_linked`
  (an unlink with nothing linked), `payout_account_ineligible` (a closed
  account), `invalid_payout_link_request`, `payout_rail_unknown`,
  `payout_unavailable`, and the verifier's own refusals (for `near_ai`:
  `near_ai_login_unverified`, `subject_mismatch`, `org_not_owned`,
  `near_ai_org_unavailable`, `near_ai_org_not_permitted`). There is no
  `passkey_required`.

`POST /v1/account/payout-rails/{rail_id}/link-requests/{handle}/confirm`,
on the same account session rule, called by the daemon after the person
confirms in the app. Outcomes: `confirmed` (with the new `version`),
`expired`, `not_found` (no such request on this account, or superseded),
`version_conflict`, `payout_link_change_pending` (a link already waits for
the pass, or an unlink is already recorded), `payout_not_linked`. A
confirmed change wakes the rail's pass.

`GET /v1/account/payout-rails` returns one entry per rail that is not
`disabled`: `{rail_id, mode, linked, recipient_ref, state, version,
request_state, unlink_requested, recipient_unavailable}`, never a
recipient or a name. For `near_ai`,
`recipient_ref` is
`sha256("nearai-payout-org-ref-v1:" || idempotency_key || ":" || org_id)`;
the daemon keeps the key and the org id together, so it can recompute the
ref and check the server holds the same org without the server echoing it.
The ref is computed at start, carried on the request row and copied onto
the link (`recipient_ref` on both tables, V134), so a read never opens the
sealed recipient.

### Storage

The send needs the raw recipient (for `near_ai`, the org UUID), so
hash-only storage is impossible. The recipient is sealed with the
machinery behind `sealed_account_name`, with a keyed blind index and key
and pepper refs for rotation. The key is not in the database. The pass
opens a link's recipient with the same key (`SealedRecipientOpener`); a seal
under another key or pepper, or one whose plaintext does not reproduce its
index, does not open and the link is held `payout_link_unreadable`. Ingest
refuses to start with a rail enabled and no recipient key
(`payout_recipient_key_unconfigured`).

### Change and unlink

- **Change** confirms a new generation as `waiting`. The next pass, under
  its lock, moves the old link to `closing` first.
- **Closing.** The pass sends nothing new to a closing link and never
  re-sends to it. It resolves every `pending` line on it through reads
  only: a `recipient_cumulative: 0` read of the program total and, for the
  transfer id, history paging. A line whose target the total reached is
  `confirmed`; one it did not reach is `no_op`. Only then does it write
  `final_paid` (the read total), set the old link `closed` and the
  waiting one `active`, in one transaction, with an audit row.
- **Old org gone.** If the zero read returns 404, the pass takes the program
  total from history alone. If history cannot be read either, the link
  stays `closing`, readiness shows `payout_link_close_blocked`, and the
  pass retries.
- **Unlink** confirms `unlink_requested_at` on the live link; the pass
  closes it with no successor. Credit keeps accruing and is sent when the
  account links again.
- **Per rail.** Change, unlink and release act on one rail's link. They
  never close, open or hold a link on another rail.
- **Operator release.** `POST /v1/admin/payout-rails/{rail_id}/links/{link_id_hash}/release`
  moves a link to `closing` (for example after a contributor reports lost
  access to the org) with an audit row naming the actor hash and a reason
  label. The account then links again from its signed-in session.

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
  NEAR AI for past use, new credit pays that first." (The draft's closing
  passkey sentence is dropped: D5 superseded 2026-10-23.)
- Scope line beside the button: "Sends credit to {org name} only."
- Button: ~~"Continue with passkey"~~ (superseded). Task 7 drafted "Choose",
  with the accessibility name "Choose {org name} for NEAR AI credit".
- Secondary: "Not now"
- ~~No passkey yet~~: no longer needed; nothing requires a passkey.
- Unlink button: "Stop sending credit"; body: "Credit you earn is kept and
  sent when you choose an organization again." (the passkey sentence is
  dropped, D5 superseded)
- Status: "Credit goes to {org name}" / "No organization chosen" /
  ~~"Waiting for passkey confirmation"~~ (superseded; Task 7 drafted
  "Credit goes to {org name} from the next payout" for a confirmed link
  the rail has not made active yet) / "Organization unavailable. Choose
  another." Task 7 added a line for a commons that reports another org
  than the one chosen on this device: "Credit may be going to an
  organization you didn't choose. Choose again."

The confirm step names the org shown, and a `connection_revision`-style
check (as `near_ai_funding` does) refuses a click if the org changed since
it was displayed. As built in Task 7: the daemon's `orgs_revision` hashes
every owned active org's id and name as shown, and the start re-reads
`/users/me` with its fresh token and refuses before calling the commons if
it differs; the confirm must name the org the start was for. The keys live
in `consent_copy.rs` as `PAYOUT_LINK_*`, with `{org_name}` as the
placeholder, and cross the C ABI as `tc_payout_link_copy_json`. On the server the same check is `expected_current_version`
("Who confirms" above). macOS is the lead shell; the others follow it.

## 6. Eligibility

D3, per account and per rail. The account rules below are shared by every
rail; the link rule is the rail's own.

- **Eligible:** any open account (`trace_accounts.closed_at IS NULL`) that
  has completed that rail's link in section 5. Tenant class does not
  matter. An account can be eligible on one rail and not on another.
- **What is paid:** only credit that settled on a line keyed
  `account:{account_id}` (a default, section 12). On `near_ai` the NEAR
  wallet hold label on a line does not matter (section 4, "Accrual"); a
  credit hold does (section 4, "Holds").
- **Pooled tenants** (`trace_legacy_invite_pooled_tenants`, V81). Each
  account in a pooled tenant links and is paid for its own lines only. No
  account is paid for another's credit, and nothing is paid at tenant level.
  Credit that settled to a bare principal (a member who had no account when
  the batch was written) is paid to the account that principal is first
  linked to (owner decision, 2026-10-21). Batch lines themselves are never
  re-keyed; the accrual records the owner once (section 4, "Which lines
  accrue").
- **Legacy `tenant-...` tenants:** the same as pooled. The rule that a
  legacy tenant must first link to a NEAR account (V81) does not apply to
  the `near_ai` rail; the NEAR AI proof replaces it.
- **Wallet (`near-`) tenants:** eligible on `near_ai` after the person
  signs in with NEAR AI in the app.

The account predicate lives in one server function shared by every rail,
checked at link time and on every send.

## 7. Schedule

D4: as soon as credit settles.

- **Trigger.** Every code path that commits a finalized settlement batch
  (the pipeline's `finalize_pending_credit_batch` and the ingest settlement
  route) notifies every enabled rail's worker after its transaction commits
  (one in-process `tokio::sync::Notify` per rail; no payload, no tenant
  id). Each worker runs its own pass. A notify while that rail's pass runs
  sets its "run again" flag, so a burst of settlements becomes at most one
  extra pass per rail. A slow or stopped rail does not delay another
  rail's pass.
- **Retry tick.** Each worker also runs every 60 s (proposed). This catches
  a notify lost to a crash, retries `pending` lines, and finishes closing
  links. A pass with nothing to do costs one enumeration query.
- **Coalescing.** One send per account per rail per pass covers everything
  accrued since the last one, so `near_ai`'s 60/min quota is per
  account-pass, not per trace.
- **No minimum send.** Owner asked for speed. A floor can be added later,
  per rail, without loss, since the target carries the remainder.

At 60 sends a minute a pass serves 3,600 accounts an hour, above the pilot.
The quota can be raised by policy.

## 8. Staging test plan and reconciliation drill

### Known staging gap

Every NEAR AI host the daemon and server use is compiled in as production.
Plan (D9): a build-time feature (`near-ai-staging`) that switches all of
them together (sign-in, Private AI inference and receipts, billing links,
attestation pins, and the server's transfer client), refused in any release
workflow by a CI check and reported in readiness and in the daemon's
status. Until it
exists, test the client against a mock #1271 server, and test the link flow
against production with a closed recipient list.

### Steps

1. Section 2 setup with `allowed_recipient_ids` set to two test orgs.
2. Unit: rate-epoch selection by batch time, the version-conflict refusal,
   pricing and floor, reversal priced at its line's epoch, `u128` to `i64`
   bounds, the three ceilings, error mapping, program derivation, target
   after a closed link.
2a. **The second rail (tests only).** A test rail implementing
   `PayoutRail` in amount mode with an idempotency key per line, an
   18-decimal `erc20` instrument (a synthetic contract address) and its own
   fixed synthetic rate, backed by an in-memory ledger that counts effects.
   The generic pass, storage and drill run against it unchanged. Tests: a
   repeated send for one line has one effect; recovery after a lost
   response finds the first receipt; amounts above `u64::MAX` and near
   `10^38` round-trip through `NUMERIC(39,0)`; **two rails at once**, one
   answering `StopPass` (or halted, or holding an account) while the other
   sends to the same account in the same settlement; the stopped rail's
   lines and holds are unchanged and the other rail's line is `confirmed`.
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
   principal accrues unowned, is paid to the account the principal is then
   linked to, and is not paid again after the principal moves to a second
   account. RLS on every new table.
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

`POST /v1/admin/payout-reconcile-drill`, following
`docs/operator/drills.md`: admin bearer, `ready` plus label-only
`blocking_gaps`, hash-only evidence, wired into `rollout-smoke/evidence`.
It runs per enabled rail and reports per rail; one rail's gap is labelled
with its rail id and does not hide another rail's result. It is read-only:
it never holds, freezes or changes a row. Through the rail's `history`, it
compares, per link: the sum of our confirmed `delta` against the program
total; every confirmed `receipt_hash` against an external record; and no
external record in one of our programs that we do not hold (for `near_ai`,
a `tc1-` program). Gaps: `payout_history_unmatched`,
`payout_program_total_mismatch`, `payout_line_missing_transfer`. It also
reports, as counts, accounts whose owed is below their paid total (netting
in progress). For `near_ai`, cloud-api's admin reconciliation route is
system admin only; NEAR AI can run it.

## 9. Threat model

- **Stolen credential.** Bounded by `max_amount_per_transfer`,
  `max_total_amount` (lifetime, survives rotation), expiry (90 days
  proposed), and source restriction. For open enrollment the caps and
  expiry are the defence. Response: halt, revoke in the UI, rotate, update
  the env, restart, resume.
- **Stolen database.** Recipients (for `near_ai`, org ids) are sealed; the
  key and every rail credential are not in the database.
- **Double pay on one rail.** The rail contract makes a repeated send for
  one line a no-op (cumulative targets converge on `near_ai`; keys dedupe
  on a keyed rail), so retries and re-runs cannot overpay one link.
  Per-link programs stop a relink from paying history twice. Each rail's
  pass is single-flight, a link has at most one pending line, and a link
  closes only after its pending lines are resolved by reads. Accruals are
  unique per rail, batch line and reversal, so a batch is priced once per
  rail.
- **Paid, then reversed (D4).** With no maturation window, credit can be
  sent and later reversed by an operator. The NEAR AI credit cannot be
  recovered. The account's later credit is netted first; an account that
  never earns again leaves the commons with the loss. Upload, settle,
  cash out and leave is therefore possible for anything that passes the
  gates and is only later found to be bad. Limits on that exposure: the
  account ceiling, the credential's lifetime cap, and the existing gates
  before settlement. This is the owner's explicit choice.
- **Paid on every rail for one trace (D6, D8).** The same settled credit
  can be paid as NEAR AI credit, as NEP-141 Trace Credit (or redeemed
  later), and on any rail added later. That is the owner's design, not a
  race: each rail keys its own exactly-once record off the same finalized
  line, and none reads another's. The commons' total exposure per award is
  therefore the sum over enabled rails; each rail's ceilings bound its own
  share, and enabling a rail is an operator decision with that sum in view.
- **One rail's fault spreading.** A stolen or revoked credential, a halt, a
  budget exhausted or a bug on one rail stops that rail only (section 3,
  "Independence"). Readiness shows each rail separately, so a stopped rail
  is visible rather than masked by a healthy one.
- **Wrong recipient.** The org must be in the person's own `/users/me`,
  active and owned, checked by the server with the person's own token
  against the host that pays. Only a session on the same account can
  confirm. A wrong send cannot be reversed.
- **Stolen device or session (accepted 2026-10-23).** With D5 superseded,
  the daemon's `tcn1_` token can start and confirm a link to any org the
  holder owns on NEAR AI, so whoever holds the app session can redirect
  future payouts (not past ones: a sent credit stays where it went). The
  owner accepted this. What bounds it: the holder must also own the new
  org on NEAR AI; every change is audited; the person's app shows the
  recipient ref it did not choose as a mismatch; revoke-all ends the
  session; the operator can release the link.
- **Role lost after linking.** The server cannot re-check org ownership
  without the person's token, so a link stays valid if the person later
  loses the owner role. Credit then goes to an org they no longer own. The
  person can unlink; the operator can release. Because `recipient_index` is not
  unique, the org's real owners can always link it themselves.
- **Shared org (D7).** Credit sent to an org with other members is
  spendable by all of them. The consent copy says so.
- **Recipient debt.** Incoming transfer credit first pays any overage the
  recipient org owes. The consent copy says so.
- **Sender out of funds.** 409 `insufficient_posted_capacity` stops the pass.
  Accruals stay owed. Readiness reports it.
- **Rounding.** Floor per batch line per rail; at most one atomic unit of
  the rail's instrument per line (for `near_ai`, one nano-USD).
- **Metadata leak.** Memo empty, metadata an opaque hash.
- **Org name.** The server never deserializes, stores or logs it. The
  contributor daemon keeps the linked or pending org's name, and no other,
  in its state directory (0600) so status can show it after a restart; it
  is refreshed on every list read, deleted when the NEAR AI session is
  forgotten, and never logged (owner decision 2026-10-23).
- **Rate.** Not in the repo, not in logs; only the epoch version label.
  The settlement table is served to clients, so the rate is visible to
  them, as S2 already accepts.
- **Enumeration role.** Reads only `tenant_id`, `rail_id` and `state` of
  the link table. A leaked connection under it reveals which tenants have a
  linked account on which rail, nothing about whom to.

## 10. Delivery plan

Small stacked PRs, each passing the repo checks on its own. The plan,
[`../plans/2026-10-21-nearai-credit-payout.md`](../plans/2026-10-21-nearai-credit-payout.md),
has the files, tests and verification for each. In order: the rail
interface and types with the test rail; the schema; rate epochs and
accrual (no network); the #1271 client and the `near_ai` rail with the
staging feature; the payout pass behind a mode that defaults to
`disabled`; the link routes on the signed-in session (D5 superseded);
the daemon link flow;
the macOS org picker and draft consent copy; the reconciliation drill and
readiness; the operator runbook and staging steps.

## 11. Lessons from #1415

The review of #1415 (2026-10-20) found fourteen problems in the earlier
settlement-rail draft. Each one, and where this design handles it. Every
answer holds **per rail**: it is a property of the generic pass and
storage, so a future rail inherits it, and `near_ai` is the first instance.

1. **Whole batch paid to one tenant-level org; payout holds skipped.**
   On every rail, recipients, links and accruals are per account,
   `(tenant_id, account_id, rail_id)`, and accrue per batch line from
   `line_items_json` (section 4, "Recipients are per account, per rail",
   "Accrual"). Only the account can link its own recipient on a rail
   (section 5). Holds are checked per account and per rail (section 4,
   "Holds"). On `near_ai` the NEP-141 wallet hold label is deliberately not
   one of them, because it is about the wallet target (a default, section
   12); a held account's credit is never sent to anyone else on any rail.
2. **One-payout-path triggers race (check then insert, no shared lock).**
   There is no cross-rail exclusivity to race on: D6 and D8 make every rail
   parallel, so no rail reads or writes another's records (`near_ai` never
   touches the NEP-141 outbox), and there are no triggers. Inside each
   rail, exactly-once rests on unique keys written with `ON CONFLICT DO
   NOTHING` (accruals per rail), the rail's single-flight lock, one pending
   line per link, and the rail's send contract (section 3). If the owner
   ever makes two rails exclusive, the chosen rail must be recorded on the
   batch line under `SELECT ... FOR UPDATE` of the batch row, as the review
   proposed, not by triggers.
3. **Pull mode serves totals above the ceiling or not yet checked.** No
   pull mode and no signed statement on any rail: the commons only pushes.
   Every bound is checked per rail before a line is written `pending`
   (section 4, "Ceilings"); on `near_ai`, cloud-api also enforces the
   credential's caps under its own lock.
4. **No dry run in pull mode; kill switch does not reach serving.** Each
   rail has its own mode, `disabled` (the default), `dry_run` or `live`;
   `dry_run` makes no write call to the rail at all. Each rail's kill switch
   is a runtime row read before every send, not a boot setting (section 4,
   "Modes and the kill switch").
5. **Award period ignores policy, so a policy change reprices the period.**
   There is no period sum. On each rail each batch line is priced once, at
   that rail's epoch in force at the batch's time, and stored; a rate
   version cannot change its rate (section 4, "Rate", "Accrual").
6. **Reconcile can falsely freeze a ledger.** Reconciliation is a read-only
   drill, per rail, that never freezes or changes a row (section 8). Crash
   recovery runs inside each rail's single-flight pass, so no send and
   reconcile overlap on one rail.
7. **A read-key 403 freezes every ledger.** A credential fault maps to
   `StopPass`, which stops only that rail's pass and changes no row
   (section 3, "The interface"; section 4, "`near_ai` refusal mapping").
   It cannot freeze another rail, let alone every account.
8. **Accrual reads the recipient without a lock.** Routes only record
   requests; every link state change on a rail is made by that rail's pass
   under its lock, and accrual is not tied to a recipient at all (it is per
   account and rail; the link only decides where the program total goes)
   (section 4, "One pass, per rail").
9. **Award period per recipient, not per account.** Owed is per account and
   rail across all its links on that rail: the target subtracts every
   closed link's final total (section 4, "Program and link generation").
   Pricing is linear per line, so alternating recipients gains nothing.
10. **A recipient is checked once and then locked forever.**
    `recipient_index` is not unique, so no link can lock a recipient away
    from its owners on any rail. The account can unlink, and the operator
    has an audited release per rail (section 5, "Change and unlink"). On
    `near_ai`, re-checking the role before each send is not possible
    without the person's token; section 9 records the residual risk.
11. **Unfreeze and rebind leave no audit trail.** `trace_payout_audit` is
    append-only, carries the rail id, and gets a row for every request,
    confirmation, change, unlink, close, release, hold, halt and resume
    (section 4, "State and storage"). There is no unfreeze.
12. **HTTPS check bypassed with a userinfo URL.** No rail takes a base-URL
    setting. The `near_ai` host is a compiled-in constant shared with login
    introspection (section 2, step 6). A future rail's endpoint is
    compiled in the same way, or is a chain RPC named by its instrument
    descriptor, never an operator-typed URL.
13. **The "generic" ledger is hard-wired to NEAR AI.** The design now
    claims generality, and makes it testable. The rail interface (section
    3) carries no #1271 concept: no `recipient_cumulative`, no org, no
    nano-USD. Amounts are `u128` atomic units of the rail's own instrument
    with its own `decimals`; storage is `NUMERIC(39,0)` with a `rail_id` on
    every row; rates, locks, modes, kill switches, holds, ceilings,
    readiness and the drill are keyed by rail. What makes the claim real
    rather than asserted is the **second rail in tests** (section 8, step
    2a): amount-mode and idempotency-keyed instead of cumulative, 18
    decimals and an `erc20` instrument instead of `credit_account` and 9,
    driven by the same pass, storage and drill with no rail-specific
    branch, and run concurrently with a stopped rail. If adding that rail
    needs a change outside its own module and tests, the interface is
    wrong, and the PR that adds it must fix the interface rather than
    special-case the rail.
14. **Stale branch and citations.** Citations here are against
    `origin/main` f164d68f9, with the ingest binary at its new path
    (`crates/trace-commons-ingest`). The migration number is picked when
    the schema PR is written (section 4, "State and storage").

## 12. Still open, and defaults the owner may override

### Defaults (owner may override)

These are the design's current answers. The owner has not ruled on them;
each can change without reshaping the design.

1. **The NEAR wallet hold label does not block the `near_ai` rail**
   (section 4, "Accrual"). `none_enrolled` and `ambiguous_no_designation`
   are about a wallet target that `near_ai` does not use. A rail that pays
   a NEAR wallet would treat them as holds.
2. (Decided by the owner, 2026-10-21; moved to Owner decisions.) Credit
   settled to a principal with no account is paid once the principal is
   linked to an account, to the first account it is linked to.
3. **Reversed-after-paid credit is netted against the account's later
   credit on the same rail, never clawed back, and the exposure is bounded
   by that rail's ceilings** (section 4, "Reversals", "Ceilings"; section
   9).

### Still open

None of these is an owner decision about the design; each is a value or a
fact to get before build, or a later rail's design.

1. **Account ceiling** value, **retry tick** interval, and **credential
   expiry**.
2. **Staging API host:** `cloud-stg-api.near.ai` or `cloud-stg.near.ai`.
3. **Staging deployment flags** (`CREDIT_ALLOCATION_POLICY_VERSION`,
   `CREDIT_USAGE_ORDER`): unknown; ask NEAR AI.
4. **History filter.** No `payout_program` filter; recovery and the drill
   page all outgoing history from the newest record, so the drill costs
   links times history. Fine at pilot scale; ask NEAR AI for a filter
   before it is not.
5. **Restriction vs budget.** One error code covers both; we infer from
   `capacity`.
6. **Spending attribution.** Deferred in #1271.
7. **Later rails.** For a NEP-141 Trace Credit token rail: whether its link
   reuses the existing NEAR wallet designation or a new passkey-confirmed
   link, and how it answers `read_total` and `find_receipt` (a memo index,
   or our own outbox as the external record). For BAT: the price source
   for its rate epochs, and the recipient proof. Each needs its own short
   design against section 3 before it is built.
8. **Whether `trace_credit` moves onto the interface.** Today it is a rail
   in readiness only. Moving the NEP-141 outbox payout onto `PayoutRail`
   would be a separate change with its own migration of outbox state.
9. **Staging inference and attestation hosts.** Decided by D9: the
   staging build switches inference too. Still to get from NEAR AI: the
   staging inference host, and staging's gateway measurements and
   per-model receipt-signer keys for the pins. PR 4 switches only the
   sign-in origin and the server base; the rest lands with the daemon
   change (PR 7), with the staging status report.
