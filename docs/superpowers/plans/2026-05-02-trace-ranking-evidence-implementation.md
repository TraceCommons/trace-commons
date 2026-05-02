# Trace Ranking Evidence Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the first durable evidence substrate for robust Trace Credits ranking.

**Architecture:** Add typed, hash-only ranking records for model versions, feature vectors, model predictions, frontier/reviewer labels, and calibration reporting. Keep raw trace bodies, lab notes, and private external refs out of ranking records; route writes through worker/admin endpoints and mirror the schema in PostgreSQL with RLS.

**Tech Stack:** Rust, Axum handlers, tenant-scoped JSONL pilot storage, PostgreSQL migrations, serde contracts, caller-level route tests.

---

### Task 1: Storage Contracts

**Files:**
- Modify: `crates/trace-commons-server/src/trace_corpus_storage.rs`
- Modify: `crates/trace-commons-server/tests/trace_corpus_storage_contract.rs`

- [x] Add typed ranking enums: model status, label source, utility category, and label outcome.
- [x] Add a storage contract test proving enums serialize as snake case and labels remain hash-only.
- [x] Run `cargo test -p trace-commons-server --test trace_corpus_storage_contract ranking_evidence_contract_uses_typed_enums_and_hash_only_outcomes`.

### Task 2: Runtime Routes And File Storage

**Files:**
- Modify: `crates/trace-commons-server/src/bin/trace-commons-ingest.rs`

- [x] Add admin routes for model versions, feature/prediction/label listing, and calibration reports.
- [x] Add worker routes for writing ranking features, predictions, and labels.
- [x] Validate accepted source submissions, target-use ABAC, bounded scores, `sha256:` evidence hashes, and sanitized identifier fields.
- [x] Store private external refs only as `sha256:` hashes.
- [x] Run `cargo test -p trace-commons-server --bin trace-commons-ingest ranking_evidence_pipeline_records_hash_only_predictions_and_labels`.

### Task 3: PostgreSQL Schema

**Files:**
- Create: `migrations/V3__trace_ranking_evidence.sql`
- Modify: `crates/trace-commons-server/src/db/postgres.rs`
- Modify: `crates/trace-commons-server/tests/trace_corpus_pg_rls.rs`

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
