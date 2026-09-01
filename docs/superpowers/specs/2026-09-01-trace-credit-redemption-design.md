# Redeeming Trace Credits against NEAR AI inference

Trace Credits were drafted as non-transferable and valueless. The decision of
2026-08-31 is that they carry value and are **redeemable by the holder alone** —
burned against that contributor's own NEAR AI inference — while remaining
non-transferable: no sending, no secondary market, no price.

This spec covers the burn path. The securities framing is settled and is not
re-argued here.

## What the code actually says today

| Claim | Evidence |
|---|---|
| Settlement is off on the pilot | `deploy/pilot-gcp/ingest.env.template:162` — `TRACE_COMMONS_NEAR_SETTLEMENT_MODE=disabled` |
| Acceptance credit is written `Pending` | `crates/trace-commons-server/src/bin/trace-commons-ingest.rs:55134-55149`, the sole `Pending` write site |
| Nothing ever moves a row to settled | `grep -rn "UPDATE trace_credit_ledger" crates/ --include='*.rs' \| wc -l` → **0**; same over `migrations/` → **0** |
| The ledger is already authoritative by design | `crates/trace-commons-server/src/near_credit.rs:1-6` — "The server keeps the off-chain settlement ledger authoritative" |
| Non-transferability is enforced at the payload layer | `near_credit.rs:296-310`, `ensure_non_transferable_method` — a four-name allowlist |
| The graded credit pipeline is shadow-mode | `crates/trace-commons-server/src/contributor_cap.rs:7` — "Shadow-only: nothing here settles or pays." |

### `settlement_state` is a label, not a lifecycle

There is no `UPDATE` against `trace_credit_ledger` anywhere in the repository.
`settlement_state` is fixed at INSERT and never revised — a write-once record of
*which code path minted the row*, not a stage a row progresses through.

The `Final` default in `mirror_credit_event_to_db` (`ingest.rs:57408-57418`) is
not a counterexample: its callers (`ingest.rs:22114`, `:22127`, `:37348`,
`:63078`) are the utility and ranking credit paths, which write `Final` at
insert time and are likewise never updated.

The pilot's 307 `pending` rows are not waiting for anything. That is the name of
the path they came from.

**Consequence: a spend-only-settled design is not conservative, it is inert.**
It ships a feature that can never fire and shows every contributor a permanent
zero. That is worse than shipping nothing.

## Where the balance lives: PostgreSQL, with the chain as receipt

Four reasons, in descending weight:

1. **The repo already declares this posture.** `near_credit.rs:1-6`: the
   contract "mirrors finalized settlement state". It mirrors; it does not
   decide. A chain-authoritative balance reverses a stated decision rather than
   extending it.
2. **Every control that must bind a spend is DB-side with no chain
   counterpart** — holds (`migrations/V2__trace_credit_settlement.sql:41-55`,
   enforced at `ingest.rs:23925`, `:25416-25441`), revocation clawback
   (`ingest.rs:56069-56198`), caps
   (`migrations/V41__trace_contributor_cap.sql:5-9`), quality (V39), dedup
   (V40). A chain-authoritative balance could be spent while Postgres says the
   account is frozen.
3. **RLS.** Forced tenant isolation through `trace_current_tenant_id()` is the
   entire access model. A chain balance has no tenant and cannot participate in
   it.
4. **Failure mode.** A debit requiring a NEAR round-trip fails when the network
   does. A Postgres transaction does not.

## The spendable figure, and why it is not `credit_points_pending`

Spendability must be defined independently of `settlement_state`. The obvious
substitute is the wrong number.

The whole graded pipeline is shadow-mode: quality `q` (V39), duplicate penalty
(V40) and the concave per-contributor cap (V41) are computed and stored, and
nothing pays on them. `credit_points_pending` — the figure contributors see
today — is the **ungraded, uncapped, un-deduped** number. Redeeming against it
disables every anti-farming control the credit pipeline exists to provide, in
one step, at the exact moment credit becomes worth farming.

```
spendable_micros =
    SUM over the account's active principal set, of credit events where:
      - settlement-eligible AND points_delta > 0
      - submission is Accepted and delayed-credit-eligible
      - the gate decision's contributor_factor_micros IS NOT NULL
      - occurred_at + MATURATION_WINDOW < now()
    each weighted by (q_micros / 1e6) * dup_pen * (contributor_factor_micros / 1e6)
  MINUS sum of burns
  MINUS sum of reversals
  MINUS clawback_deficit_micros
  , then hard-zeroed if any active credit hold exists on the account
```

Principal set is the active-link expansion from `trace_account_principals`
(`migrations/V30__trace_accounts.sql:44-64`), already the predicate used by the
credit read path (`ingest.rs:53112-53121`). `dup_pen` is `1/dedup_cluster_size`
exactly as `contributor_cap.rs:38-43` computes it. The hold predicate matches
`idx_trace_credit_holds_account` (`V2:57-58`).

Two clauses are load-bearing:

- **`contributor_factor_micros IS NOT NULL`.** The cap pass is batch-only,
  writing that column onto `trace_gate_decisions` when `recompute-contributor-caps`
  runs. Without this clause a later recompute can revise downward a balance that
  was already spent. The clause converts that race into a precondition: credit
  is simply not spendable until the cap pass has scored it.
- **`MATURATION_WINDOW`.** Credit must outlive the window in which it can still
  be revoked, or burn-and-abandon is free. See below.

**Promoting q, dup_pen and the cap out of shadow mode is a hard prerequisite,
and the largest single item of work.** Sequence it first.

## Identity: what NEAR AI may hold

The stable key is `account_id`, not `auth_principal_ref`.

`auth_principal_ref` is method-bound —
`method_bound_principal_ref(method, material)` at
`trace_upload_claim_issuer.rs:3070-3081` hashes the method in — so a contributor
with a device key, a passkey and a NEAR key has three of them. The union is
`trace_account_principals` (`V30__trace_accounts.sql:44-64`), and account-scoped
reads already expand it (`ingest.rs:14676-14685`, `:53112-53121`). A credit
account is `(tenant_id, account_id)`; its ledger footprint is the active
principal set.

**NEAR AI must not be trusted to assert that identity.** The tempting shortcut
is `trace_near_identities.near_account_id` (`V33__near_identities.sql:29`), but
that column's own comment calls it "the human-readable NEAR account label
(attribution only)". If debiting works by naming `alice.near`, then anyone who
compromises NEAR AI — or guesses a handle — can drain the balance behind it.

**Use a contributor-minted redemption grant**, presented alongside NEAR AI's own
worker credential:

- minted at the contributor's own account surface, strong-auth-gated, the way
  payout designation already is (`ingest.rs:17531`, and
  `docs/operator/settlement-mode.md` §"Designating payout");
- stored hash-only, `sha256:`-shaped, following the `trace_sessions.token_hash` /
  `trace_login_links.code_hash` convention (`V30__trace_accounts.sql:73`,
  `:105`) with the same `CHECK (… ~ '^sha256:[0-9a-f]{64}$')`;
- carrying an audience (`near-ai-inference`), an expiry, a `revoked_at`, and an
  optional per-grant spend ceiling. It is not a login session: it authorizes one
  verb, debit;
- revocable from the account surface without touching the account itself.

The debit route then requires **two independent credentials**. The worker token
proves who is asking; the grant proves whose credit may be spent. Neither alone
suffices, and the worker token alone must never be able to name an account and
spend it. A compromised NEAR AI can spend only what contributors individually
handed it, capped — not the register.

## The debit route

Patterned on `utility_credit_handler` (`ingest.rs:22169`) and its gate
`require_utility_operator` (`ingest.rs:52875-52884`), the closest existing
credit-mutating worker route.

### A new scope

Add `TokenRole::RedemptionWorker` to the enum (`ingest.rs:3076-3089`),
`"redemption_worker" | "redemption-worker"` to `TokenRole::parse`
(`:3091-3111`), and `"redemption_worker"` to `storage_name` (`:3131-3143`).

**Do not reuse the utility scope.** `utility_credit_handler` *issues* credit.
Issuance and redemption sharing one key means a compromised inference provider
can mint.

### Routes

Registered beside `/v1/workers/utility-credit` (`ingest.rs:7664`):

```
POST /v1/workers/redemption/quote
  {grant} -> {spendable_micros, expires_at}
POST /v1/workers/redemption/debit
  {grant, amount_micros, external_ref}
    -> {debited_micros, remaining_micros, redemption_id, skipped_existing}
POST /v1/workers/redemption/reverse
  {grant, external_ref}    # refunds one prior debit (failed inference)
```

Each handler opens as `utility_credit_handler` does
(`ingest.rs:22174-22175`): `authenticate_with_tenant_access_grant`, then
`require_redemption_operator`.

**Quote must not be an existence oracle.** Unknown, expired and revoked grants
return the same status as a valid grant on a zero balance. The repo already
holds this line for cross-account NEAR keys — "404 for cross-account — no
existence oracle" (`docs/operator/settlement-mode.md`). Otherwise anyone holding
the worker token can enumerate grants.

**Idempotency** reuses the established mechanism verbatim:
`deterministic_trace_uuid_for_external_ref("redemption-debit", &tenant.tenant_id,
submission_id, &external_ref)`, the same call the revocation reversal path uses
at `ingest.rs:56086-56091`. The response mirrors `utility_credit_handler`'s
`{appended, skipped_existing}` contract (`:22265-22284`): a replayed
`external_ref` returns the original result, never a second debit.

Single-phase debit plus a reversal route, **not** reserve/commit. Reserve/commit
needs an expiry sweeper, the pilot has no worker to run one, and an unswept
reservation table is a slow leak.

Hash-only per CLAUDE.md: a debit row stores `credit_account_hash`, `grant_hash`,
`external_ref_hash`, `amount_micros` and timestamps. Never the grant, the NEAR
account, the model slug, or the inference request.

### Burns cannot be ledger rows

`trace_credit_ledger.submission_id` is `UUID NOT NULL` with an FK to
`trace_submissions` and `ON DELETE CASCADE`
(`migrations/V1__trace_commons_schema.sql:302`, `:316-318`). A burn has no
originating submission, so it cannot be expressed in that table. The burn ledger
is a new migration, not a new event type — which is the right answer anyway,
since cascading a burn away with a deleted submission would be wrong.

## Storage and races

Enforcement is in the database, in both forms: a row lock for ordering and a
check constraint to make overdraw unrepresentable. The contract enforces nothing
here.

New tables (V49+), following `V30`/`V33` conventions — tenant FK first column,
`ENABLE` then `FORCE` RLS, `trace_current_tenant_id()` policy, `sha256` checks:

```sql
CREATE TABLE trace_credit_account_balances (
    tenant_id               TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    credit_account_ref      TEXT NOT NULL,
    credit_account_hash     TEXT NOT NULL CHECK (credit_account_hash ~ '^sha256:[0-9a-f]{64}$'),
    spendable_micros        BIGINT NOT NULL DEFAULT 0 CHECK (spendable_micros >= 0),
    burned_micros           BIGINT NOT NULL DEFAULT 0 CHECK (burned_micros >= 0),
    clawback_deficit_micros BIGINT NOT NULL DEFAULT 0 CHECK (clawback_deficit_micros >= 0),
    version                 BIGINT NOT NULL DEFAULT 0,
    updated_at              TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, credit_account_ref)
);
```

plus `trace_credit_redemptions` — `redemption_id`, `credit_account_hash`,
`grant_hash`, `external_ref_hash`, `amount_micros`, `status IN
('debited','reversed')`, `UNIQUE (tenant_id, external_ref_hash)`.

The debit transaction:

1. `SELECT … FROM trace_credit_account_balances WHERE (tenant_id,
   credit_account_ref) = … FOR UPDATE`
2. hold check in the same transaction against `trace_credit_holds` where
   `released_at IS NULL`; `idx_trace_credit_holds_account` (`V2:57-58`) is
   already shaped for it. Any hit returns 409.
3. `INSERT INTO trace_credit_redemptions … ON CONFLICT (tenant_id,
   external_ref_hash) DO NOTHING`; if nothing inserted, return the prior row.
4. `UPDATE … SET spendable_micros = spendable_micros - $amt, burned_micros =
   burned_micros + $amt, version = version + 1`

**Two concurrent debits** serialize on the lock from step 1; the second sees the
decremented value and either fits or 409s. If a bug in the balance derivation
lets an overdraw through anyway, `CHECK (spendable_micros >= 0)` aborts the
transaction. The lock gives ordering; the constraint makes overdraw
unrepresentable. Belt-and-braces is right here specifically because the
spendable read is a multi-table aggregate and therefore the likeliest future bug
site.

**A hold wins over a debit — but only if the hold path is amended.**
`credit_hold_handler` (`ingest.rs:25453-25500`) today inserts a hold with no
coordination against a balance row that does not yet exist. It must take the
same `FOR UPDATE` lock on the balance row before inserting, or a hold and a
debit interleave and the debit commits against a freshly-frozen account. This is
a required change to existing code, not new-table work, and it is the item most
likely to be dropped from a plan.

**Clawback** uses the same lock.
`reverse_credit_settlement_for_revocation_propagation`
(`ingest.rs:56069-56198`) already writes the negative ledger event
unconditionally (`:56136-56165`) and only skips the chain leg when no finalized
batch matches (`:56224` returns `Ok(None)`). So the DB-side reversal already
works for pending credit; extend it to decrement `spendable_micros` under the
same lock. The clawback must **not** be blocked by the `>= 0` check — the
shortfall goes to `clawback_deficit_micros`.

An on-chain check is the wrong place for any of this: it puts a network
round-trip inside the debit path and creates a second source of truth that can
disagree with Postgres, while every existing control already lives in Postgres.

## Revocation after a burn

The premise that non-transferability preserves clawback does not survive contact
with spending.

Burning is the one operation that makes credit irreversibly leave the system,
and it does so without any transfer occurring: the inference was computed, the
tokens emitted, the provider paid, the output consumed. Non-transferability
prevents credit reaching a third party; it does nothing about credit reaching
the *exit*. Redeemability is structurally a transfer to the inference provider —
one we authorize.

So there is nothing to claw back, and no design can invent one. The question is
only what happens to the shortfall.

**Negative carry (recommended).** The clawback decrements `spendable_micros`;
the portion that would drive it below zero lands in `clawback_deficit_micros`
instead of violating the check. Future earned credit pays the deficit down
before becoming spendable. It is purely local — no counterparty, no collection,
no agreement with anyone. It is honest: the row says the contributor over-drew.
It fails closed against repetition, because an account in deficit has zero
spendable and cannot burn again until it has re-earned. And it is small: a
`GREATEST(0, …)` plus the overflow, inside the transaction the reversal already
runs in.

**Charging it back to NEAR AI is rejected.** It makes the inference provider
carry our fraud risk, no such agreement exists, and it gives them a standing
interest in second-guessing our revocations.

**Write-off stays as an operator escape hatch** — an admin route to forgive a
deficit, hash-only audited like every other admin credit action. Some
revocations are our fault rather than the contributor's: a PII backstop finding
from a detector we later fixed, a policy change applied retroactively. Leaving
someone in debt for those is wrong.

### Negative carry alone does not close the attack

The exploit is burn-and-abandon: submit traces that clear the gate, redeem
immediately, abandon the account before review catches up. A deficit on an
abandoned account costs the attacker nothing. Deficit carry punishes the honest
contributor who has one trace revoked and does nothing whatsoever to the
adversary.

The only real defence is time, and it belongs in the design rather than bolted
on: credit is not spendable until it has outlived the window in which it could
still be revoked. `MATURATION_WINDOW` must exceed the realistic latency of the
PII backstop hold-and-rescrub queue, the quarantine review queue, and manual
revocation.

**That number is not set here, deliberately.** Those queues are known to run
long and at least one has no drain guarantee, so a window short enough to feel
responsive may not be long enough to be safe. Set it from measured pilot
latency. If the measurement shows the tail is effectively unbounded, maturation
alone cannot bound the risk and the fallback is a **per-epoch spend cap**
mirroring the per-epoch earn cap (`contributor_cap.rs:19-23`, K=25.0 per epoch,
7-day buckets), so worst-case loss per abandoned account is bounded by
construction rather than by timing.

## The NEAR contract

An append-only receipt log with a frozen-account flag. No transfer method, and
no balance arithmetic any external party can drive.

```
// existing, unchanged -- near_credit.rs:53-125
settle_credit_receipt(settlement_batch_id, credit_account_hash, policy_version,
                      source_list_hash, attestation_hash, amount_micros,
                      issuer_signature_hash)
reverse_credit_receipt(… identical shape …)
freeze_credit_account(credit_account_hash, reason_hash)
unfreeze_credit_account(credit_account_hash, reason_hash)

// new -- burn path
record_burn_receipt(redemption_id, credit_account_hash, amount_micros,
                    external_ref_hash, issuer_signature_hash)
reverse_burn_receipt(redemption_id, credit_account_hash, amount_micros,
                     external_ref_hash, issuer_signature_hash)

// views -- no method mutates another account's state
view balance_of(credit_account_hash) -> { settled_micros, burned_micros, frozen }
view receipt(id) -> Receipt | null
```

- **Issuer-only writes.** NEAR AI never touches the contract: it calls us, and
  we emit the receipt. This is what makes non-transferability true at the
  protocol level rather than by convention — there is no method a balance holder
  can call to move their own balance.
- **No `transfer`/`approve`/`ft_transfer`.** Extend
  `ensure_non_transferable_method` (`near_credit.rs:296-310`) with exactly the
  two new names. **Do not convert it to a denylist** — the allowlist is why an
  unrecognized method fails closed.
- **Idempotent by `redemption_id`**, consistent with `NearCreditReceiptCall::raw`
  deriving `idempotency_key` from a canonical sha256 of
  `contract_id + method + args` (`near_credit.rs:139-142`).
- **Hash-only args.** `ensure_hash_like` (`near_credit.rs:264-277`) already
  enforces the shape; wire the new methods into `validate_method_args`
  (`near_credit.rs:186-212`).
- **Burn receipts will arrive with no matching settle receipt.** Credit is
  spendable while pending, so `burned_micros` can exceed `settled_micros` on
  chain. Let it — the chain is an admittedly partial view. Deferring burn
  receipts until settlement lands creates a second queue with the same
  head-of-line starvation the PII backstop already exhibits.

## What blocks shipping

### Blocked on the three known missing pieces

Only the **chain receipt leg**: `record_burn_receipt` / `reverse_burn_receipt`
reaching the chain, and any claim that a burn is on-chain-attested.

Nothing else. That is the payoff of a ledger-authoritative design: the debit path
is one Postgres transaction. With `TRACE_COMMONS_NEAR_SETTLEMENT_MODE=disabled`,
burn receipts queue in an outbox exactly as credit receipts do today, and
redemption works end to end without a deployed contract, a funded key, or a
signing adapter.

### Blocked on design work in this repository

1. **Grade the spendable figure** — promote q, dup_pen and the cap out of shadow
   mode (`contributor_cap.rs:1-7`). Largest item; sequence first.
2. **Set `MATURATION_WINDOW`** from measured revocation latency, and decide
   whether a per-epoch spend cap is also needed. Owner decision, needs data.
3. **Balance and burn-ledger migration** (V49+), plus a one-time backfill over
   the existing 307 events. `run_migrations` is hand-rolled — wire the new
   migrations in explicitly.
4. **Lock ordering in `credit_hold_handler`** (`ingest.rs:25453`).
5. **Clawback overflow into `clawback_deficit_micros`**, wired into
   `ingest.rs:56069`, plus the admin forgive route.
6. **Redemption grant lifecycle** — mint, list and revoke on the account
   surface, strong-auth-gated like payout designation. A slice of its own.
7. **A redemption drill** under `/v1/admin/*-drill`, producing hash-only
   evidence and wired into the rollout-smoke required checks. Coverage: mint,
   quote, debit, replay-is-idempotent, concurrent-debit-rejected,
   hold-blocks-debit, revoke-after-burn-produces-deficit. The last two would
   otherwise ship untested, because neither has an existing analog to copy.
8. **Contributor-facing disclosure.** `settlement_posture_explanation` maps the
   deployment's settlement mode to one line on the receipt. Redemption needs the
   equivalent: credit is spendable-when-matured, clawback-able, and a revocation
   after spending leaves a deficit. Shipping without it repeats the exact defect
   #445 was filed for — a receipt describing a posture the deployment is not in.
9. **Decide `spendable`'s place in the contributor credit response.**
   `TraceCommonsTenantCreditResponse` (`ingest.rs:69521-69540`) already carries
   nine credit figures. Adding a tenth without deciding whether it replaces the
   headline will have contributors reading the wrong number. A product call, but
   it blocks the UI.

## Open items

- **Cross-tenant balance versus cross-tenant cap.** The per-contributor cap is
  computed cross-tenant per `auth_principal_ref`, while the balance table is
  keyed per tenant. Verify the two agree before shipping.
- **Whether NEAR AI will accept a third-party bearer grant** in its inference
  path at all. The design assumes it; that assumption is unconfirmed and is
  worth settling before building the grant lifecycle.
- **Reversal reconciliation is undesigned.** `POST /v1/workers/redemption/reverse`
  refunds a debit for a failed inference, but nothing reconciles our view of
  what was reversed against NEAR AI's.
- **Item 9 above** is a product decision, not an engineering one.
