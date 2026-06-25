# Slice 3b — Account Credit Consolidation, Device Merge & Mockable NEAR Settlement (Design Spec)

Date: 2026-06-24
Status: Approved for planning
Slice: 3b of the contributor-account feature
Depends on: Slice 1 (account core, session cookie, narrow `trace_login_resolver`
role, device-minted single-use login-link), Slice 2 (passkeys, ceremony store,
`resolve_account_ctx` auth middleware + rotation), and Slice 3a (login-with-NEAR,
`trace_near_identities`, the strong-authenticator gate). This branch
(`contributor-account-slice3b`) is stacked on the Slice 2/3a branch
(`contributor-account-slice2`).

## Context

Slice 3 was split into 3a (the authenticator layer — shipped) and 3b (this spec —
the principal/credit layer). Today contributor credit is attributed to an opaque
**principal** (a device's `auth_principal_ref`). An account can already own several
principals (multiple devices) and several authenticators (passkeys, NEAR). Slice 3b
makes credit **account-centric**:

1. **Credit re-keying** — aggregate and settle credit per *account* instead of per
   principal, by resolving principal → account dynamically through the existing link
   table (no ledger rewrite).
2. **Device-principal merge** — pull a second Ironclaw device's submission principal
   (and its historical credit) into your account, moving its authenticators and
   closing the absorbed account.
3. **Mockable NEAR settlement worker** — turn finalized per-account credit into the
   existing NEAR credit outbox, drive the `pending → submitted → confirmed` state
   machine through a mockable submitter trait (dry-run/disabled/http modes), and pay
   each account's credit to its **designated** NEAR identity. Real in-process NEAR
   transaction signing is explicitly **out of scope** (deferred to a future 3b-2);
   this slice builds the seam and the full off-chain-safe pipeline.

## Repo invariants this design must honor

- PostgreSQL-only; forced RLS via `trace_current_tenant_id()`.
- Hash-only / label-only audit and logs (no principal_refs, public keys,
  near_account_ids, raw tx hashes, or secrets in audit rows or log strings).
- Fail-closed with a safe missing-control name.
- Tenant scoping auth-derived; no client-supplied account/principal input beyond
  proven capabilities (a device-minted link, an authenticated session).
- Reuse Slice 1/2/3a patterns: the narrow resolver, `resolve_account_ctx`
  middleware, the strong-authenticator gate, hash-only audit, the
  no-`ensure_trace_tenant`-before-verification invariant, and the worker-route
  bearer-token pattern.

## Resolved decisions

- **Credit attaches to accounts by dynamic resolution, not a data rewrite.** The
  `trace_credit_ledger` stays principal-keyed at write time; the read/aggregation
  and settlement-grouping paths resolve principal → account at read time through
  `trace_account_principals`. This makes merge free and historical: re-pointing a
  principal automatically re-attributes its past credit. (No live traffic exists,
  so there is no migration either way — dynamic stays correct forever.)
- **Merge is two authenticated steps on the surviving account, gated and
  irreversible.** Redeeming device B's single-use login-link while authenticated to
  account A is realized as an authenticated `merge/start` (not an overload of the
  unauthenticated login-redeem path), which consumes the link as proof-of-control
  and stages a proposal; an explicit, strong-auth-gated `merge/confirm` executes it.
- **On merge, B's authenticators move to A and B closes.** Best "one identity" UX;
  irreversible.
- **Payout destination is an explicit per-account designation, fail-closed.** Auto
  only when exactly one active NEAR identity exists; otherwise hold (never guess
  where funds go).
- **The settlement submitter is a mockable trait with three explicit modes**,
  defaulting to `disabled` so no dry-run ever runs in production by accident.
- **No new external dependencies.** Reuse in-tree `ring`/`sha2`/`serde_json`/
  `reqwest` and the Slice 3a `borsh`/`bs58`.

## Component 1 — Credit re-keying (dynamic principal → account)

### Resolution helper
Add `resolve_principals_to_accounts(tenant_id, principal_refs: &[String]) ->
HashMap<String, Uuid>` (DB method, RLS-scoped): one **batched** query
(`SELECT principal_ref, account_id FROM trace_account_principals WHERE tenant_id =
trace_current_tenant_id() AND unlinked_at IS NULL AND principal_ref = ANY($1)`)
returning the resolution map for a whole settlement run / credit-view read in a
single round-trip — **not** a per-event lookup. A principal absent from the map (no
active account link) settles under its own principal-derived key (no regression for
raw, unmerged devices). A single-principal convenience wrapper may exist for the
merge path, but the batch paths (settlement grouping, contributor view) MUST use the
batched form to avoid N round-trips.

### Visibility change (contributor credit view)
**Correction (post-implementation discovery):** the contributor credit view is NOT
an owner-aggregate — it is a flat per-event list plus tenant-wide scalar sums, and
the only per-principal logic is a **visibility filter** (`can_access_credit_event`,
`trace-commons-ingest.rs:46490`, and the credit-handler filter ~62815-62889): a
device-principal-authenticated caller sees an event only if `event.auth_principal_ref
== auth.principal_ref`. The account-centric change is therefore a **visibility
broadening**, not a re-grouping: a caller sees credit for every principal resolved to
the caller's account (via `resolve_principals_to_accounts`); an unlinked caller still
sees only its own principal. Fail-closed: a resolution error denies (visibility never
widens on error). This realizes "your account aggregates credit across all your
devices" on the existing device-authenticated surface. (Settlement grouping below is
the true per-account aggregate.)

### Settlement grouping change
Settlement currently does (`trace-commons-ingest.rs:20010`):
```rust
grouped.entry(event.auth_principal_ref.clone())...
```
Change: resolve each event's principal to its account; group by the resolved key.
The per-account line item and NEAR outbox row become **account-keyed**:
`credit_account_hash = sha256_prefixed("account:" + account_id)` for linked
accounts, `sha256_prefixed(principal_ref)` for unlinked principals (unchanged path).

No `trace_credit_ledger` schema change.

## Component 2 — Device-principal merge

### `POST /v1/account/merge/start` (authenticated as A)
1. `ctx = Extension<AccountCtx>` (surviving account A).
2. Body `{ "merge_code": "<device-B login-link code>" }`. Resolve it via the same
   narrow resolver + the login-link lookup; require a **valid, unconsumed** link for
   a **different, open** account B (B != A; B not closed). Any failure → uniform
   reject (do not enumerate).
3. **Consume** the link (single-use; possession of a device-minted link = proof of
   control of B, same trust model as login).
4. Write a single-use **merge proposal** (`trace_account_merge_proposals`:
   surviving=A, absorbed=B, `absorbed_principal_count`, ~10-min `expires_at`).
5. Return the proposal `{ proposal_id, absorbed_principal_count, expires_at }` for
   explicit review. Nothing has merged yet.

`merge/start` requires a normal authenticated A session; the irreversible step is
gated below.

### `POST /v1/account/merge/confirm` (authenticated as A, strong-auth-gated)
1. `ctx`; apply `require_authenticator_change_allowed` (the Slice 3a strong-auth
   gate — merge is at least as sensitive as an authenticator change).
2. Body `{ "proposal_id" }`. Load the proposal; require unexpired, unconsumed, and
   `surviving_account_id == ctx.account_id`. Re-check B is still open. Any failure →
   reject.
3. Execute atomically in **one tenant tx** (RLS-scoped to A's tenant):
   - **Move principals**: `UPDATE trace_account_principals SET account_id = $A
     WHERE tenant_id = trace_current_tenant_id() AND account_id = $B AND
     unlinked_at IS NULL`. Note `account_id` is part of this table's PK
     `(tenant_id, account_id, principal_ref)`, so this is a **primary-key-column
     update** (legal in Postgres). It is **collision-free** because the separate
     `UNIQUE (tenant_id, principal_ref)` guarantees each principal_ref appears once
     per tenant — so A cannot already hold a row for any of B's principals, and the
     moved row's new `(tenant_id, A, principal_ref)` cannot duplicate an existing A
     row. (Do **not** delete+reinsert; a plain UPDATE is correct.)
   - **Move authenticators**: `UPDATE trace_webauthn_credentials SET account_id = $A
     …` and `UPDATE trace_near_identities SET account_id = $A …` for B's active rows.
     Here `account_id` is a **non-key column** (the PKs are `(tenant_id,
     credential_id)` / `(tenant_id, public_key)`, and credential_id / public_key are
     globally UNIQUE), so the move changes only `account_id` and **cannot** collide —
     A can never already hold the same credential. **Clear `payout_designated_at`** on
     moved NEAR identities in the same UPDATE (a merge must not create two payout
     destinations; leaving B's payout flag set would create two designated rows under
     A and violate the partial-unique payout index, aborting the tx).
   - **Revoke B's sessions**; set `B.closed_at = now()`.
   - Mark the proposal `consumed_at`.
   - Credit: nothing to do (dynamic resolution re-attributes B's history to A).
   - Audit `account_merged` (actor = A's `actor_ref`, hash-only; coarse counts only:
     principals_moved, authenticators_moved).
4. Return `{ "merged": true, "principals_moved", "authenticators_moved" }`.

### Guards
B==A → friendly no-op/reject; B already closed → reject; proposal expired / not
owned by A / already consumed → reject. The attacker bar: both a device-B-minted
link **and** a strong session on A. Irreversible by design (no undo endpoint).

**Accepted behaviors (conscious, not oversights):** (1) Expired-unconsumed merge
proposals are **not reaped** — they are tenant-scoped, single-use, and harmless;
confirm rejects on `expires_at`. (2) `merge/start` **consumes device B's single-use
link even if the user never confirms** — an abandoned merge burns that link with no
rollback (the proven authority lives in the proposal capability). This is acceptable:
device B simply mints another link. Both are documented so they are not mistaken for
gaps.

## Component 3 — Payout designation & the mockable settlement worker

**Which outbox.** This component drives the **settlement outbox `trace_near_credit_outbox`**
(migration V2: keyed by `(tenant_id, near_outbox_id)`, FK to
`trace_credit_settlement_batches`, rows built during settlement via
`NearCreditReceiptCall::settle`, driven by the `/v1/workers/near-credit-outbox/{submit,confirm}`
routes). It is **distinct** from `trace_near_credit_account_outbox` (V21, the
freeze/unfreeze account outbox keyed by `credit_hold_id`), which this slice does not
touch.

**Where the payout destination lives.** Routing a settlement to a designated
`near_account_id` requires that id on the row the worker submits. The settle receipt
is built from attestation hashes (not a NEAR account id), so this slice **adds a
`payout_near_account_id TEXT` column to `trace_near_credit_outbox`** (a public
on-chain identifier, consistent with how Slice 3a stores `near_account_id`; NOT an
audit field), set when the outbox row is created. The submitter reads it to target
the payout. (This corrects the earlier "no outbox schema change" intent — it is a
single additive column.)

### Payout designation
- `trace_near_identities` gains `payout_designated_at TIMESTAMPTZ` (NULL = not the
  payout) + a partial-unique index `(tenant_id, account_id) WHERE
  payout_designated_at IS NOT NULL AND revoked_at IS NULL` (≤1 active payout/account).
- A management action (extends the Slice 3a NEAR-management surface,
  strong-auth-gated) sets/clears the payout designation for one of the account's
  active NEAR identities. Surfaced in the NEAR-identities list (`is_payout`).
- `resolve_payout_near_account_id(tenant_id, account_id)`:
  - a designated active identity → `Designated(near_account_id)`;
  - else exactly one active identity → `SoleActive(near_account_id)`;
  - else → `Hold(NoneEnrolled)` (0 active) or `Hold(AmbiguousNoDesignation)` (>1, none designated).

### Settlement holds (and their recovery)
A `Hold` outcome means the credit batch still finalizes internally, but **no
`trace_near_credit_outbox` row is created** for that account; the per-account
`line_items_json` records the account-keyed hash + a coarse hold reason
(`none_enrolled` / `ambiguous_no_designation`), surfaced in the account's credit
view so the user can enroll/designate a payout. Money never targets a guessed
destination.

**Holds are recoverable, not lost.** The repo already has a "repair missing NEAR
credit outbox items for finalized settlement batches" path (`trace-commons-ingest.rs:19878`).
This slice extends that repair/sweep so that, once an account designates a payout,
its previously-held finalized line items get their `trace_near_credit_outbox` rows
created (status `pending`) on the next worker/repair run — no re-settlement needed.
A held line item is thus a deferred outbox row keyed off the same finalized batch,
recovered idempotently (the batch + account-hash UNIQUE prevents duplicates).

### The submitter trait
`NearSettlementSubmitter`:
```rust
async fn submit(&self, call: &NearCreditReceiptCall, idempotency_key: &str)
    -> Result<SubmitOutcome /* { near_transaction_hash } */>;
async fn confirm(&self, near_transaction_hash: &str) -> Result<ConfirmOutcome>;
```
Three explicit modes via `TRACE_COMMONS_NEAR_SETTLEMENT_MODE`, **default `disabled`**:
- `disabled` (default) → outbox rows are created but left unsubmitted; the worker
  no-ops. No dry-run ever runs in production by accident.
- `dry_run` → `DryRunSubmitter`: records the call, returns a **deterministic
  synthetic** `near_transaction_hash` (e.g. derived from the idempotency_key),
  advancing the full `pending → submitted → confirmed` state machine with **no
  network and no real funds**. This makes the worker end-to-end testable now.
- `http` → a thin adapter to an external signing service
  (`TRACE_COMMONS_NEAR_CREDIT_SUBMITTER_URL`) — the fill-in seam. Actual in-process
  NEAR tx-signing (issuer key custody, nonce, borsh tx, broadcast) is **deferred to
  a future 3b-2**.

**Reconciling with the existing per-request `dry_run` flag.** The submit/confirm
worker request bodies already carry a per-request `dry_run: bool`
(`trace-commons-ingest.rs:16830/16855`). The env mode is the **master switch that
selects the submitter implementation** (`disabled` → no submitter / worker no-ops;
`dry_run` → `DryRunSubmitter`; `http` → the http adapter). The request-level
`dry_run` remains a **preview**: it reports what *would* be submitted and **never
mutates outbox state** (never advances to submitted/confirmed), regardless of mode.
Precedence: env mode decides whether real submission is even possible; request
`dry_run=true` is always a non-mutating preview on top. This closes the
prod-safety gap — in a production `disabled`/`http` deployment, a request
`dry_run=true` cannot synthesize a confirmation, and `dry_run` *mode* must be set
deliberately (it is never the default).

### Worker behaviour
The existing worker routes (`/v1/workers/near-credit-outbox/{submit,confirm}`,
utility-operator gated) drive the trait. Idempotency / no double-submit: the worker
selects only `pending` rows and flips `pending → submitted` **atomically** (a
concurrent run cannot resend); the deterministic `idempotency_key` is recorded and
handed to the submitter so a real backend dedupes. `confirm` transitions
`submitted → confirmed`; failures record `last_error_hash` and mark `failed`. The
outbox call targets the resolved payout `near_account_id`.

## Data model summary (migrations)

- **New** `trace_account_merge_proposals` (tenant_id TEXT FK → trace_tenants
  ON DELETE CASCADE, proposal_id UUID, surviving_account_id UUID, absorbed_account_id
  UUID, absorbed_principal_count INT NOT NULL DEFAULT 0, created_at, expires_at,
  consumed_at; PK (tenant_id, proposal_id); FKs to trace_accounts for both account
  ids; forced RLS + `trace_corpus_tenant_isolation`; registered in the RLS table
  registry + the migration-policy coverage arrays — **confirm the exact registry
  symbol against the codebase** (Slice 3a registered `trace_near_identities` the same
  way; the constant name in this spec may be approximate)). **No** resolver grant.
- **Alter** `trace_near_identities`: add `payout_designated_at TIMESTAMPTZ` + the
  partial-unique payout index `(tenant_id, account_id) WHERE payout_designated_at IS
  NOT NULL AND revoked_at IS NULL`.
- **Alter** `trace_near_credit_outbox` (V2, the settlement outbox): add a single
  additive `payout_near_account_id TEXT` column (see Component 3).
- **No** `trace_credit_ledger` change (dynamic resolution); **no**
  settlement-batch schema change (account-keying is a value change; holds live in
  `line_items_json`).

## Error handling & security

- Fail-closed: invalid/spent/cross-account merge link → reject; payout
  ambiguous/absent → hold; settlement mode `disabled` → no submit.
- Merge executes in one atomic tenant tx; no partial merge is observable.
- Hash-only audit (`account_merged`, `account_payout_designated`, settlement
  events) — coarse only; no principal_refs / public keys / near_account_ids / raw
  secrets. The outbox keeps its existing `near_transaction_hash` + `last_error_hash`
  operational columns (operational state, not audit rows).
- Re-keying resolution is tenant-scoped under forced RLS (a principal resolves to an
  account only within its tenant).
- Moved authenticators authenticate to A afterward; B's sessions are revoked and B
  is closed (a moved passkey/NEAR identity's `account_id` is now A).

## Testing strategy

DB-backed (real PostgreSQL, `--test-threads=1`):
- **Merge**: start consumes B's link → proposal; confirm (strong session) moves
  principals + authenticators (B's passkey/NEAR identity then logs into A), closes
  B, revokes B's sessions; B's historical credit appears under A's credit view.
  Guards: weak-session confirm → 403; expired/replayed proposal → reject;
  cross-account proposal (not owned by A) → reject; B==A → no-op; B-already-closed →
  reject.
- **Re-keying**: an account with two linked principals sums both in the contributor
  credit view and in settlement grouping; an unlinked principal still settles under
  its own key.
- **Payout**: designate/clear; auto-at-sole; hold-at-zero; fail-closed-at->1-none;
  payout flag cleared on merge.
- **Worker**: `disabled` → no submit; `dry_run` → deterministic
  pending→submitted→confirmed; idempotency (a re-run does not double-submit); a held
  account → no outbox row; outbox rows are account-keyed.

## Residual risks (documented, accepted)

- Real in-process NEAR transaction signing is deferred to a future **3b-2**; `http`
  mode is a stub adapter. Pre-pilot operation uses `disabled` (safe) or `dry_run`
  (full pipeline, no funds).
- `dry_run` must never run in production; the `disabled` default enforces this.
- Merge is intentionally irreversible (no undo).
- Settlement to NEAR settling to *real* credit remains gated on a deployed credit
  contract + funded issuer account (3b-2), per the credit model's "as the model
  matures" framing.

## Out of scope (explicitly)

- In-process NEAR transaction signing / key custody / RPC broadcast (3b-2).
- A deployed on-chain credit contract or its final method ABI.
- Reversing/undoing a merge.
- Cross-tenant merges (a merge is within one tenant).
