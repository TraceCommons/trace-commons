# Slice 3b — Account Consolidation, Device Merge & Mockable NEAR Settlement Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make contributor credit account-centric — aggregate/settle per account via dynamic principal→account resolution, add a strong-auth-gated device-principal merge, and a mockable NEAR settlement worker that pays an account's credit to its designated NEAR identity.

**Architecture:** Credit stays principal-keyed in `trace_credit_ledger`; the read/settlement paths resolve principal→account dynamically through `trace_account_principals` (so merge is historical/free). A two-step authenticated merge (`merge/start` consumes device B's login-link as proof → strong-auth-gated `merge/confirm` moves B's principals + authenticators to A and closes B). A `NearSettlementSubmitter` trait with `disabled`(default)/`dry_run`/`http` modes drives the existing `trace_near_credit_outbox` state machine, paying a fail-closed, explicitly-designated payout NEAR identity. Real in-process NEAR tx-signing is deferred to a future 3b-2.

**Tech Stack:** Rust, axum 0.8, PostgreSQL (forced RLS), in-tree `ring`/`sha2`/`serde_json`/`reqwest`, Slice 3a `borsh`/`bs58`. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-06-24-contributor-account-consolidation-slice3b-design.md`.

**Repo gates (run before every commit; CI-enforced):**
```bash
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run
cargo clippy -p trace-commons-server --all-targets -- -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching
```
DB-backed tests need a throwaway PostgreSQL with a LOGIN role inheriting `trace_login_resolver` (`docs/operator/login-resolver-role.md`), `DATABASE_URL` + `TRACE_COMMONS_LOGIN_RESOLVER_DATABASE_URL` set, and run `--test-threads=1`. Every commit ends with `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>` and no emojis. Agents work only in this worktree (relative paths) and verify the main checkout shows only the pre-existing `community/*` + `AGENTS.md` after committing.

---

## File Structure

- `migrations/V34__account_consolidation.sql` — new `trace_account_merge_proposals` table; `trace_near_identities.payout_designated_at` + partial-unique index; `trace_near_credit_outbox.payout_near_account_id`.
- `crates/trace-commons-server/src/db/postgres.rs` — new DB methods (resolution, payout, merge ops, closed-account guard); RLS registration + V34 wiring + coverage arrays.
- `crates/trace-commons-server/src/db/mod.rs` — `Database` trait additions + result structs.
- `crates/trace-commons-server/src/bin/trace-commons-ingest.rs` — re-keyed credit view + settlement grouping; payout-designation handler; merge handlers + routes; settlement submitter trait + modes + worker wiring; hold-recovery in the repair path.
- `crates/trace-commons-server/src/account_settlement.rs` (new) — the `NearSettlementSubmitter` trait, `DryRunSubmitter`, `NearSettlementMode` config enum (kept out of the huge ingest binary; mirror how `account_near.rs` is a sibling module).
- `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs` — all DB-backed tests.
- `docs/operator/` — merge runbook + settlement-mode env var.

Mirror Slice 3a conventions throughout (V33 migration shape, resolver/DB-method patterns, handler + audit shapes, the strong-auth gate).

---

## Phase 1 — Data model + resolution foundation

### Task 1: V34 migration (merge proposals, payout column, outbox payout destination)

**Files:** Create `migrations/V34__account_consolidation.sql`; modify `db/postgres.rs`; test in `tests.rs`.

Mirror `migrations/V33__near_identities.sql` exactly for the RLS/forced-RLS/policy block conventions and the `run_migrations` wiring.

- [ ] **Step 1 — failing migration test** in `tests.rs` (mirror `near_identities_migration_applies_table_and_widens_client_kind`, DB-backed, self-skips without PG): assert `trace_account_merge_proposals` has `relforcerowsecurity=true`; assert `trace_near_identities` has column `payout_designated_at`; assert `trace_near_credit_outbox` has column `payout_near_account_id`; assert the partial-unique payout index exists (`pg_indexes` name match).
- [ ] **Step 2 — run, expect FAIL.**
- [ ] **Step 3 — write `V34__account_consolidation.sql`:**
  - `trace_account_merge_proposals(tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE, proposal_id UUID NOT NULL, surviving_account_id UUID NOT NULL, absorbed_account_id UUID NOT NULL, absorbed_principal_count INT NOT NULL DEFAULT 0, created_at TIMESTAMPTZ NOT NULL DEFAULT now(), expires_at TIMESTAMPTZ NOT NULL, consumed_at TIMESTAMPTZ, PRIMARY KEY (tenant_id, proposal_id), FOREIGN KEY (tenant_id, surviving_account_id) REFERENCES trace_accounts(tenant_id, account_id) ON DELETE CASCADE, FOREIGN KEY (tenant_id, absorbed_account_id) REFERENCES trace_accounts(tenant_id, account_id) ON DELETE CASCADE)` + ENABLE + FORCE ROW LEVEL SECURITY + the `trace_corpus_tenant_isolation` policy (copy V33's exact DROP/CREATE POLICY block). **No resolver grant** (authenticated-only).
  - `ALTER TABLE trace_near_identities ADD COLUMN payout_designated_at TIMESTAMPTZ;` + `CREATE UNIQUE INDEX IF NOT EXISTS idx_trace_near_identities_one_payout ON trace_near_identities (tenant_id, account_id) WHERE payout_designated_at IS NOT NULL AND revoked_at IS NULL;`
  - `ALTER TABLE trace_near_credit_outbox ADD COLUMN payout_near_account_id TEXT;`
  - Header comment in the V33 style.
- [ ] **Step 4 — `postgres.rs` wiring:** add `"trace_account_merge_proposals"` to `TRACE_COMMONS_RLS_TABLES` (~line 100). Add the V34 already-applied/`include_str!`/insert block after the V33 block (~line 976), version `34_i32`, name `"account_consolidation"`. Add `trace_account_merge_proposals` to BOTH coverage arrays in `trace_commons_rls_registry_matches_migration_policy_coverage` (~lines 2931-2954), adding V34 to the included-migrations lists. (Confirm the exact array/const names on the branch.)
- [ ] **Step 5 — run, expect PASS** (incl. the no-DB `trace_commons_rls_registry_matches_migration_policy_coverage`). Stand up throwaway PG; confirm V34 applies after V33.
- [ ] **Step 6 — gates + commit** `Add V34 account-consolidation migration: merge proposals, payout designation`.

### Task 2: `resolve_principals_to_accounts` (batched principal→account)

**Files:** `db/postgres.rs`, `db/mod.rs`, `tests.rs`.

- [ ] **Step 1 — failing test** (DB-backed, distinct tenant): create an account with 2 linked principals + a 3rd unlinked principal; `resolve_principals_to_accounts(tenant, &[p1,p2,p3,unknown])` returns a map `{p1→A, p2→A}` (p3 unlinked → absent; unknown → absent).
- [ ] **Step 2 — FAIL.**
- [ ] **Step 3 — implement** on `PgBackend` + `Database` trait (default not-implemented `Err`): `async fn resolve_principals_to_accounts(&self, tenant_id: &str, principal_refs: &[String]) -> Result<std::collections::HashMap<String, Uuid>, DatabaseError>` — one query under a tenant tx: `SELECT principal_ref, account_id FROM trace_account_principals WHERE tenant_id = trace_current_tenant_id() AND unlinked_at IS NULL AND principal_ref = ANY($1)`. Build the map. RLS-scoped; `ensure_trace_tenant` is appropriate (authenticated/internal path).
- [ ] **Step 4 — PASS.**
- [ ] **Step 5 — gates + commit** `Add batched principal-to-account resolution`.

---

## Phase 2 — Credit re-keying & payout resolution

> Ordering note (from plan review): the payout DB methods (Task 4) come **before** the settlement re-key (Task 5) so the settlement subagent has `resolve_payout_near_account_id` available and owns a single file-set. The payout *handler* is split out to Task 6.

### Task 3: Re-key the contributor credit view

**Files:** `trace-commons-ingest.rs` (~46860 `read_contributor_credit_events_from_db`), `tests.rs`.

The current path builds `owner_by_submission: BTreeMap<submission_id, auth_principal_ref>` and groups events by principal. Change the grouping key to the resolved account (fall back to the principal string when unlinked).

- [ ] **Step 0 — PRECONDITION (do not skip):** read the current `read_contributor_credit_events_from_db` response shape and record exactly what identifier it emits to the client today (raw principal_ref? a hash? nothing?). The re-key must NOT newly leak a raw principal_ref that wasn't exposed before — if the view emits a client-visible owner key, the account-keyed replacement must be `sha256("account:"+id)` (label-only), matching the prior leak posture.
- [ ] **Step 1 — failing test:** seed an account A with two principals each owning a submission with credit events; assert the contributor credit view for A sums BOTH submissions' credit under one account-keyed group. Seed a third unlinked principal with credit; assert it still appears under its own principal key. (Use the existing credit-view test helpers; if none, read events via the DB methods.)
- [ ] **Step 2 — FAIL.**
- [ ] **Step 3 — implement:** after loading the owner principals, call `resolve_principals_to_accounts(tenant, &distinct_principal_refs)` ONCE; map each event's owner principal to `account:{uuid}` when present, else the raw principal_ref; group by that resolved key. Keep the response shape; only the grouping key changes (honor the Step 0 leak posture).
- [ ] **Step 4 — PASS** (real PG).
- [ ] **Step 5 — gates + commit** `Re-key contributor credit view by resolved account`.

### Task 4: Payout designation DB ops + resolution

**Files:** `db/postgres.rs`, `db/mod.rs`, `tests.rs`. (DB layer only — the HTTP handler is Task 6.)

- [ ] **Step 1 — failing tests** (DB-backed): `designate_payout_near_identity(tenant, account_id, public_key)` sets `payout_designated_at` and clears any other active designation for the account (so ≤1 holds); `clear_payout_near_identity` unsets it; `resolve_payout_near_account_id` returns `Designated` when set, `SoleActive` when exactly one active identity and none designated, `Hold(NoneEnrolled)` at zero, `Hold(AmbiguousNoDesignation)` at >1 none-designated.
- [ ] **Step 2 — FAIL.**
- [ ] **Step 3 — implement** on `PgBackend` + `Database` trait: `designate_payout_near_identity` in one tenant tx (`UPDATE trace_near_identities SET payout_designated_at = NULL WHERE tenant_id = trace_current_tenant_id() AND account_id = $1 AND payout_designated_at IS NOT NULL` then `UPDATE ... SET payout_designated_at = now() WHERE ... AND public_key = $2 AND revoked_at IS NULL`; return `bool` affected → caller 404s if not owned/active). `clear_payout_near_identity`. `resolve_payout_near_account_id(tenant, account_id) -> PayoutResolution` (new enum in `db/mod.rs`: `Designated(String)|SoleActive(String)|Hold(PayoutHoldReason)` where `PayoutHoldReason = NoneEnrolled|AmbiguousNoDesignation`). Add an `is_payout` field (from `payout_designated_at IS NOT NULL`) to `NearIdentitySummary` (used by Task 6's list).
- [ ] **Step 4 — PASS** (real PG).
- [ ] **Step 5 — gates + commit** `Add NEAR payout designation DB ops with fail-closed resolution`.

### Task 5: Re-key settlement grouping + account-keyed outbox + payout/hold

**Files:** `trace-commons-ingest.rs` (~20010 settlement grouping; ~20051-20084 outbox build), `tests.rs`. Uses Task 4's `resolve_payout_near_account_id`.

- [ ] **Step 0 — PRECONDITION:** read the current outbox-build + `line_items_json` shape (~20051-20084) and confirm where `credit_account_hash` and the new `payout_near_account_id` land on the `TraceNearCreditOutboxItem` (Task 1 added the column; confirm the Rust struct + insert are updated to carry it).
- [ ] **Step 1 — failing test:** finalize a settlement for an account with 2 principals; assert ONE account-keyed line item summing both principals' micros, `credit_account_hash = sha256_prefixed("account:"+account_id)`, and (with a designated payout NEAR identity) an outbox row whose `payout_near_account_id` is the designated `near_account_id`. Assert an account with 0 NEAR identities → line item marked held (`none_enrolled`), NO outbox row. Assert an unlinked principal still settles under its own `sha256_prefixed(principal_ref)` key.
- [ ] **Step 2 — FAIL.**
- [ ] **Step 3 — implement:** before the grouping loop, batch-resolve all `selected_events` principals to accounts. Group by resolved key (`account:{uuid}` or raw principal). For each group: `credit_account_hash = sha256_prefixed(resolved_key)`. For account groups, call `resolve_payout_near_account_id(tenant, account_id)`:
  - `Designated|SoleActive(near_account_id)` → build the outbox row as today, additionally setting `payout_near_account_id = Some(near_account_id)`.
  - `Hold(reason)` → DO NOT push an outbox row; record `near_status = Disabled`/held + the coarse reason in the `line_items_json` line item.
  - Unlinked-principal groups settle as today (no payout lookup; existing behavior).
  Keep the `NearCreditReceipt`/`NearCreditReceiptCall::settle` build unchanged except the new outbox column.
- [ ] **Step 4 — PASS** (real PG).
- [ ] **Step 5 — gates + commit** `Group settlement by account and route outbox to designated payout`.

---

## Phase 3 — Payout management surface

### Task 6: Payout designation handler + is_payout in list

**Files:** `trace-commons-ingest.rs` (near-identity management ~13949-14091, routes ~5888-5946), `tests.rs`. Uses Task 4's DB methods.

- [ ] **Step 1 — failing tests** (DB-backed, HTTP): `PATCH /v1/account/near-identities/{public_key}/payout` (strong-auth-gated) designates payout; `GET /v1/account/near-identities` now returns `is_payout` per identity; clearing works. Cross-account: B can't designate A's identity (404). Weak session → 403.
- [ ] **Step 2 — FAIL.**
- [ ] **Step 3 — implement:** new handler `account_near_identity_designate_payout_handler` mirroring `account_near_identity_rename_handler`, **gated** with `require_authenticator_change_allowed` (changing where money goes is at least authenticator-sensitive); calls `designate_payout_near_identity`/`clear_payout_near_identity`; audit `account_payout_designated` (hash-only, `{}`). Surface `is_payout` in the list handler JSON. Route under `authenticated_account_routes`.
- [ ] **Step 4 — PASS** (real PG).
- [ ] **Step 5 — gates + commit** `Add NEAR payout designation endpoint`.

---

## Phase 4 — Device-principal merge

### Task 7: Merge DB operations (stage + execute, atomic)

**Files:** `db/postgres.rs`, `db/mod.rs`, `tests.rs`.

- [ ] **Step 1 — failing tests** (DB-backed): `stage_merge_proposal(tenant, surviving=A, merge_code_hash)` — resolves the login-link (reuse the `redeem_login_link` consume mechanics but WITHOUT issuing a session: consume the link, read its `account_id` = B), requires B open and B != A, inserts a `trace_account_merge_proposals` row (10-min expiry), returns `{proposal_id, absorbed_account_id, absorbed_principal_count}`. `execute_merge(tenant, surviving=A, proposal_id)` — atomic: moves B's active principals (`UPDATE trace_account_principals SET account_id=$A WHERE account_id=$B AND unlinked_at IS NULL` — PK-column update, collision-free per `UNIQUE(tenant,principal_ref)`), moves B's active passkeys + NEAR identities to A (`UPDATE ... SET account_id=$A` and, for near, also `payout_designated_at = NULL`), revokes B's sessions, sets `B.closed_at = now()`, marks the proposal `consumed_at`, returns `{principals_moved, authenticators_moved}`. Tests: round-trip (A's credit view then includes B's submissions' credit; B's passkey/NEAR identity now have account_id=A; B closed; B sessions revoked); B==A → reject; B already closed → reject; expired/already-consumed proposal → reject; proposal not owned by A → reject; merging an account whose NEAR identity was payout → A doesn't end up with two payout designations.
- [ ] **Step 2 — FAIL.**
- [ ] **Step 3 — implement** both methods on `PgBackend` + `Database` trait. `stage_merge_proposal` consumes the link via the same UPDATE pattern as `redeem_login_link` (atomic `consumed_at = now() WHERE code_hash=$ AND consumed_at IS NULL AND expires_at > now()` RETURNING account_id), then the open/!=A checks, then the proposal insert — all in one tenant tx. `execute_merge` does the five updates + audit `account_merged` (hash-only metadata `{principals_moved, authenticators_moved}`, actor = `account_actor_ref(&A)`) in one tenant tx. Use `RedeemAudit`-style hash-only audit. Fail-closed: any missing/invalid → `Err`/`Ok(None)`.
- [ ] **Step 4 — PASS** (real PG, `--test-threads=1`).
- [ ] **Step 5 — gates + commit** `Add device-principal merge DB operations`.

### Task 8: Merge handlers (start + strong-auth-gated confirm)

**Files:** `trace-commons-ingest.rs` (handlers + routes ~5888-5946), `tests.rs`.

- [ ] **Step 1 — failing tests** (DB-backed, HTTP): `POST /v1/account/merge/start` (authenticated as A) with device B's login-link code → 200 `{proposal_id, absorbed_principal_count, expires_at}` + a proposal row + B's link consumed. `POST /v1/account/merge/confirm {proposal_id}` from a STRONG session → 200 `{merged, principals_moved, authenticators_moved}`, B merged+closed. From a WEAK session → 403 (the gate). Unknown/expired proposal → reject; cross-account proposal → reject; a link for the SAME account (B==A) at start → reject.
- [ ] **Step 2 — FAIL.**
- [ ] **Step 3 — implement:** `account_merge_start_handler` (Extension<AccountCtx>): parse `{merge_code}`; `code_hash = hash_secret(...)` (mirror login-link hashing); `account_db(state).stage_merge_proposal(ctx.tenant_id, ctx.account_id.as_uuid(), &code_hash)`; uniform reject on any failure (don't enumerate). `account_merge_confirm_handler` (Extension<AccountCtx>): `require_authenticator_change_allowed(state, &ctx).await?`; parse `{proposal_id}`; `execute_merge(ctx.tenant_id, ctx.account_id.as_uuid(), proposal_id)`; map `Ok(None)`/not-owned → reject. Routes in `authenticated_account_routes`.
- [ ] **Step 4 — PASS** (real PG).
- [ ] **Step 5 — gates + commit** `Add merge start and strong-auth-gated confirm handlers`.

### Task 9: Defensive closed-account gating

**Files:** `db/postgres.rs` (`validate_session` SELECT ~1983 and/or `create_or_reuse_account` ~1719; `resolve_account_ctx` paths in ingest.rs), `tests.rs`.

Session validation does NOT currently exclude closed accounts. After merge, B's authenticators move to A so no session resolves to B — but add belt-and-suspenders so a closed account can never be acted under.

- [ ] **Step 1 — failing test:** close an account directly; assert a session/ctx that resolves to it is rejected (fail-closed) — e.g. `validate_session` returns `None` (or the ctx resolver denies) when the account's `closed_at IS NOT NULL`. Also assert `create_or_reuse_account` for a principal whose only link is to a closed account does NOT resurrect the closed account (mints fresh or follows the moved link). (Pick the minimal correct gate; document which path you gated.)
- [ ] **Step 2 — FAIL.**
- [ ] **Step 3 — implement:** add `AND a.closed_at IS NULL` (join `trace_accounts a`) to the session-validation/account-context resolution query so a closed account yields no valid context. Keep it minimal and additive; do NOT change rotation behavior.
- [ ] **Step 4 — PASS** (real PG; confirm passkey/NEAR/device login tests still green — moved authenticators authenticate to the OPEN account A).
- [ ] **Step 5 — gates + commit** `Reject sessions resolving to a closed account`.

---

## Phase 5 — Mockable settlement worker

### Task 10: Submitter trait + modes + worker wiring + hold recovery

**Files:** Create `crates/trace-commons-server/src/account_settlement.rs` (+ `pub mod account_settlement;` in `lib.rs`); modify `config.rs` (or the env-parse area in ingest.rs), `trace-commons-ingest.rs` (worker handlers ~21615-21681, the repair path ~20256), AppState + test constructors, `tests.rs`.

- [ ] **Step 1 — failing tests:** `NearSettlementMode::from_env` parses `disabled`(default)/`dry_run`/`http`. With mode `disabled`: the submit worker no-ops (rows stay `pending`/`disabled`, none → `submitted`). With mode `dry_run`: a `DryRunSubmitter.submit` returns a deterministic synthetic `near_transaction_hash` (e.g. `sha256_prefixed("dryrun:"+idempotency_key)` shaped to the hash column), the worker flips `pending → submitted`, and confirm flips `submitted → confirmed`; a re-run does NOT re-submit an already-`submitted` row (idempotency). A per-request `dry_run=true` under any mode is a non-mutating preview (no status change). **A submitter error → the row transitions to `failed` with `last_error_hash` set (hash-only, no raw error)** — assert this failure path explicitly. Hold-recovery: a finalized batch with a held line item (no outbox row) gets its outbox row created (status `pending`) by the repair path once the account has a designated payout — idempotent (the `UNIQUE(tenant, settlement_batch_id, credit_account_hash)` prevents dupes).
- [ ] **Step 2 — FAIL.**
- [ ] **Step 3 — implement:**
  - `account_settlement.rs`: `pub trait NearSettlementSubmitter { async fn submit(&self, call: &serde_json::Value, idempotency_key: &str) -> anyhow::Result<String /* near_tx_hash */>; }` (+ a confirm trait or reuse the existing `TraceNearCreditConfirmer`); `pub struct DryRunSubmitter;` impl returning the deterministic hash; `pub enum NearSettlementMode { Disabled, DryRun, Http }` + `from_env()` reading `TRACE_COMMONS_NEAR_SETTLEMENT_MODE` (default `Disabled`).
  - Wire mode into AppState (a `near_settlement_mode: NearSettlementMode` field; update ALL AppState constructors incl. the test one ~3022). The worker selects the submitter: `Disabled` → skip submission (no-op, return counts only); `DryRun` → `DryRunSubmitter`; `Http` → the existing `near_credit_submitter` adapter (the fill-in seam). Preserve the existing per-request `dry_run` as preview-only (never mutate state). Idempotency: only select `status = 'pending'` rows and flip to `submitted` in the same UPDATE that records the tx hash (a concurrent run can't double-submit).
  - Extend `repair_missing_near_credit_outbox_items_for_finalized_batches` (~20256) to also emit outbox rows for previously-held line items whose account now resolves a payout (`resolve_payout_near_account_id` no longer `Hold`), setting `payout_near_account_id`.
- [ ] **Step 4 — PASS** (real PG).
- [ ] **Step 5 — gates + commit** `Add mockable NEAR settlement submitter with disabled/dry_run/http modes`.

---

## Phase 6 — Sweep

### Task 11: Regression suite, operator docs, full gates

**Files:** `tests.rs` (consolidation), `docs/operator/*`, residual-risk notes.

- [ ] **Step 1 — confirm/consolidate** the Slice 3b security regression suite is present + GREEN on real PG: merge (round-trip, weak→403, expired/cross-account/B==A/B-closed guards, payout-flag-cleared-on-merge), re-keying (account sums principals; unlinked standalone), payout (designate/clear, auto-at-sole, hold-at-zero, fail-closed->1, cross-account 404), closed-account gating, settlement modes (disabled no-op, dry_run deterministic + idempotent, hold-recovery), V34 forced-RLS + coverage. Fill any gap. List with pass/fail.
- [ ] **Step 2 — operator docs:** `docs/operator/` — a merge runbook (the two-step flow, irreversibility, strong-auth gate, that an abandoned start burns device B's link); the `TRACE_COMMONS_NEAR_SETTLEMENT_MODE` env var (default `disabled`; `dry_run` never in prod; `http` = external signing adapter, real in-process signing deferred to 3b-2); payout designation; note the held-credit recovery path.
- [ ] **Step 3 — full sweep** (each command, report outcomes; note pre-existing unrelated failures verified against the base commit so they aren't attributed to this slice):
```bash
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run
cargo clippy -p trace-commons-server --all-targets -- -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching
cargo test -p trace-commons-server --test trace_corpus_storage_contract
bash scripts/operator/pilot-bootstrap-smoke.sh
# throwaway PG, resolver role provisioned: the account/passkey/near/session/gate/merge/settlement subset, --test-threads=1
```
- [ ] **Step 4 — commit** `Finalize Slice 3b: consolidation regression suite and operator docs`.

**Residual risks to record:** real in-process NEAR tx-signing deferred to 3b-2 (`http` is a stub adapter); `dry_run` must never run in prod (default `disabled`); merge is irreversible (no undo); on-chain settlement to *real* credit still gated on a deployed contract + funded issuer (3b-2).

---

## Notes for the executor
- Several spec items say "confirm the exact symbol on the branch" — verify `TRACE_COMMONS_RLS_TABLES`, the coverage-array names, and `validate_session`'s exact SELECT before editing.
- Keep the two `dry_run` notions distinct in code (the `NearSettlementMode::DryRun` config variant vs the per-request `dry_run` preview flag) — name them so they can't be conflated.
- Reuse, don't reinvent: the resolver/consume mechanics for the merge link mirror `redeem_login_link`; the management handler + audit shapes mirror the Slice 3a NEAR handlers; the gate is the Slice 3a `require_authenticator_change_allowed`.
- Do not unilaterally split `trace-commons-ingest.rs`; add the settlement trait in the new `account_settlement.rs` sibling module.
