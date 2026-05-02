# Trace credit settlement implementation plan

Date: 2026-05-02
Repo: `zmanian/tracedao-server`
Scope: server-side non-transferable Trace Credits, utility attestations, settlement batches, holds, and NEAR non-transferable receipt outbox

## Constraints

- Keep the off-chain TraceDAO server ledger authoritative for v1.
- Treat NEAR as a non-transferable settlement mirror, not as a transferable token.
- Do not place trace bodies, raw contributor identity, or raw lab notes in settlement records or NEAR payloads.
- Preserve existing delayed utility credit worker behavior; settlement consumes that evidence instead of replacing every producer in this slice.
- Start with caller-level tests before production code.

## Implementation Steps

### 1. Storage contract and migration

Files:
- `crates/tracedao-server/src/trace_corpus_storage.rs`
- `crates/tracedao-server/src/db/trace_corpus_pg.rs`
- `migrations/V2__trace_credit_settlement.sql`
- `crates/tracedao-server/tests/trace_corpus_storage_contract.rs`

Add typed storage models for:
- utility attestations
- settlement batches and account line items
- credit holds
- NEAR credit outbox items

Add PostgreSQL tables with tenant-scoped primary keys, idempotency keys, and safe-hash metadata. Extend the `TraceCorpusStore` trait and PostgreSQL backend with append/list/upsert methods.

Verification:
- storage-contract tests prove enum JSON names and hash-only metadata shape.
- PostgreSQL store tests prove tenant scoping and idempotency once a database is available.

### 2. NEAR payload builder

Files:
- `crates/tracedao-server/src/near_credit.rs`
- `crates/tracedao-server/src/lib.rs`
- `crates/tracedao-server/tests/near_credit_contract.rs`

Implement a dependency-light NEAR method-call payload builder for `settle_credit_receipt`, `reverse_credit_receipt`, and `freeze_credit_account`. The builder should produce deterministic JSON args and idempotency keys for a later signer/submitter, while refusing transfer-style operations.

Verification:
- tests assert deterministic method names, idempotency, safe fields, no raw contributor/trace fields, and transfer rejection.

### 3. Ingest/admin routes

Files:
- `crates/tracedao-server/src/bin/tracedao-ingest.rs`

Add routes:
- `POST /v1/workers/utility-attestations`
- `GET /v1/admin/credit-attestations`
- `POST /v1/admin/credit-holds`
- `GET /v1/admin/credit-holds`
- `POST /v1/admin/credit-settlements`
- `GET /v1/admin/credit-settlements`
- `GET /v1/admin/near-credit-outbox`
- `POST /v1/workers/near-credit-outbox/mark-status`

Settlement should:
- require admin auth
- support dry-run with no writes
- select only accepted, non-terminal, policy-allowed delayed utility credit events
- exclude held accounts
- avoid double-settling source events already in finalized batches
- write deterministic settlement batch and line items on non-dry-run
- optionally enqueue NEAR outbox items when configured

Verification:
- caller-level route tests for dry-run, non-dry-run idempotency, holds, tenant isolation, and NEAR disabled/enabled behavior.

### 4. Contributor projection

Files:
- `crates/tracedao-server/src/bin/tracedao-ingest.rs`

Extend `/v1/contributors/me/credit` with additive fields:
- `credit_points_estimated`
- `credit_points_pending_ledger`
- `credit_points_settled`
- `credit_points_reversed`
- `credit_points_held`
- `last_settlement_batch_id`

Keep existing response fields for compatibility.

Verification:
- contributor route test proves pending and settled are separate and terminal/revoked records do not leak into settled totals.

### 5. Docs and status

Files:
- `README.md`
- `docs/trace-commons.md`
- `docs/trace-commons-storage.md`

Document the new settlement routes, NEAR outbox behavior, and v1 non-transferability boundary.

Verification:
- `cargo fmt`
- targeted `cargo test` for storage contracts, NEAR payloads, and ingest route settlement tests
- `cargo check -p tracedao-server --bins`
