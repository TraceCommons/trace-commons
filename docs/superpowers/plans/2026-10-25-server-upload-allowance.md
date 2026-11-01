# Server Upload Allowance Implementation Plan

> **For agentic workers:** Use superpowers:executing-plans to implement task-by-task.

**Goal:** Automatically adopt server daily allowances without overriding explicit local choices.
**Architecture:** An authenticated endpoint returns an optional versioned allowance. A five-minute client refresh maintains an identity-scoped runtime value; effective settings feed both upload enforcement and budget status.
**Tech Stack:** Rust, serde, axum, existing operator HTTP client.
**Spec:** docs/superpowers/specs/2026-10-25-server-upload-allowance-design.md

## Global Constraints

No dependencies or migrations. Bound allowances to 1,000 uploads and 5 GiB. Preserve consent, pause, usage counters, explicit overrides, account/session rotation, tenant scope, and hash-only logs. Every cargo command uses RUSTFLAGS='-D warnings' CARGO_INCREMENTAL=0.

## Review Focus

Malformed/oversized replies, legacy explicit settings, identity changes during requests, spent-admission backoff, and matching status/enforcement limits require coverage.

### Task 1: Protocol and authenticated server endpoint

- [x] Add tests for optional v1 allowances and invalid/partial/out-of-range operator configuration.
- [x] Implement `trace_commons_protocol::upload_allowance::{DailyUploadAllowance, UploadAllowanceResponse}` with public schema_version/u32 count/u64 bytes and validation, and authenticated GET `/v1/account/upload-allowance`.
- [x] Read operator configuration once at startup; absent returns null allowance; apply account binding and no-store conventions.
- [x] Run focused protocol/server tests and warning-denying checks.

### Task 2: Client effective limits and provenance

- [x] Write regressions for a spent default budget becoming available with unchanged counters, explicit count/byte overrides (including default-valued choices), and legacy migration.
- [x] Persist per-dimension override provenance; retain server policy only in runtime state.
- [x] Feed a single effective-settings calculation to budget snapshots and upload passes.
- [x] Run focused contributor tests.

### Task 3: Independent allowance refresh

- [x] Write HTTP/lifecycle regressions for refresh while capped, failures, absent and invalid policies, cadence, dry run, and stale replies after identity changes.
- [x] Add startup/five-minute refresh independent of admission eligibility/backoff, using cached account credential loading, existing allowlist HTTP and rotated-token persistence.
- [x] Document endpoint/operator configuration, migration behavior, and effective-budget semantics.
- [x] Run contributor suite, formatting, relevant clippy/checks and license boundary; review the final diff and fix material findings.

## Verification

- Contributor library: 3,961 passed, six ignored; includes real capped approved-queue resumption with unchanged counters and transient credential-store failure retention.
- Protocol: 565 tests passed across unit/integration targets, including required explicit allowance/null decoding.
- Ingest allowance configuration and authenticated routes: three passed; exercised native and cookie auth, bound/legacy accounts, and unbound/closed refusals without PostgreSQL skips.
- Unchanged license boundary: four passed.
- Clippy: protocol, contributor and ingest, all targets, repository allow-list, warnings denied.
- Workspace formatting and diff whitespace checks passed.
- Independent static review found a transient credential-store failure could clear a valid allowance; reproduced with a failing HTTP/lifecycle regression, repaired, and included in the passing suite. Final review found no remaining important issues.

No dependencies, migrations, deployment or release were added. Operator configuration is opt-in; see docs/operator/client-upload-allowance.md.


### Earned-trust refinement

- [x] Surface the validated bounded effective allowance from the account-status transaction, with PostgreSQL coverage for fresh/stale/shadow/generation-invalid evaluations.
- [x] Scale the daily base caps proportionally in the authenticated endpoint using bounded integer arithmetic; retain invitation, legacy and unconfigured compatibility.
- [x] Update endpoint/configuration tests and operator documentation; keep private scoring rules and thresholds outside this change.
- [x] Run one consolidated relevant verification pass and review before committing.


Earned-trust refinement verification: 3,961 contributor library tests passed (six ignored); eight client and five server allowance tests passed. Isolated PostgreSQL regressions passed for external evaluation freshness/scope/clamping/spend, live tiered applied-growth fallback, and the real authenticated endpoint's threefold daily caps and stale fallback. Protocol wire tests (two), unchanged license boundary (four), all-target Clippy for protocol/contributor/server/ingest, and formatting passed with warnings denied and incremental compilation disabled. Independent final review found no important issues. No evaluator rules, thresholds, dependency, migration or deployment changes.
