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
Add `resolve_principal_to_account(tenant_id, principal_ref) -> Option<Uuid>` (DB
method, RLS-scoped): the active link in `trace_account_principals`
(`unlinked_at IS NULL`) for that principal, else `None`. A principal with no active
account link resolves to `None` and continues to settle under its own
principal-derived key (no regression for raw, unmerged devices).

### Aggregation change (contributor credit view)
`read_contributor_credit_events_from_db` (`trace-commons-ingest.rs:46860`) currently
groups by the submission's `auth_principal_ref`. Change: resolve each event's
principal to its `account_id` (falling back to the principal string when unlinked)
and group the contributor credit view by the resolved key. An account with two
linked principals sums both.

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
   - **Move principals**: B's active `trace_account_principals` rows → `account_id = A`.
   - **Move authenticators**: B's active `trace_webauthn_credentials` and
     `trace_near_identities` → `account_id = A`. **Clear `payout_designated_at`** on
     moved NEAR identities (a merge must not create two payout destinations).
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

## Component 3 — Payout designation & the mockable settlement worker

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

### Settlement holds
A `Hold` outcome means the credit batch still finalizes internally, but **no NEAR
outbox row is created** for that account; the per-account `line_items_json` records
the account-keyed hash + a coarse hold reason, surfaced in the account's credit
view so the user can enroll/designate a payout. Money never targets a guessed
destination.

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
  ids; forced RLS + `trace_corpus_tenant_isolation`; registered in
  `TRACE_COMMONS_RLS_TABLES` + coverage arrays). **No** resolver grant.
- **Alter** `trace_near_identities`: add `payout_designated_at TIMESTAMPTZ` + the
  partial-unique payout index.
- **No** `trace_credit_ledger` change; **no** settlement-batch/outbox schema change
  (account-keying is a value change; holds live in `line_items_json`).

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
