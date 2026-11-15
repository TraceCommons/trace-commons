# Reviewed Privacy Revisions Implementation Plan

> **For agentic workers:** Use scoped parallel implementation with one final whole-branch review.

**Goal:** Save and approve corrected content, then resume existing corpus and credit processing.

**Architecture:** Immutable candidate chain; claim-scoped server-side recheck; exact candidate-bound assessment; existing pipeline consumes bound bytes.

**Tech Stack:** Existing Rust protocol/server/ingest/operator client, PostgreSQL migrations, encrypted object store.

**Spec:** ../specs/2026-11-15-reviewed-privacy-revisions-design.md

## Global constraints

- Original source/pass immutable; no automatic approval or direct ledger edits.
- Tenant RLS, current consent, real reviewer claims, source CAS and exact candidate hashes.
- AllEvents review-only coverage; preserve correction text and dynamic keys.
- No new dependencies; AGPL headers on new server/ingest Rust files.
- Isolated worktree; RUSTFLAGS=-D warnings, CARGO_INCREMENTAL=0, SCCACHE_SERVER_PORT=4227.
- Write regression tests per task; run final suites once after implementation per repository guidance.

## Review focus

- A candidate swap cannot change what an existing assessment authorizes.
- Revocation or policy narrowing during external classification must refuse commit.
- Incomplete coverage or protected correction findings cannot permit release.
- Crash cleanup must not delete a committed candidate.
- Worker retries must index exact reviewed bytes and credit once.

## Tasks

- [x] Protocol: add evidence/result types and atomic review-only complete coverage; fake-classifier tests for side fields, protected corrections, keys, budgets and legacy behavior.
- [x] Core: V159 RLS/immutable candidate schema, staging/idempotency/source CAS, assessment binding, worker source/derived ref changes; PostgreSQL regressions including exact corpus bytes and credit retry.
- [x] Boundary/API: fail-closed privacy capability and AllEvents production adapter; metadata routes, double ABAC, body caps, safe responses, stale-candidate refusal and audited content read. Add HTTP denial/race/body/audit regressions.
- [x] CLI: explicit pipeline queue/claim/recheck/assessment commands and wire/parser tests.
- [x] Final validation: focused protocol + server/ingest PostgreSQL tests, license boundary, clippy with repository allowances, fmt/diff checks. Fix regressions without weakening gates.
- [x] Fresh whole-branch security review, including re-review of coverage fixes.
- [x] Prepare draft PR content after binary/test compilation gates; report implementation and rollout separately.

Implementation, validation, independent security review and draft PR preparation
are complete. No production rollout is part of this plan.

## Rulings during validation

- Reviewed source coverage uses typed wire serialization rather than to_value: shortest f32 wire decimals must not be mistaken for dropped or changed content. Exact field/value checks remain.
- Reauthenticate reviewer access after classifier/object I/O, then reapply current ABAC. A revoked grant cannot publish a prepared candidate or return content.
- Only the review-specific risk calculation excludes the existing exact non-severity labels; unchanged harmless correction paths do not block complete evidence, while personal data, secrets, unknown labels and all blocking flags stay conservative.
- The real queue exceeds ordinary structured limits in all 32 cases. Use the separate bounded manual profile from the spec, retaining ordinary defaults and external-I/O safeguards. A realistic 5,000-tool-event test reproduced the original inability to complete coverage before the profile change.

## Final local validation

- Protocol library: 748 passed; diagnostic example: 17 passed.
- Server library: 1437 passed, 7 ignored; review CLI: 28 passed.
- Runtime PostgreSQL: 470 passed; tenant RLS: 34 passed.
- Full pipeline HTTP PostgreSQL: 145 passed, 1 ignored.
- Final affected rerun: authority 8, candidate PostgreSQL 9, HTTP PostgreSQL 8 passed.
- License boundary: 4 passed; no dependencies changed.
- Server/ingest binary checks and all test targets compiled successfully (`cargo test --no-run`).
- Server/ingest all-targets clippy with repository allowances, formatting and diff checks passed.
- Independent security review closed event failure prose, original key preservation and classifier uncertainty findings with regressions; no remaining blocker.

Tests use an isolated local PostgreSQL cluster. This build does not deploy code,
execute production rechecks or assessments, release held content, or award credit.
