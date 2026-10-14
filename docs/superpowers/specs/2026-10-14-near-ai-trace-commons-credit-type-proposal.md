# Proposal to NEAR AI: a `trace_commons` credit type

Status: draft, 2026-10-14. Not sent. A reviewable implementation exists on
a local cloud-api branch, `trace-commons-credit-type` (commit `d18e2ca1` on
top of `752e2976`). It is unpushed, and no PR or issue is open; opening one
is the owner's call. Citations of the form `cloud-api:<path>:<line>` are
into `nearai/cloud-api` at `752e2976`.

## Why

The only way NEAR AI exposes for an admin to grant credit is
`PATCH /v1/admin/organizations/{org_id}/limits`. It replaces the active row
for `(organization, credit_type)` with an absolute value
(`cloud-api:crates/database/src/repositories/organization_limits.rs:73-121`).
It has no compare-and-swap, and replacement is keyed on the credit type alone
(`:76-80`). An external writer of `grant` would race every other writer of
`grant`. The design spec, section 1, has the full analysis.

NEAR AI already has the safe pattern, built for House of Stake staking
rewards:

- **A dedicated credit type.** `CreditType::StakingFarm`
  (`cloud-api:crates/api/src/models.rs:4561-4562`) has one writer, so no other
  limit write touches it.
- **Cumulative and monotonic.** Each sync recomputes the total credit from
  the cumulative on-chain rewards and keeps
  `max(computed, last_synced)` (`cloud-api:crates/services/src/staking_farm.rs:393-395`).
  The write happens only when the value grows (`:397-401`).
- **Lock and rechecks.** `update_staking_farm_limit`
  (`cloud-api:crates/database/src/repositories/organization_staking_farm_sources.rs:286`):
  1. takes `lock_organization_accounting` (`:302`);
  2. rechecks that the source and the organization are active (`:311-325`);
  3. writes nothing unless the new value is greater (`:331-334`);
  4. closes the active row (`:340`) and inserts a new one;
  5. runs `settle_unfunded_usage` in the same transaction (`:379`).
- **Binding and sync triggers.** The organization binds a NEAR account through
  a user-initiated flow (`cloud-api:crates/api/src/routes/staking_farm.rs:581`).
  Sync runs on demand from the user (`:198`) or an admin (`:297`), and as a
  usage-middleware preflight when the stored value is stale
  (`cloud-api:crates/api/src/middleware/usage.rs:31`,
  `services/src/staking_farm.rs:444`). Sync is gated on AML (`:487`).

Because every write is a monotonic maximum of a cumulative total, it is
idempotent: a retry, a duplicate, or a late, smaller value changes nothing.
It needs no idempotency key and no compare-and-swap. We are asking for the
same properties for credit earned through Trace Commons.

## The change

1. **Credit type.** Add `CreditType::TraceCommons`, serialised as
   `trace_commons`, with the same `Display`, `as_str` and `FromStr` arms as the
   existing types (`models.rs:4556-4610`). Add it to `SUPPORTED_CREDIT_TYPES`
   (`cloud-api:crates/config/src/types.rs:117`). The default spend order
   becomes `grant -> staking_farm -> trace_commons -> payment -> postpay`, so
   earned credit is spent before purchased credit. Add it to the
   reconciliation allowlist (`credit_reconciliation.rs:237`).
2. **Migration `V0095`.** Widen the `credit_type` CHECK constraints on
   `usage_credit_allocations` and `organization_credit_consumption`
   (`V0078__add_usage_credit_allocations.sql:48-49`, `:66-67`). No existing
   column can carry the value, because those constraints enumerate the
   allowed types. The one-active-row-per-type index from V44 already covers
   the new type.
3. **Repository: `TraceCommonsCreditRepository::set_cumulative(org, total, statement_ref, actor)`.**
   This mirrors `update_staking_farm_limit` step by step:
   1. take `lock_organization_accounting`;
   2. recheck `organizations.is_active`;
   3. read the active `trace_commons` limit;
   4. if `total <= current`, commit without writing and return
      `applied: false`;
   5. otherwise close the active row and insert a new one with source
      `trace-commons`, `changed_by` `trace_commons_sync`, and
      `change_reason = "Trace Commons cumulative credit " + statement_ref`;
   6. run `settle_unfunded_usage`.

   Bounds: `0 <= total <= 1e18`, the same ceiling the limits PATCH enforces
   (`services/src/admin/mod.rs:1504`). `statement_ref` is 1-128 characters
   of `[A-Za-z0-9_.:-]`. The repository also has `get_cumulative(org)`.
4. **Route (push mode).**
   - `PUT /v1/admin/organizations/{org_id}/trace-commons/credit` takes
     `{"cumulativeNanoUsd": i64, "statementRef": string}` and returns
     `{"organization_id", "cumulativeNanoUsd", "applied"}`.
   - `GET` on the same path returns the current total, or 0 when there is
     none.
   - Errors: 404 for an unknown or inactive organization, 400 for invalid
     input.
   - `GET` is added to the read-only admin list
     (`cloud-api:crates/api/src/middleware/admin_policy.rs:36-69`); `PUT`
     needs a read-write token or a session, which is the default for any
     unlisted route.
   - The routes are mounted the way the reconciliation route is
     (`lib.rs:2409-2422`) and registered in OpenAPI.
5. **Single writer.** The generic limits PATCH refuses `trace_commons`, in
   `validate_organization_limits` (`services/src/admin/mod.rs:1497`).
6. **Documentation.** `docs/credit-allocation.md` gains a section on the
   new type.

## Semantics we depend on

- **Monotonic.** The limit never decreases through this route, and nothing
  else writes the type.
- **Idempotent.** Repeating any total, or sending an older one, is a no-op
  that returns the current total.
- **Serialised.** All writes take the organization accounting lock, the same
  lock that usage posting, limit changes and staking sync take.
- **Active-only.** An inactive organization gets nothing.
- **Consumption survives.** Raising the ceiling adds only new capacity,
  because consumption per type is lifetime
  (`organization_limits.rs:169-172`). New capacity first pays down any
  unfunded usage (`settle_unfunded_usage`).

## Pull mode, for discussion (not in the draft)

In the staking-farm style, NEAR AI would bind an organization to a Trace
Commons recipient and periodically fetch a signed statement:
`{org_id, cumulative_nano_usd, statement_ref, issued_at}`, signed with
Ed25519 over a fixed line-separated preimage. NEAR AI would verify it
against a pinned public key and apply it through the same `set_cumulative`.
Our side of this is built (`near_ai_credit_statement.rs`, `signed_statement`
in `settlement_rail_store.rs`). It needs these on NEAR AI's side:

- a binding table and flow;
- key pinning;
- a sync schedule or preflight;
- possibly the AML gate.

Pull removes the admin credential from our side. Later the statement could
be replaced by a NEP-141 balance read.

## Checks run on the local branch

| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | pass |
| `cargo clippy --all-targets --all-features -- -D warnings` (their CI command) | pass |
| `cargo test -p config -p database -p api -p services --lib` | 1471 passed |
| `cargo test --workspace --lib --bins` | 1264 passed, 18 failed. All 18 failures are in `inference_providers` `placement_hook`, a crate the change does not touch and none of whose dependencies it touches. |
| Database suites on PostgreSQL 16, their CI set plus `staking_source_inactive_writes` and the new `trace_commons_credit` (5 tests) | 52 passed, on fresh database `admission_test_cloudapi_db2` (`xact_commit` 2634) |
| New e2e module `trace_commons_credit` (4 tests) | not run: the e2e server needs a dstack socket |

## Open questions for NEAR AI

1. Is a dedicated type acceptable, or would they rather have a
   `(type, source)`-scoped limit for external sources in general?
2. Where should the type sit in the default spend order?
3. Push or pull? Under push, would they mint a narrower token scope than
   read-write admin?
4. Should the AML gate apply, and to which identity?
5. Do they want a per-organization or per-call cap beyond $1B?
6. Explicit `CREDIT_USAGE_ORDER` values must list five types after deploy.
   Is that acceptable as a deploy step?
