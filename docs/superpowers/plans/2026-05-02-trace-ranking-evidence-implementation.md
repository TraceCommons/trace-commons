# Trace Ranking Evidence Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the first durable evidence substrate for robust Trace Credits ranking.

**Architecture:** Add typed, hash-only ranking records for model versions, feature vectors, model predictions, frontier/reviewer labels, calibration reporting, and persisted calibration runs. Keep raw trace bodies, lab notes, and private external refs out of ranking records; route writes through worker/admin endpoints and mirror the schema in PostgreSQL with RLS.

**Tech Stack:** Rust, Axum handlers, tenant-scoped JSONL pilot storage, PostgreSQL migrations, serde contracts, caller-level route tests.

---

### Task 1: Storage Contracts

**Files:**
- Modify: `crates/tracedao-server/src/trace_corpus_storage.rs`
- Modify: `crates/tracedao-server/tests/trace_corpus_storage_contract.rs`

- [x] Add typed ranking enums: model status, label source, utility category, and label outcome.
- [x] Add a storage contract test proving enums serialize as snake case and labels remain hash-only.
- [x] Run `cargo test -p tracedao-server --test trace_corpus_storage_contract ranking_evidence_contract_uses_typed_enums_and_hash_only_outcomes`.

### Task 2: Runtime Routes And File Storage

**Files:**
- Modify: `crates/tracedao-server/src/bin/tracedao-ingest.rs`

- [x] Add admin routes for model versions, feature/prediction/label listing, and calibration reports.
- [x] Add worker routes for writing ranking features, predictions, and labels.
- [x] Validate accepted source submissions, target-use ABAC, bounded scores, `sha256:` evidence hashes, and sanitized identifier fields.
- [x] Store private external refs only as `sha256:` hashes.
- [x] Run `cargo test -p tracedao-server --bin tracedao-ingest ranking_evidence_pipeline_records_hash_only_predictions_and_labels`.

### Task 3: PostgreSQL Schema

**Files:**
- Create: `migrations/V3__trace_ranking_evidence.sql`
- Modify: `crates/tracedao-server/src/db/postgres.rs`
- Modify: `crates/tracedao-server/tests/trace_corpus_pg_rls.rs`

- [x] Add tables for ranking model versions, features, predictions, and labels.
- [x] Add tenant RLS policies and include the tables in RLS diagnostics.
- [x] Chain the V3 migration after V1/V2 in the PostgreSQL migration runner.

### Task 4: Documentation And Verification

**Files:**
- Modify: `README.md`
- Modify: `docs/trace-commons.md`
- Modify: `docs/trace-commons-storage.md`

- [x] Document the ranking evidence and calibration API surface.
- [x] Run focused tests, broad ingest tests, bin check, and diff hygiene.
- [x] Commit and push the PR update.

### Task 5: DB-Backed Runtime Ranking Evidence

**Files:**
- Modify: `crates/tracedao-server/src/trace_corpus_storage.rs`
- Modify: `crates/tracedao-server/src/db/trace_corpus_pg.rs`
- Modify: `crates/tracedao-server/src/bin/tracedao-ingest.rs`
- Modify: `crates/tracedao-server/tests/trace_corpus_pg_store.rs`
- Modify: `README.md`
- Modify: `docs/trace-commons.md`
- Modify: `docs/trace-commons-storage.md`

- [x] Add store write/read structs and trait methods for ranking model versions, feature records, prediction records, and labels.
- [x] Implement PostgreSQL upsert/list methods with tenant-scoped RLS context and idempotent frontier/reviewer labels.
- [x] Mirror ranking endpoint writes into the DB when `TRACE_COMMONS_DB_DUAL_WRITE=true`; fail closed under `TRACE_COMMONS_REQUIRE_DB_MIRROR_WRITES=true`.
- [x] Serve admin ranking evidence lists and calibration reports from the DB mirror under `TRACE_COMMONS_DB_REVIEWER_READS=true`.
- [x] Add Postgres store coverage and a caller-level DB mirror/read integration canary for ranking routes.

### Task 6: Persisted Calibration Runs

**Files:**
- Create: `migrations/V4__trace_ranking_calibration_runs.sql`
- Modify: `crates/tracedao-server/src/trace_corpus_storage.rs`
- Modify: `crates/tracedao-server/src/db/postgres.rs`
- Modify: `crates/tracedao-server/src/db/trace_corpus_pg.rs`
- Modify: `crates/tracedao-server/src/bin/tracedao-ingest.rs`
- Modify: `crates/tracedao-server/tests/trace_corpus_pg_store.rs`
- Modify: `crates/tracedao-server/tests/trace_corpus_pg_rls.rs`
- Modify: `README.md`
- Modify: `docs/trace-commons.md`
- Modify: `docs/trace-commons-storage.md`

- [x] Add a persisted calibration-run record with model version, target use, policy version, evaluation dataset hash, joined counts, aggregate error metrics, confidence threshold, promotion threshold, reason codes, and a hash-only report digest.
- [x] Add a worker route for writing calibration runs and an admin route for listing calibration-run history.
- [x] Mirror calibration runs to PostgreSQL under DB dual-write and serve them from the DB mirror under DB reviewer reads.
- [x] Include calibration runs in Trace Commons RLS diagnostics.
- [x] Add caller-level route coverage and PostgreSQL store coverage.
