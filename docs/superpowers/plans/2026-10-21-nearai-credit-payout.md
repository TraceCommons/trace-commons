# Parallel Payout Rails (NEAR AI first) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Pay every settled Trace Credit award out on N parallel external rails, each exactly-once, capped, haltable and reconcilable on its own, and ship the first external rail: NEAR AI inference credit to an org the contributor links from the app's signed-in session (D5's passkey superseded 2026-10-23).

**Architecture:** A `PayoutRail` contract in `trace-commons-gate-api` (rail id, instrument with decimals, rate, recipient-link kind, exactly-once send keyed by line, reads, preflight, refusal mapping). One set of `trace_payout_*` tables keyed by `rail_id`. A per-rail payout pass in `trace-commons-server` (own advisory lock, mode, kill switch, ceilings, readiness) that accrues from finalized settlement batch lines and sends through the rail. The NEAR AI rail wraps a #1271 transfer client in cumulative mode. Ingest gains link routes on the signed-in account session with a server-side NEAR AI ownership check, the per-rail worker, readiness and a reconciliation drill. The contributor daemon lists owned NEAR AI orgs on the device and starts a link; the native macOS app shows the org picker. A keyed, 18-decimal test rail proves the contract is not #1271-shaped.

**Tech Stack:** Rust (gate-api, gate-enclave, server on axum + PostgreSQL, ingest, protocol, contributor daemon on tokio), C ABI (`trace-commons-contributor-ffi`), Swift / SwiftUI (`macos/`).

**Spec:** `docs/superpowers/specs/2026-10-21-nearai-credit-payout-design.md` (draft PR #1532). Owner decisions D1-D8 are settled; do not re-litigate them. Section 12's three defaults are the current answers; build them, and keep each in one place so an owner override is a small change. Every implementer reads the spec sections their task cites.

## Owner decisions after the plan was written

- **2026-10-23, D5 superseded:** "We aren't using passkeys at all in the
  app right now." Link, change and unlink are authorized by the
  contributor's signed-in app session only. No passkey gate, no step-up
  page section, and nothing may require the account to have a passkey. The
  server still verifies org membership with NEAR AI (`/users/me`, the
  contributor's NEAR AI token used for that check and never stored), keeps
  the org-changed-since-shown check, and audits every link, change and
  unlink. Every passkey step below (the goal line, Review Focus 7, Task 6's
  `require_passkey_step_up` and step-up page section, Task 7's step-up
  redirect, Task 8's passkey copy) is replaced by this when that task is
  built; this note governs where they disagree. Task 6 is built this way (see its section), and so is task 7: the daemon calls the confirm route itself once the person confirms in the app, with no step-up URL. Task 8 follows it.
- **2026-10-23, payout source:** `TRACE_COMMONS_NEAR_AI_PAYOUT_SOURCE` is
  `payment`, `grant` or `staking_farm`, required in `dry_run`/`live`, with
  no default (the staging sender is funded with paid credit, so staging
  runs `payment`). Built in task 5, which also tightened the staging render
  to accept only an `oc_` plus 64 hex payout credential.
- **2026-10-23, task 5 seam:** the pass opens a link's sealed recipient
  through `PayoutRecipientOpener`. Task 5 ships `UnconfiguredRecipientOpener`
  (opens nothing, so any link is held `payout_link_unreadable`); task 6,
  which seals recipients, supplies the real opener (`SealedRecipientOpener`)
  and refuses the start with a rail enabled and no recipient key.

## Global Constraints

Repo and process rules (every task):

- Each task is one PR on its own branch, stacked on the previous task's branch (table below). Create each worktree with `git -C $CODE/trace-commons-server worktree add -b <branch> $CODE/tc-wt-<branch> <base>`. Never work in, or read the working tree of, the shared checkout `$CODE/trace-commons-server`. Never `git stash`; never reset or check out away uncommitted work. In the commands below `CODE` is the directory that holds the shared checkout and `W` is the task's worktree.
- `gh` always takes `--repo TraceCommons/trace-commons`. Open each PR as a draft; never merge, never mark ready. PR body from `.github/pull_request_template.md`, the MIT/Apache inbound-licensing box ticked, last line `🤖 Generated with [Claude Code](https://claude.com/claude-code)`.
- TDD: write the listed tests first, run them, see them fail for the stated reason, then implement.
- Commits: short imperative subject, no prefix, no emoji, last line `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Run `cargo fmt --all` before each commit.
- Verification uses the CI forms, and the report pastes the real `test result:` lines. Never claim green from truncated or filtered output. Every Rust task runs, at minimum:
  ```bash
  cargo fmt --all -- --check
  RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins
  RUSTFLAGS="-D warnings" cargo check -p trace-commons-ingest --bins
  cargo clippy --workspace --all-targets -- -D warnings -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching
  RUSTFLAGS="-D warnings" cargo test --workspace --no-run
  ```
  plus the task's targeted tests and the full suite of each touched crate. For a touched permissive crate also run `RUSTFLAGS="-D warnings" cargo check -p <crate> --no-default-features`.
- PostgreSQL tests: a fresh database per run, on the literal host `127.0.0.1`, single-threaded:
  ```bash
  DB=admission_test_payout_$(date +%s); createdb -h 127.0.0.1 $DB
  export TRACE_COMMONS_PG_TEST_DATABASE_URL=postgres://$USER@127.0.0.1:5432/$DB
  export TRACE_COMMONS_LOGIN_RESOLVER_DATABASE_URL=postgres://tc_login_resolver_login@127.0.0.1:5432/$DB
  ```
  A pg test that self-skips proves nothing: check the run printed no skip line for the new tests. Every new pg test file or `--lib` pg test must be added to the `postgres-suites` job in `.github/workflows/ci.yml` (or it never runs in CI), and ingest bin pg tests run under `ingest-bin-postgres` automatically.
- Known environmental failures to ignore (compare against the baseline, do not chase): `an_unentitled_process_reports_one`, `managed_sign_in_redeem_hosts_the_proxy_and_ends_it`, `opencode_starts_on_the_sign_in_without_a_model`, `every_shipped_table_is_owner_reviewed`; load flakes `near_wallet*`, `contextual_entropy_stays_bounded*` (rerun alone).
- Licence split: gate-api, gate-enclave, server and ingest are AGPL; every new `.rs` file in them starts with
  `// Copyright (C) 2026 K&Z Partners LLC` and `// SPDX-License-Identifier: AGPL-3.0-or-later`.
  Protocol, contributor and ffi are permissive and never gain an AGPL dependency. `crates/trace-commons-server/tests/license_boundary.rs` stays unchanged.
- No new dependency. `reqwest` 0.12, `secrecy` 0.10, `async-trait`, `uuid`, `sha2`, `serde` are already present where needed. If a task seems to need a new crate, stop and report.
- If `Cargo.toml` changes (a feature, never a dependency), run all four `cargo deny` licence checks from `CLAUDE.md`.
- Public repo: synthetic values only. Never a real org id, credential, rate, ceiling, pilot value, personal path or the owner's data in code, fixtures, commits or PR text. Before each push: `git -C $W diff origin/main...HEAD | grep -nEi 'oc_[0-9a-f]{8}|/Users/|[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}'` and review every hit (synthetic UUIDs in tests are fine; say so in the report), and grep commit messages the same way.
- Hash-only logging and audit (`docs/operator/hash-only-logging.md`): rail id, labels, `sha256:` hashes and amounts only. Never a recipient, org id or name, token, memo, metadata, tenant or account id, or rate.
- Fail closed: an unknown rail id, a missing setting for a rail in `dry_run`/`live`, a missing rate, a sealed recipient that will not open: refuse with a safe label. Never fall back.
- RLS forced on every new table, tenant predicates through `trace_current_tenant_id()`. Cross-tenant reads only through the narrow enumeration role with `SET ROLE` (precedent: `migrations/V101__unbound_account_reaper.sql`, `docs/operator/register-stats-role.md`). Never `BYPASSRLS`.
- **Do not touch** (D6 and spec section 1): `trace_credit_ledger`, `trace_credit_settlement_batches` rows, `trace_near_credit_outbox`, `NearPayoutAdapter`, `process_payouts`, `dispatch_near_settlements`, `SettlementAdapter`. The pass only reads finalized batches and ledger reversal rows. The only edit to settlement code is the post-commit notify in task 5.
- Rail id: the NEAR AI rail is `near_ai`, never `near` (that label is the NEP-141 outbox rail's in readiness).
- User-facing words follow `docs/ui-copy-guide.md`; every new string is doc-commented `DRAFT, NEEDS APPROVAL` until the owner signs it off. Native macOS is the lead client; no Tauri, GTK or Windows work in this plan.
- The C ABI header exists twice (`crates/trace-commons-contributor-ffi/include/trace_commons.h`, `macos/Sources/CTraceCommons/include/trace_commons.h`); edit both and `diff` them to identical. Before any `swift test`, run `cargo build -p trace-commons-contributor-ffi`.

## PR Stack

| # | Branch | Base | Scope |
|---|---|---|---|
| 1 | `payout-rail-interface` | `main` | `PayoutRail` contract, `RailId`, rail types; keyed 18-decimal mock rail |
| 2 | `payout-rail-schema` | `payout-rail-interface` | migration: links, link requests, accruals, lines, holds, audit, rate epochs, control, enumerator role; RLS lists; store trait + pg impl skeleton |
| 3 | `payout-rail-accrual` | `payout-rail-schema` | rate sources, rate epochs at boot, accrual of credit and reversal rows, per rail; no network |
| 4 | `payout-rail-near-ai-client` | `payout-rail-accrual` | #1271 client, `near_ai` rail, refusal mapping, shared compiled-in host, `near-ai-staging` feature |
| 5 | `payout-rail-pass` | `payout-rail-near-ai-client` | per-rail pass: lock, owed, send, recovery, close, ceilings, holds, modes (default `disabled`), halt/resume, notify, tick, readiness |
| 6 | `payout-rail-link-routes` | `payout-rail-pass` | link request/confirm/status routes on the signed-in session (D5 superseded), `/users/me` verifier, liveness probe, recipient seal and opener, protocol types |
| 7 | `payout-rail-daemon-link` | `payout-rail-link-routes` | daemon org listing (on device), link/confirm/unlink IPC on the signed-in session, ingest client, core copy keys and their C ABI export |
| 8 | `payout-rail-macos-picker` | `payout-rail-daemon-link` | macOS org picker, status, consent copy surfaces (copy marked for owner review) |
| 9 | `payout-rail-reconcile-drill` | `payout-rail-macos-picker` | per-rail reconciliation drill wired into rollout-smoke evidence |
| 10 | `payout-rail-runbook` | `payout-rail-reconcile-drill` | operator runbook, README index, staging test steps |

Tasks 4 and 9 depend only on tasks 1-3 and 1-5 respectively in code; they stay in the stack so each PR's diff is its own, but a reviewer can take them out of order.

## Review Focus

1. **One rail's fault never reaches another.** Two rails, one `StopPass` (or halted, or a held account) and one healthy, in the same pass tick, on the same account and settlement: the healthy rail's line is `confirmed`, the stopped rail's rows are unchanged. Test: `a_stopped_rail_does_not_block_another` (task 5).
2. **Exactly-once across a crash on each rail shape.** Kill between "line pending" and "outcome recorded", rerun: one external effect on the cumulative rail and on the keyed rail. Tests: `crash_after_pending_sends_once_on_near_ai`, `crash_after_pending_sends_once_on_keyed_rail` (task 5).
3. **No double pay across relinks.** Change org twice, then pay: total sent across programs equals owed. Test: `relinking_pays_owed_once_across_programs` (task 5).
4. **A credential 403 changes no row** on a read and on a send. Test: `a_credential_refusal_stops_the_rail_and_changes_no_row` (task 5).
5. **Precision.** `u128` amounts near `10^38` round-trip through `NUMERIC(39,0)`; a `near_ai` target above `i64::MAX` holds as `payout_amount_out_of_range`, never wraps. Tests: `atomic_units_round_trip_numeric_39` (task 2), `near_ai_target_above_i64_holds` (task 4).
6. **Rate immutability.** A rate version re-installed with a different rate refuses boot. Test: `a_changed_rate_under_one_version_refuses_boot` (task 3).
7. ~~**The passkey gate has no carve-out.**~~ Superseded 2026-10-23 (see "Owner decisions after the plan was written"): no passkey; the signed-in session authorizes a link, the server verifies org membership, and every change is audited. Tests in task 6.

---

### Task 1: Rail interface and types (`payout-rail-interface`, base `main`)

Spec: section 3 ("The interface", "Rail ids", "What makes the interface real").

**Files:**
- Create: `crates/trace-commons-gate-api/src/payout_rail.rs`
- Modify: `crates/trace-commons-gate-api/src/lib.rs` (`pub mod payout_rail;`, re-exports beside `settlement`, and `pub use async_trait::async_trait;` so gate-enclave can implement the trait without a new dependency: gate-enclave has `uuid` and `sha2` but not `async-trait`)
- Create: `crates/trace-commons-gate-enclave/src/payout_rail.rs` (`MockKeyedPayoutRail`, a test double like `MockPerplexityScorer`; uses `trace_commons_gate_api::async_trait`)
- Modify: `crates/trace-commons-gate-enclave/src/lib.rs`

**Interfaces (all `pub`, in `trace_commons_gate_api::payout_rail`):**
- `struct RailId(String)` with `new` (same rule as `InstrumentId`: `is_safe_identifier`), `as_str`, serde `try_from = "String"`; const `NEAR_AI_RAIL_ID: &str = "near_ai"`, `TRACE_CREDIT_RAIL_ID: &str = "trace_credit"`; `RailId::new("near")` is accepted by the rule but `RailId::near_ai()` is the only constructor the server uses for NEAR AI.
- `struct RecipientKind(String)` (safe label); const `NEAR_AI_ORG_RECIPIENT_KIND = "near_ai_org"`.
- `struct RailDescriptor { rail_id: RailId, instrument_id: InstrumentId, instrument: InstrumentDescriptor, recipient_kind: RecipientKind, wire_max: Option<AtomicUnits> }` with `validate()` (calls `InstrumentDescriptor::validate`).
- `struct RailRate { version: String, numerator: u128, denominator: u128 }` with `new` refusing `denominator == 0`, `numerator == 0`, or an unsafe `version` label; `fn price(&self, award_units: u128) -> Result<u128, ContractError>` = `floor(award * num / den)`, checked (`AtomicUnitOverflow` on overflow; use `u128` widening via `checked_mul`, and if it overflows, refuse rather than approximate).
- `struct RecipientRef` (opaque bytes/string, `Debug` prints `RecipientRef(..)`, never the value).
- `struct RailLine<'a> { line_ref_hash: &'a str, recipient: &'a RecipientRef, program: &'a str, target_total: AtomicUnits, delta: AtomicUnits, confirmed_total: AtomicUnits }`.
- `enum SendOutcome { Committed { receipt_hash: String, delta: AtomicUnits, total_after: Option<AtomicUnits> }, AlreadyAtOrAbove { total: AtomicUnits } }`.
- `enum ReceiptLookup { Found { receipt_hash: String, delta: AtomicUnits }, NotFound }`.
- `struct RailCapacity { sending_enabled: bool, stop_label: Option<&'static str>, per_send_max: AtomicUnits, sends_remaining: Option<u32> }`.
- `struct RailHistoryEntry { receipt_hash: String, program: String, delta: AtomicUnits, total_after: Option<AtomicUnits> }`.
- `enum RailRefusal { Retry(&'static str), StopPass(&'static str), HoldLine(&'static str), RecipientUnavailable, Failed(&'static str) }` with `label()`.
- `#[async_trait] trait PayoutRail: Send + Sync { fn descriptor(&self) -> &RailDescriptor; fn adapter_identity(&self) -> &str; fn production_qualified(&self) -> bool { false } fn program_for_link(&self, link_id: Uuid) -> String; async fn preflight(&self) -> Result<RailCapacity, RailRefusal>; async fn read_total(&self, recipient: &RecipientRef, program: &str) -> Result<AtomicUnits, RailRefusal>; async fn send(&self, line: &RailLine<'_>) -> Result<SendOutcome, RailRefusal>; async fn find_receipt(&self, line: &RailLine<'_>, exclude: &[String]) -> Result<ReceiptLookup, RailRefusal>; async fn history(&self, recipient: &RecipientRef, program: &str) -> Result<Vec<RailHistoryEntry>, RailRefusal>; async fn probe_recipient(&self, recipient: &RecipientRef) -> Result<(), RailRefusal>; }`. The trait doc states the contract verbatim from spec section 3: a repeated `send` for one line never repeats the effect.
- `MockKeyedPayoutRail` (gate-enclave): rail id `test_keyed`, instrument `test_token` kind `Erc20`, network `"1"`, a synthetic lowercase `0x` address, `decimals: 18`; in-memory ledger keyed by `line_ref_hash`; `send` applies `delta` once per key and returns the first receipt on a repeat; counters `effects()`; knobs to return a chosen `RailRefusal` from the next call or to "lose" a response after committing (commit, then return `Retry("lost_response")`). `production_qualified()` false. Never constructed outside tests (task 5 asserts the server's rail registry cannot build it from config).

- [ ] **Step 1: Capture the baseline (once for the whole plan; later tasks compare against it)**

```bash
W=$CODE/tc-wt-payout-rail-interface
cd $W
cargo test -p trace-commons-gate-api 2>&1 | grep 'test result:'
cargo test -p trace-commons-gate-enclave 2>&1 | grep 'test result:'
cargo test -p trace-commons-protocol 2>&1 | grep 'test result:'
cargo test -p trace-commons-contributor 2>&1 | grep 'test result:'
RUSTFLAGS="-D warnings" cargo test --workspace --no-run 2>&1 | tail -3
cargo test -p trace-commons-server --lib 2>&1 | grep 'test result:'
```

Record every line. Do not pipe through `head`.

- [ ] **Step 2: Write the failing tests** in `payout_rail.rs` and gate-enclave's `payout_rail.rs`

```rust
#[test] fn rail_id_follows_the_instrument_id_rule()        // "near_ai" ok; "" / "Near" / 65 chars refused
#[test] fn rail_rate_prices_with_floor_and_checks_overflow() // (num 3, den 2) prices 5 -> 7; u128::MAX * 2 refuses AtomicUnitOverflow
#[test] fn rail_rate_refuses_zero_terms_and_unsafe_versions()
#[test] fn a_descriptor_with_bad_decimals_is_refused()      // decimals 39 -> InvalidInstrumentDescriptor
#[test] fn recipient_ref_debug_never_prints_the_value()
#[test] fn refusal_labels_are_safe_identifiers()            // every variant's label passes is_safe_identifier
#[tokio::test] async fn keyed_mock_repeated_send_has_one_effect()
#[tokio::test] async fn keyed_mock_lost_response_is_found_by_receipt_lookup()
#[tokio::test] async fn keyed_mock_carries_amounts_above_u64()  // delta 10^30 -> read_total == 10^30
```

- [ ] **Step 3: Run to verify they fail** — `cargo test -p trace-commons-gate-api payout_rail; cargo test -p trace-commons-gate-enclave payout_rail`. Expected: compile failure (module missing).

- [ ] **Step 4: Implement** the types and trait, then the mock.

- [ ] **Step 5: Verify** — the targeted tests pass; gate-api and gate-enclave totals = baseline + new; the global check list passes; `tests/license_boundary.rs` passes unchanged (`cargo test -p trace-commons-server --test license_boundary`).

- [ ] **Step 6: Commit** — `Add the payout rail contract and a keyed test rail`.

**Done when:** the contract compiles with no #1271 term in it (`grep -nE 'cumulative|nano|organization' crates/trace-commons-gate-api/src/payout_rail.rs` returns only doc text that says the contract is rail-neutral), the mock meets the contract in its own tests, and every global check is green with output pasted.

---

### Task 2: Schema (`payout-rail-schema`, base `payout-rail-interface`)

Spec: section 4 ("State and storage", "Accrual", "Modes and the kill switch"), section 5 ("Storage").

**Files:**
- Create: `migrations/V<N>__payout_rails.sql`. Pick `<N>` at implementation time: `git -C $W ls-tree --name-only origin/main migrations/ | sed -E 's#migrations/V([0-9]+)__.*#\1#' | sort -n | tail -1`, plus one; also check open PRs (`for n in $(gh pr list --repo TraceCommons/trace-commons --state open --json number --jq '.[].number'); do gh pr diff $n --repo TraceCommons/trace-commons --name-only | grep '^migrations/'; done`). On 2026-10-21 that gives **V131**. Never V129.
- Modify: `crates/trace-commons-server/src/db/postgres.rs` (`TRACE_COMMONS_RLS_TABLES`, line ~188: add every new table)
- Modify: `crates/trace-commons-server/tests/trace_corpus_pg_rls.rs` (`expected_trace_rls_tables()`, line ~184)
- Modify: any other expected-table list that fails after the migration (grep `TRACE_COMMONS_RLS_TABLES` users: `src/versioned_pipeline_product.rs`, `src/db/postgres/pipeline_upgrade_tests.rs`, ingest `pipeline_restore_pg_tests.rs`) only where they enumerate all tables
- Create: `crates/trace-commons-server/src/db/postgres_payout.rs` (store functions, tenant-scoped)
- Modify: `crates/trace-commons-server/src/db/mod.rs` (`PayoutStore` trait and row types)
- Create: `crates/trace-commons-server/tests/payout_rails_pg.rs`
- Modify: `.github/workflows/ci.yml` (`postgres-suites`: add `cargo test -p trace-commons-server --test payout_rails_pg -- --test-threads=1`)

**Schema (exactly the spec's columns; all amounts `NUMERIC(39,0)` with `CHECK (x >= 0)` except signed accrual columns):**
- `trace_payout_links`, `trace_payout_link_requests`, `trace_payout_accruals`, `trace_payout_lines`, `trace_payout_holds`, `trace_payout_audit` (tenant tables: `ENABLE` + `FORCE ROW LEVEL SECURITY`, policy `USING (tenant_id = trace_current_tenant_id()) WITH CHECK (...)`, copied from `V79__inference_connection.sql`).
- Partial unique indexes: links one `active` / one `waiting` / one `closing` per `(tenant_id, account_id, rail_id)`; lines one `pending` per `link_id`; lines `(rail_id, receipt_hash) WHERE receipt_hash IS NOT NULL`; accruals `(tenant_id, rail_id, settlement_batch_id, credit_account_hash) WHERE kind = 'credit'` (accruals carry nullable `account_id`, set once; index `(tenant_id, rail_id, account_id)` for `owed`) and `(tenant_id, rail_id, reversal_event_id) WHERE kind = 'reversal'`. No unique index on `recipient_index`.
- `CHECK` constraints on every enum-like text column (`state`, `kind`, `rail_id ~ '^[a-z0-9_.-]{1,64}$'`).
- `trace_payout_audit`: no `UPDATE`/`DELETE` grant to the runtime role (append-only).
- `trace_payout_rate_epochs (rail_id, version, numerator, denominator, pricing_label, observed_at DEFAULT clock_timestamp())`, PK `(rail_id, version)`; `trace_payout_control (rail_id PK, halted, label, updated_at)`. Forced RLS, read-all policy, writes only via role `trace_payout_config_writer` (assumed with `SET ROLE` by the boot path and the admin routes).
- Role `trace_payout_enumerator`: `GRANT SELECT (tenant_id, rail_id, state) ON trace_payout_links`, permissive `SELECT` policy for that role only, as V101 does. Nothing else.

**Store interface (`db/mod.rs`, implemented in `postgres_payout.rs`):** tenant-scoped functions taking an auth-derived tenant: `insert_accruals` (`ON CONFLICT DO NOTHING`, returns inserted count), `owed(account, rail)`, `closed_links_final_paid(account, rail)`, `live_link`, `insert_pending_line`, `resolve_line`, `pending_line_for_link`, link state transitions (each writes an audit row in the same transaction), `open_hold`/`clear_hold`, `insert_link_request`/`take_link_request`; non-tenant: `record_rate_epoch` (under `trace_payout_config_writer`), `rate_epochs(rail)`, `control(rail)`/`set_control`, `tenants_with_live_links(rail)` (under `SET ROLE trace_payout_enumerator`, `RESET ROLE` on every path).

- [ ] **Step 1: Write the failing pg tests** in `tests/payout_rails_pg.rs`

```rust
#[tokio::test] async fn every_payout_table_forces_rls()                    // pg_class.relforcerowsecurity for all 8
#[tokio::test] async fn a_tenant_cannot_read_another_tenants_links_or_lines()
#[tokio::test] async fn at_most_one_active_waiting_and_closing_link_per_account_and_rail() // a second active on near_ai refused; an active on test_keyed beside it accepted
#[tokio::test] async fn one_pending_line_per_link()
#[tokio::test] async fn one_receipt_answers_one_line_per_rail()
#[tokio::test] async fn accrual_inserts_are_idempotent_per_rail()           // same batch line twice -> one row; same line on another rail -> second row
#[tokio::test] async fn atomic_units_round_trip_numeric_39()               // 10^38 - 1 and u64::MAX + 1 read back exactly
#[tokio::test] async fn the_enumerator_role_sees_only_tenant_rail_and_state() // SELECT recipient_index under the role -> permission denied
#[tokio::test] async fn rate_epochs_and_control_are_not_writable_by_the_runtime_role() // connect AS the runtime role, never the migration/superuser URL: an owner or superuser ignores REVOKE and the test passes vacuously
#[tokio::test] async fn audit_rows_cannot_be_updated_or_deleted()                // same: as the runtime role
#[tokio::test] async fn two_accounts_link_the_same_recipient_index()        // D7: no uniqueness
```

- [ ] **Step 2: Run to verify they fail** — `cargo test -p trace-commons-server --test payout_rails_pg -- --test-threads=1` with the pg env set. Expected: missing relations.
- [ ] **Step 3: Implement** the migration and store; update both RLS lists.
- [ ] **Step 4: Verify** — the new suite passes against PostgreSQL (no skip line); `cargo test -p trace-commons-server --test trace_corpus_pg_rls -- --test-threads=1` passes; `cargo test -p trace-commons-server --test migration_atomicity_pg -- --test-threads=1` passes; the whole ingest bin pg run per `CLAUDE.md` shows no regression against the baseline; global checks.
- [ ] **Step 5: Commit** — `Add the payout rail tables, roles and store`.

**Done when:** the migration applies on a fresh database, every new table is in both RLS lists, the 11 pg tests pass against a real database (output pasted), CI's `postgres-suites` runs the new file, and nothing outside the store writes the tables yet.

---

### Task 3: Rate epochs and accrual (`payout-rail-accrual`, base `payout-rail-schema`)

Spec: section 3 ("Rate source"), section 4 ("Rate", "Accrual", "Reversals"), section 12 default 1, and the 2026-10-21 owner decision on pre-account credit.

**Files:**
- Create: `crates/trace-commons-server/src/payout_rate.rs` (`trait RailRateSource { fn current(&self) -> Option<RailRate> }`; `SettlementTableRate` built from `trace_commons_protocol::settlement_estimate::SettlementEstimateTable`: numerator `usd_micros_per_credit * 1000`, denominator `atomic_units_per_credit`, version = table `version`, pricing label = `pricing_version`; `None` when the rate is absent or zero)
- Create: `crates/trace-commons-server/src/payout_accrual.rs` (`accrue_rail(store, tenant, rail_id, epochs) -> AccrualReport`)
- Modify: `crates/trace-commons-server/src/lib.rs`
- Modify: `crates/trace-commons-ingest/src/bin/trace-commons-ingest.rs` (boot: after the settlement table validates, record the `near_ai` epoch when its mode is not `disabled`; refusal label `payout_rate_version_conflict`)
- Modify: `crates/trace-commons-ingest/src/bin/trace_commons_ingest_internal/credit_estimate.rs` (expose the loaded table to the boot hook; no behaviour change)
- Tests: `crates/trace-commons-server/tests/payout_rails_pg.rs` (extend), unit tests in both new modules

**Rules to implement (no network in this task):**
- Epoch for a batch = latest `observed_at <= batch.created_at` on that rail, else the first epoch; no epoch, no accrual on that rail.
- Credit lines: finalized batches only; `settled_credit_delta_micros > 0`; account-keyed and bare-principal lines both accrue (owner decision 2026-10-21). `account_id` on the accrual = `settlement_key_account_id(credit_account_ref)` when `Some`, else NULL. Each pass then claims unowned accruals: for each NULL-owner accrual whose principal now has a `trace_account_principals` row, `UPDATE trace_payout_accruals SET account_id = $1 WHERE id = $2 AND account_id IS NULL` plus an audit row. Never reassign a set owner. `near_payout_hold_reason` is not read (default 1; a comment cites spec section 12).
- Reversal rows: `trace_credit_ledger` rows with `settlement_state = 'reversed'` and `external_ref` prefix `revocation_credit_reversal:`; locate the finalized line whose `source_credit_event_ids` contains the event; price at the version that line's credit accrual used on this rail; negate. Unlocatable for a settled event: open hold `payout_reversal_unpriced` on that rail only.
- Amounts are `u128` end to end; `i64` micros convert with a checked cast.

- [ ] **Step 1: Write the failing tests**

```rust
// payout_rate.rs
#[test] fn settlement_table_rate_maps_to_a_rational()        // synthetic usd 2_000_000, units 1_000_000 -> num 2_000_000_000, den 1_000_000
#[test] fn a_table_without_a_positive_rate_yields_no_epoch()
// payout_rails_pg.rs
#[tokio::test] async fn a_changed_rate_under_one_version_refuses_boot()
#[tokio::test] async fn epochs_cannot_be_backdated()          // observed_at comes from the database clock
#[tokio::test] async fn a_batch_is_priced_at_the_epoch_in_force_at_its_time()
#[tokio::test] async fn a_batch_older_than_the_first_epoch_uses_the_first()
#[tokio::test] async fn only_finalized_positive_lines_accrue() // dry_run, failed, zero delta -> nothing
#[tokio::test] async fn a_bare_principal_line_accrues_unowned_and_is_claimed_when_linked()
#[tokio::test] async fn a_claimed_accrual_never_moves_to_a_second_account()
#[tokio::test] async fn a_near_wallet_hold_label_does_not_stop_near_ai_accrual()
#[tokio::test] async fn a_reversal_takes_back_exactly_its_credit_on_each_rail()
#[tokio::test] async fn an_unlocatable_reversal_holds_only_that_rail()
#[tokio::test] async fn rails_accrue_the_same_line_independently() // near_ai and test_keyed (rate 10^12/1) each get one row at their own rate and decimals
#[tokio::test] async fn rerunning_accrual_writes_nothing_new()
```

- [ ] **Step 2: Run to verify they fail.** **Step 3: Implement.** **Step 4: Verify** (targeted, server lib, `payout_rails_pg`, ingest bin pg run, global checks). **Step 5: Commit** — `Price settled credit per payout rail at its rate epoch`.

**Done when:** accrual is a pure function of stored rows per rail, boot refuses a re-priced version, the two defaults are each one named function with a spec citation, all tests pass against PostgreSQL with output pasted, and no network call exists in the diff.

---

### Task 4: The #1271 client and the `near_ai` rail (`payout-rail-near-ai-client`, base `payout-rail-accrual`)

Spec: section 2 step 6, section 4 ("The NEAR AI client", "`near_ai` refusal mapping", "Fail-closed rules"), section 8 ("Known staging gap").

**Files:**
- Create: `crates/trace-commons-server/src/near_ai_api.rs` (the one compiled-in base: `pub const NEAR_AI_API_BASE_URL`, `https://cloud-api.near.ai/v1` by default; under `feature = "near-ai-staging"` `https://cloud-stg-api.near.ai/v1` (the default in cloud-api's own scripts; spec section 12 item 2 says to confirm it, so the PR body flags it), written as a constant in this file only; `pub fn near_ai_staging() -> bool`, which readiness reports. No `compile_error!`: the staging run needs an optimized build with the feature on, so the refusal is the release-workflow check below, not the `--release` profile)
- Modify: `crates/trace-commons-ingest/src/bin/trace_commons_ingest_internal/near_provisioning.rs` (drop the local `NEAR_AI_API_BASE_URL`, use the server constant; keep the `cfg(test)` override exactly as it is)
- Create: `crates/trace-commons-server/src/near_ai_transfer.rs` (`NearAiTransferClient`: `SecretString` credential, sender org, source type; `send_cumulative`, `capacity`, `history_page`; `redirect(Policy::none())`, TLS-or-loopback, fixed user agent, 15 s timeout, `bearer_auth`; error body `error.type` into a closed `NearAiTransferError` enum; a `#[cfg(test)]`/test-only constructor taking a loopback base)
- Create: `crates/trace-commons-server/src/near_ai_rail.rs` (`NearAiRail: PayoutRail`; program `tc1-` + first 32 hex of `sha256("nearai-payout-program-v1:" || link_id)`; probe program `tcprobe-v1`; metadata `{"tc": sha256("nearai-payout-line-v1:" || line_id)}`; memo empty; `receipt_hash = sha256("nearai-transfer-v1:" || id)`; `target_total` narrowed to `i64` or `HoldLine("payout_amount_out_of_range")`; the refusal table from the spec, verbatim)
- Create: `crates/trace-commons-server/src/near_ai_rail_config.rs` (env parsing: `TRACE_COMMONS_NEAR_AI_PAYOUT_MODE` default `disabled`, `_CREDENTIAL` (`oc_` + 64 hex, `SecretString`), `_CREDENTIAL_ID`, `_SENDER_ORG` (UUID), `_SOURCE` (`payment`|`grant`|`staking_farm`, no default; widened 2026-10-23), `_ACCOUNT_CEILING_NANO`; any missing or malformed in `dry_run`/`live` -> `near_ai_payout_config_missing`; no base-URL variable exists)
- Modify: `crates/trace-commons-server/Cargo.toml`, `crates/trace-commons-ingest/Cargo.toml` (feature `near-ai-staging`, ingest forwards to server and contributor where it links them), `crates/trace-commons-contributor/Cargo.toml` (feature `near-ai-staging` switching `CLOUD_API_ORIGIN` in `src/daemon/nearai_credential/api.rs`; the daemon's status reports it)
- Modify: `scripts/ci/check-release-credential-store.sh` or add `scripts/ci/check-release-staging-feature.sh` beside it (same shape, with its own self-test), and call it from the `cargo fmt --check` job in `.github/workflows/ci.yml`, so no release workflow or shipped manifest enables `near-ai-staging`
- Create: `crates/trace-commons-server/tests/near_ai_rail_mock.rs` (a loopback mock #1271 server built on axum, which the server already depends on)

- [ ] **Step 1: Write the failing tests** (mock server)

```rust
#[tokio::test] async fn a_positive_delta_commits_and_hashes_the_transfer_id()
#[tokio::test] async fn a_zero_delta_is_already_at_or_above()
#[tokio::test] async fn read_total_is_a_zero_cumulative_send()
#[tokio::test] async fn find_receipt_pages_history_and_skips_claimed_receipts() // stops on a short page though a cursor is returned
#[tokio::test] async fn each_reply_maps_to_its_refusal()   // one case per spec table row, incl. 401 and both 403s -> StopPass
#[tokio::test] async fn near_ai_target_above_i64_holds()
#[tokio::test] async fn redirects_are_not_followed()
#[tokio::test] async fn the_body_never_carries_a_memo_and_metadata_is_one_hash()
#[test] fn the_program_is_derived_from_the_link_id_and_fits_100_bytes()
#[test] fn config_refuses_missing_or_malformed_values_outside_disabled()
#[test] fn config_has_no_base_url_setting()                 // grep-style: the env list has no *_URL/*_HOST key
#[test] fn logs_carry_no_org_id_token_or_rate()             // capture tracing output across a send; assert none of the synthetic secrets appear
```

- [ ] **Step 2: Run to verify they fail.** **Step 3: Implement.**
- [ ] **Step 4: Verify** — targeted tests; global checks; additionally `RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --features near-ai-staging`, the same for `-p trace-commons-ingest` and `-p trace-commons-contributor`; the staging-feature check script passes and its self-test (which seeds a release workflow and a shipped manifest enabling the feature) shows each seeded violation refused (paste both); the four `cargo deny` licence runs (Cargo.toml changed).
- [ ] **Step 5: Commit** — `Add the NEAR AI transfer client and its payout rail`.

**Done when:** the rail meets the task 1 contract against the mock, every spec table row has a test, the host is one compiled-in constant shared with login introspection, no release workflow or shipped manifest can enable the staging feature (shown by the self-test, not asserted), and every check is green with output pasted.

---

### Task 5: The payout pass (`payout-rail-pass`, base `payout-rail-near-ai-client`)

Spec: section 3 ("Independence"), section 4 ("Holds", "Ceilings", "Modes and the kill switch", "One pass, per rail", "Crash recovery, per rail", "Fail-closed rules"), section 5 ("Change and unlink"), section 7.

**Files:**
- Create: `crates/trace-commons-server/src/payout_pass.rs` (`PayoutPass { rail: Arc<dyn PayoutRail>, store, mode, ceiling }`, `run_once() -> PassReport`; lock key `hashtextextended('payout-pass-v1:' || rail_id, 0)` on a pinned connection, released on every path; steps 0-5 of the spec; closing by reads only; dry-run count-only report and no `send`/`read_total`/`find_receipt` calls)
- Create: `crates/trace-commons-server/src/payout_worker.rs` (one task per enabled rail, one `tokio::sync::Notify` per rail, run-again flag, 60 s tick constant `PAYOUT_RETRY_TICK`; `PayoutRails` registry built from config, which can construct `near_ai` only)
- Modify: `crates/trace-commons-server/src/versioned_pipeline.rs` (`finalize_pending_credit_batch`, line ~16181: after commit, `notify_all()` on an optional `PayoutNotifier` handle; no other change) and the ingest settlement route (after its commit)
- Modify: `crates/trace-commons-ingest/src/bin/trace-commons-ingest.rs` (`AppState.payout_rails`, boot, worker spawn, admin routes `POST /v1/admin/payout-rails/{rail_id}/halt|resume` and `.../links/{link_id_hash}/release` copying an existing admin-bearer handler's auth shape, readiness: add one `payout_rails` entry per rail to `GET /v1/pipeline/readiness` with `rail_id`, `mode`, `halted`, `rate_epoch`, `qualified`, `trace_credit` listed with mode `internal`)
- Modify: `crates/trace-commons-ingest/src/bin/trace_commons_ingest_internal/tests.rs` (every `test_state` literal gains the new `AppState` fields)
- Create: `crates/trace-commons-server/tests/payout_pass_pg.rs` (PostgreSQL + mock #1271 + `MockKeyedPayoutRail`); add it to CI `postgres-suites`. Link routes arrive in task 6, so these tests create links and confirmations through the task 2 store functions directly.

- [ ] **Step 1: Write the failing tests** (`payout_pass_pg.rs` unless noted)

```rust
#[tokio::test] async fn a_linked_account_is_paid_owed_and_an_unlinked_one_accrues_only()
#[tokio::test] async fn crash_after_pending_sends_once_on_near_ai()     // drop the pass after step 2 (test hook), rerun: one 201 at the mock
#[tokio::test] async fn crash_after_pending_sends_once_on_keyed_rail()  // MockKeyedPayoutRail lost-response knob; effects() == 1
#[tokio::test] async fn recovery_without_a_matching_receipt_holds_the_line()
#[tokio::test] async fn a_stopped_rail_does_not_block_another()          // near_ai mock answers 403 credential_refused; keyed rail pays the same account; near_ai rows unchanged
#[tokio::test] async fn a_halted_rail_does_not_block_another()
#[tokio::test] async fn a_credential_refusal_stops_the_rail_and_changes_no_row() // on read_total and on send
#[tokio::test] async fn back_pay_steps_up_by_the_per_send_max()
#[tokio::test] async fn a_target_over_the_account_ceiling_holds_and_sends_nothing()
#[tokio::test] async fn a_credit_hold_or_closed_account_stops_sends_on_every_rail()
#[tokio::test] async fn a_link_with_a_pending_line_gets_no_new_line()
#[tokio::test] async fn closing_resolves_pending_lines_by_reads_only_then_activates_the_successor()
#[tokio::test] async fn closing_after_a_404_completes_from_history()
#[tokio::test] async fn relinking_pays_owed_once_across_programs()
#[tokio::test] async fn netting_after_a_reversal_sends_nothing_until_new_credit_exceeds_the_gap()
#[tokio::test] async fn dry_run_writes_no_line_and_makes_no_write_call()
#[tokio::test] async fn a_second_pass_on_one_rail_is_refused_while_the_first_holds_the_lock()
#[tokio::test] async fn passes_on_two_rails_run_concurrently()
#[tokio::test] async fn a_notify_during_a_pass_causes_exactly_one_more()
#[tokio::test] async fn the_kill_switch_stops_before_the_next_send()
#[test] fn the_registry_cannot_build_a_mock_or_unknown_rail_from_config()
#[test] fn every_mode_defaults_to_disabled()
// ingest tests.rs
#[tokio::test] async fn readiness_reports_each_rail_separately()
#[tokio::test] async fn halt_and_resume_require_the_admin_bearer_and_write_audit_rows()
```

- [ ] **Step 2: Run to verify they fail.** **Step 3: Implement.**
- [ ] **Step 4: Verify** — `payout_pass_pg` and `payout_rails_pg` against PostgreSQL; the whole ingest bin pg run per `CLAUDE.md`; `cargo test -p trace-commons-server --test versioned_pipeline_runtime_pg -- --test-threads=1` unchanged against baseline (the notify edit); global checks.
- [ ] **Step 5: Commit** — `Run a payout pass per rail, off by default`.

**Done when:** every Review Focus item 1-4 has a passing test against PostgreSQL with output pasted, all modes default to `disabled` (so merging changes nothing in production), the settlement edit is the notify only, and readiness lists each rail.

---

### Task 6: Link routes on the signed-in session (`payout-rail-link-routes`, base `payout-rail-pass`)

Spec: section 5 (all, including "Who confirms"), section 6.

**D5 superseded (owner, 2026-10-23).** No passkey gate: link, change and unlink are authorized by the contributor's signed-in account session (any kind; a device bearer is refused). `require_passkey_step_up`, the `confirm-payout-link` step-up page section and the `native-step-up-page.md` change are dropped. The two calls stay (start, then confirm from the same account) so the app can show the consent between them and nothing is written before the person confirms. The server still verifies org ownership with NEAR AI (`/users/me` with the person's token, never stored), keeps the "changed since shown" check (`expected_current_version` against the account's link version on the rail, checked at start before the token leaves the machine and again at confirm), and audits every request and confirmation.

**Files (as built):**
- Create: `crates/trace-commons-protocol/src/payout_link.rs` (permissive; `PayoutLinkRequest { action, idempotency_key, expected_current_version, proof: Option<PayoutLinkProof> }`, `#[serde(tag = "rail")] enum PayoutLinkProof { #[serde(rename = "near_ai")] NearAi { organization_id: Uuid, near_ai_access_token: String } }`, start, confirm and status response types, the outcome label enum, `near_ai_recipient_ref`; every struct `deny_unknown_fields`; the token is redacted in `Debug`)
- Modify: `crates/trace-commons-protocol/src/lib.rs`
- Create: `crates/trace-commons-server/src/payout_recipient_proof.rs` (`trait RecipientProofVerifier`; `NearAiOrgVerifier`: one `GET {base}/users/me` with the person's token, read into a struct of `id` and `organizations[]{id, role, is_active}` only, never `name`; the subject check against the account's `near_ai_login` anchors; `role == "owner"`, active; then `rail.probe_recipient`, mapped by `RailRefusal`)
- Create: `crates/trace-commons-server/src/payout_recipient_seal.rs` (sealing under the NEAR account identity's key with a recipient index, a subject index and a link handle, each keyed under its own domain; `SealedRecipientOpener`, the real `PayoutRecipientOpener` task 5 left a seam for)
- Modify: `crates/trace-commons-server/src/near_account_identity.rs` (crate-private `keyed_label` and `seal_bound_to`)
- Modify: `crates/trace-commons-server/src/db/mod.rs`, `db/postgres_payout.rs` (`SealedRecipient.recipient_ref`; `payout_account_link_view`, `payout_account_login_anchors`, `confirm_payout_link_request`, which takes the request and writes the link or the unlink mark with its audit rows in one transaction, serialized per account and rail)
- Create: `migrations/V134__payout_link_recipient_ref.sql` (`recipient_ref` on links and link requests); register it in `db/postgres.rs`
- Modify: `crates/trace-commons-server/src/payout_worker.rs` (`PayoutRailEntry::rail`, `PayoutRails::from_config_at_loopback` for tests)
- Create: `crates/trace-commons-ingest/src/bin/trace_commons_ingest_internal/payout_link.rs` (routes `POST /v1/account/payout-rails/{rail_id}/link-requests`, `POST .../link-requests/{handle}/confirm`, `GET /v1/account/payout-rails`)
- Modify: `crates/trace-commons-ingest/src/bin/trace-commons-ingest.rs` (route registration; boot refuses an enabled rail without the recipient key), `.../account_routes.rs` (the three routes `Refused` for an unbound account), `.../payout_rails.rs` (the sealed-recipient opener)
- Tests: `crates/trace-commons-ingest/src/bin/trace_commons_ingest_internal/tests/payout_link_tests.rs` (ingest lib, runs under `ingest-bin-postgres`), unit tests in the protocol and server modules

- [x] **Step 1: Write the failing tests**

```rust
// protocol
#[test] fn link_requests_refuse_unknown_fields_and_unknown_rails()
#[test] fn the_access_token_never_appears_in_debug()
#[test] fn recipient_ref_is_recomputable_by_the_client()
// server
#[test] fn a_sealed_recipient_opens_only_under_its_own_keys()
#[test] fn the_subject_index_is_not_the_login_anchor()
#[tokio::test] async fn a_non_owner_inactive_or_absent_org_is_refused_before_the_probe()
#[tokio::test] async fn probe_404_and_409_and_403_map_to_their_outcomes()
// ingest pg
#[tokio::test] async fn a_link_request_with_an_owned_active_org_is_pending_confirmation_and_changes_nothing()
#[tokio::test] async fn an_account_with_no_passkey_links_from_its_signed_in_session()
#[tokio::test] async fn a_non_owner_or_inactive_org_is_refused()
#[tokio::test] async fn a_nearai_tenant_with_a_different_subject_is_refused()
#[tokio::test] async fn the_org_name_and_token_are_never_stored_or_returned()
#[tokio::test] async fn probe_404_and_409_and_403_map_to_their_outcomes()
#[tokio::test] async fn confirm_needs_the_same_account_session_and_no_passkey()
#[tokio::test] async fn an_expired_or_second_request_is_discarded()
#[tokio::test] async fn a_link_that_changed_since_it_was_shown_is_refused()
#[tokio::test] async fn a_device_bearer_is_refused()
#[tokio::test] async fn a_disabled_rail_answers_payout_unavailable_and_an_unknown_rail_is_refused()
#[tokio::test] async fn status_lists_each_enabled_rail_without_a_recipient()
#[tokio::test] async fn change_and_unlink_on_one_rail_leave_another_rails_link_alone()
#[tokio::test] async fn every_request_and_confirmation_writes_an_audit_row()
#[tokio::test] async fn an_enabled_rail_without_the_recipient_key_refuses_the_start()
```

The passkey tests the plan first listed (`confirm_requires_a_fresh_passkey_session_on_the_same_account`, `an_account_with_no_passkey_gets_passkey_required`, `a_near_wallet_session_cannot_confirm`) are dropped with D5.

- [x] **Step 2: Run to verify they fail.** **Step 3: Implement.**
- [x] **Step 4: Verify** — protocol tests and `--no-default-features` check; the whole ingest lib pg run; `cargo test -p trace-commons-server --test license_boundary`; global checks.
- [x] **Step 5: Commit** — `Let an account link a payout recipient from its signed-in session`.

**Done when:** a link changes nothing until the same account confirms it, the server alone verifies ownership with the person's token against the compiled-in host, no org name or id reaches storage unsealed or a log, nothing asks for a passkey, and every test passes against PostgreSQL with output pasted.

---

### Task 7: Daemon link flow (`payout-rail-daemon-link`, base `payout-rail-link-routes`)

Spec: section 5 ("Verification on the server, not the client" steps 1-3, 7; "Consent copy").

**D5 superseded (owner, 2026-10-23).** No step-up URL and no passkey. The daemon presents its account session on start and on confirm and calls `POST /v1/account/payout-rails/{rail_id}/link-requests/{handle}/confirm` itself once the person confirms in the app, after the app has shown the consent for the org they chose. The status `version` the app showed is passed through as `expected_current_version` on every start. The copy keys drop the passkey lines.

**Files (as built):**
- Create: `crates/trace-commons-contributor/src/payout_link.rs` (ingest client for the three routes, in `inference_connection.rs`'s client shape: account session bearer, rotated token kept on refusals too, fixed-label errors; a server refusal keeps its `PayoutLinkOutcome` label, matched before the 401/403 session fallthrough; a started link must answer `near_ai_recipient_ref(idempotency_key, org)` or it is refused as `payout-link-recipient-mismatch` and never confirmed; rail ids and handles are checked before they become path segments)
- Create: `crates/trace-commons-contributor/src/daemon/payout_link.rs` and `payout_link_tests.rs`. Five IPC methods, not the four first planned, because start and confirm stay separate (the app shows the consent between them, and nothing is saved before the click):
  - `payout_link_orgs`: exchange the retained NEAR AI refresh session for a fresh access token, `GET {CLOUD_API_ORIGIN}/v1/users/me` with it, keep active orgs with `role == "owner"` and a UUID id as `{id, name}` in memory only, and answer them with an `orgs_revision` (a domain-separated hash over every shown id and name). The token is dropped; the commons is not called.
  - `payout_link_start` (`rail_id`, `organization_id`, `orgs_revision`, `expected_current_version`, all required; nothing is preselected): exchange again for a fresh token, read `/users/me` again, refuse `payout-link-orgs-changed` before anything reaches the commons if the revision differs, then send the org id and that token in the start body once and drop the token. The handle is held in memory.
  - `payout_link_confirm` (`rail_id`, `action`, `organization_id` for a link, `confirmed: true`): refuses `payout-link-confirmation-required` without the flag and `payout-link-changed` if the action or org is not the started one; on success records the idempotency key and org id (never the name) in `daemon-payout-link.json`.
  - `payout_link_unlink` (`rail_id`, `expected_current_version`): starts an unlink with no proof and no NEAR AI call; confirmed with `payout_link_confirm`.
  - `payout_rails_status`: the commons' status per rail, plus `recipient_ref_check` (`match`, `mismatch`, `unknown`, or null with nothing linked) recomputed from the local record, the org name only on a match, from the listing in memory or, after a restart, from `daemon-payout-org-names.json` (owner decision 2026-10-23: "save the org names to disk"; the linked or pending org's name only, 0600, refreshed on every list read, deleted on NEAR AI forget), and `pending_here` for a started request.
  - A wallet-only account (no NEAR AI session) answers `payout-link-near-ai-sign-in-required`. Audit rows are labels only.
- Modify: `crates/trace-commons-contributor/src/daemon/nearai_credential/api.rs` (`CloudApi::owned_organizations`; reads only `organizations[]{id, name, role, is_active}`), `daemon/ipc.rs` (the five methods in `METHODS`, `ASYNC_ONLY_METHODS` and the async dispatcher; `DaemonShared::payout_link`), `daemon/mod.rs`, `lib.rs`, `config.rs` (`DAEMON_PAYOUT_LINK_FILE`, an enrollment file swept by `wipe()` and unenroll)
- Modify: `crates/trace-commons-contributor/src/consent_copy.rs` (`PayoutLinkCopy`, `payout_link_copy()`, `render_org_name`, `PAYOUT_COPY_PLACEHOLDERS`: title, body, scope line, button, secondary, accessibility name, unlink button, its accessibility name and body, five status lines (linked, none, waiting, unavailable, mismatch) and four refusal lines; each doc-commented `DRAFT, NEEDS APPROVAL`; templates take `{org_name}`; no passkey line)
- Modify: `crates/trace-commons-contributor-ffi/src/lib.rs` and both `trace_commons.h` copies (`tc_payout_link_copy_json`; `tc_consent_copy`'s payload is left alone so no shell's decoder changes)
- Tests: the client's in `payout_link.rs`, the IPC flow against a stub commons and a stub cloud-api in `daemon/payout_link_tests.rs`, the copy's in `consent_copy.rs`, and `the_payout_link_copy_crosses_the_abi` in the FFI `abi` test

- [x] **Step 1: Write the failing tests**

```rust
#[tokio::test] async fn org_listing_keeps_only_owned_active_orgs_and_never_sends_names_to_ingest()
#[tokio::test] async fn a_single_org_still_needs_a_confirm()
#[tokio::test] async fn start_sends_the_fresh_token_once_and_drops_it()
#[tokio::test] async fn a_changed_org_list_refuses_the_confirm()
#[tokio::test] async fn status_flags_a_recipient_ref_mismatch()
#[tokio::test] async fn unlink_starts_and_confirms_without_a_token_or_a_near_ai_call()
#[tokio::test] async fn a_wallet_only_account_is_sent_to_near_ai_sign_in()
#[tokio::test] async fn the_shown_version_is_required_and_passed_through()
#[tokio::test] async fn a_refused_confirm_keeps_the_commons_label_and_ends_the_request()
#[tokio::test] async fn audit_rows_are_labels_only()
#[tokio::test] async fn every_method_requires_an_account_session()
#[test] fn every_payout_copy_key_is_marked_draft_and_follows_the_copy_guide() // short action labels, separate title and accessibility keys
#[test] fn copy_says_other_members_can_spend_and_overage_is_paid_first()       // D7, recipient debt
```

- [x] **Step 2: Run to verify they fail.** **Step 3: Implement.**
- [x] **Step 4: Verify** — `cargo test -p trace-commons-contributor` against baseline; `RUSTFLAGS="-D warnings" cargo check -p trace-commons-contributor --no-default-features`; `cargo test -p trace-commons-contributor-ffi`; `diff` of the two headers; global checks.
- [x] **Step 5: Commit** — `Let the daemon list owned NEAR AI orgs and start a payout link`.

**Done when:** org names never leave the device, the daemon can start, confirm and undo a link against mocks with no passkey, copy keys exist and are marked for owner review, and every check is green with output pasted.

---

### Task 7a: Staging build of the whole NEAR AI path (`payout-rail-staging-build`, base `payout-rail-daemon-link`)

Spec: owner decision D9; section 8 ("Known staging gap"); section 12 item 9.

The `near-ai-staging` feature from task 4 switches only the daemon's credential API origin and the server's transfer base. D9 requires one build that runs sign-in, settlement and Private AI against staging together.

**Files:**
- Modify: `crates/trace-commons-contributor/src/managed/route.rs` (inference base), `src/envelope.rs` (receipt endpoint default), `src/daemon/nearai_credential/funding.rs` (`CLOUD_ORIGIN`) and `src/external_url.rs` (dashboard prefix): each switches under `near-ai-staging`, ideally from one shared host module so no host is missed
- Modify: the attestation pins the daemon checks (gateway measurements, per-model receipt signers) so a staging build carries staging's pins; verification is never skipped
- Modify: the IronWire upstream, if it holds its own host
- Modify: `crates/trace-commons-contributor/src/daemon/ipc.rs` (the daemon's status reports a staging build; wire up the existing `near_ai_staging()`)
- Modify: `scripts/ci/check-release-staging-feature.sh` self-test only if a new manifest gains the feature

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn a_staging_build_points_every_near_ai_host_at_staging() // grep-style: no production NEAR AI host is reachable under the feature
#[test] fn a_staging_build_still_refuses_an_unpinned_receipt_signer()
#[test] fn status_reports_a_staging_build()
```

- [ ] **Step 2: Run to verify they fail.** **Step 3: Implement.**
- [ ] **Step 4: Verify** — `cargo test -p trace-commons-contributor` and `--features near-ai-staging`, both against baseline; the release staging check and its self-test; global checks.
- [ ] **Step 5: Commit** — `Point the whole NEAR AI path at staging in a staging build`.

**Built in the channel module and this PR:** the daemon half (every NEAR AI host, the app identity and the pin set switch together under `near-ai-staging`, from `crates/trace-commons-contributor/src/channel.rs`, ratcheted by `tests/channel_hosts.rs`) landed before this stack. This PR covers the server deployment half: `deploy/staging-gcp/render-env.sh` and `ingest.env.template` now carry every key an enabled payout rail reads, including `TRACE_COMMONS_CREDIT_SETTLEMENT_TABLE_PATH` (required whenever the mode is `dry_run` or `live`, supplied only at render time, an absolute path to an existing file), and the staging README documents the V132 role grant (`GRANT trace_payout_enumerator, trace_payout_config_writer TO <ingest login> WITH INHERIT FALSE, SET TRUE`), the recipient-key requirement (provisioning on, pepper, account-name KEK) and the order to enable the rail. `scripts/ci/test-deploy-render.sh` checks that every setting `NearAiRailConfig` and the rate path name is rendered. Link confirmation is the signed-in session only (D5 superseded 2026-10-23); nothing here needs a passkey.

**Blocked on facts from NEAR AI:** the staging inference host, and staging's gateway measurements and receipt-signer keys (spec section 12 item 9). Build the switch with placeholders that fail closed until they are filled in.

**Done when:** a staging build signs in, makes an attested Private AI call and receives a payout, all on staging, and no shipped build can enable the feature.

---

### Task 8: macOS org picker (`payout-rail-macos-picker`, base `payout-rail-daemon-link`)

Spec: section 5 ("Consent copy"), D1, D5 (superseded 2026-10-23), D7.

**D5 superseded:** no passkey state, no "Continue with passkey", no step-up page and no add-passkey route; where this section says otherwise, the owner decisions note at the top governs. The daemon's five IPC calls and the copy (`tc_payout_link_copy_json`) are task 7's.

**As built:**
- `macos/Sources/TCShellCore/PayoutLinkSurface.swift`: `PayoutLinkCopy` (decodes the core export, refuses a payload with a missing or blank field), the wire types for `payout_rails_status` and `payout_link_orgs`, `PayoutLinkSurface` (status line per rail state, when choosing and stopping are offered), `PayoutPicker` (nothing preselected even with one org; the consent for the selected org is built before the Choose button), `PayoutLinkRefusal` (routes the daemon's labels to the sign-in, changed and failed lines) and `PayoutLinkClient` (the five IPC calls over `DaemonTransport`, asserted with a fake transport).
- `macos/Sources/TraceCommonsApp/DaemonClient+PayoutLink.swift`, `PayoutLinkModel.swift`, `Views/PayoutLinkView.swift`: the model drives the calls off the main actor; the view sits in the Private AI settings section and draws nothing unless the commons lists `near_ai` as `dry_run` or `live`.
- `TCConsentCopy.payoutLinkCopyJSON()` reads `tc_payout_link_copy_json`.
- Choose is one click that runs start then confirm, after the consent for the selected org is on screen. Stop sending credit does the same for an unlink. There is no separate waiting state to show: the confirm is the person's click.
- The picker shows while no link is active (or a mismatch or a gone org needs a new choice). An active link has a direct Change button (owner decision 2026-10-23): it lists the orgs and runs the same consent, start and confirm as a first link, with the `version` shown as `expected_current_version`, so a link that moved since is refused. Its label is "Change" and its accessibility name "Change the organization that receives NEAR AI credit" (core copy keys `change_button`, `change_accessibility_name`, drafts).
- The owner approved the payout wording as written on 2026-10-23 ("Wording is fine"); the constants keep their `DRAFT, NEEDS APPROVAL` markers until the copy test's marker rule is retired.

**Done when:** the picker works end to end against a local daemon and mock services (describe the manual run; no screenshots of the user's screen), Swift tests pass with output pasted, and every visible string comes from core copy.

---

### Task 9: Reconciliation drill (`payout-rail-reconcile-drill`, base `payout-rail-macos-picker`)

Spec: section 8 ("Reconciliation drill"); `docs/operator/drills.md`.

**Files:**
- Create: `crates/trace-commons-server/src/payout_reconcile.rs` (per rail: enumerate links through the enumerator role, `rail.history` per link, compare confirmed deltas to program totals, every confirmed receipt to a record, no unknown record in our programs; netting-in-progress count; read-only)
- Modify: `crates/trace-commons-ingest/src/bin/trace-commons-ingest.rs` (`POST /v1/admin/payout-reconcile-drill` beside `db_reconciliation_drill_handler` at ~48154, same auth and evidence shape; wire its evidence into `rollout_smoke_evidence_handler` at ~48029 the way the existing drills are)
- Modify: `docs/operator/drills.md`
- Tests: `crates/trace-commons-server/tests/payout_pass_pg.rs` (extend) and ingest `tests.rs`

- [ ] **Step 1: Write the failing tests** — `a_clean_rail_reports_ready`, `a_missing_receipt_reports_payout_line_missing_transfer`, `an_unknown_record_in_our_program_reports_payout_history_unmatched`, `a_total_mismatch_reports_payout_program_total_mismatch`, `one_rails_gap_does_not_hide_another_rails_result` (near_ai and keyed rail), `the_drill_changes_no_row` (row checksums before and after), `drill_evidence_is_hash_only_and_reaches_rollout_smoke`.
- [ ] **Step 2: Run to verify they fail.** **Step 3: Implement.** **Step 4: Verify** — pg suites, ingest bin pg run, global checks.
- [ ] **Step 5: Commit** — `Add a per-rail payout reconciliation drill`.

**Done when:** the drill reports per rail, never writes, and appears in rollout-smoke evidence, with tests passing against PostgreSQL and output pasted.

---

### Task 10: Operator runbook and staging steps (`payout-rail-runbook`, base `payout-rail-reconcile-drill`)

Spec: sections 2, 8 ("Steps"), 9, 12.

**Files:**
- Create: `docs/operator/payout-rails.md` (what a rail is; enabling `near_ai` in `dry_run` then `live`; the env list with no values; credential minting and rotation per `commons-credential-storage.md` and `key-rotation.md`; halt, resume, release; readiness fields; the drill; the staging steps 1-9 of spec section 8 with expected results; the commons' total exposure is the sum over enabled rails)
- Modify: `docs/operator/README.md` (list it), `docs/operator/env-reference.md` (the new env names)
- No code.

- [ ] **Step 1:** Write the runbook; every command and route in it must exist in the stack (grep each route and env name against the code and paste the hits).
- [ ] **Step 2:** `grep -nEi 'oc_[0-9a-f]|[0-9a-f]{8}-[0-9a-f]{4}-' docs/operator/payout-rails.md` returns nothing.
- [ ] **Step 3: Commit** — `Document payout rails for operators`.

**Done when:** the runbook is indexed, every route and env name in it is verified against the code, and it contains no real value.
