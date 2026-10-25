# Staging Earned Upload Trust Implementation Plan

> **For agentic workers:** Use superpowers:executing-plans; independent file scopes may run in parallel.

**Goal:** Apply the approved 5/20/50 qualified-contribution progression without changing staging admission.
**Architecture:** Independent client-upload trust policy feeds the existing applied worker and validated database reader; endpoint scales shared daily defaults.
**Tech Stack:** Existing Rust/serde/PostgreSQL/ingest scheduler and deployment renderer.
**Spec:** docs/superpowers/specs/2026-10-25-staging-earned-upload-trust-design.md

## Global Constraints

No dependencies, scoring-threshold, consent, payout or signup changes. V137 is required for the evidence-driven ownership/status fact repair described in the spec; no historical credit or submission rewrite. Existing policies retain behavior. Fixed RUSTFLAGS='-D warnings' CARGO_INCREMENTAL=0. Follow CLAUDE.md: write tests alongside tasks, defer all test/check/Clippy runs to final consolidated verification. Preserve the public/private policy seam and hash-only output.

## Review Focus

Policy mismatch, evidence bypass, stale/wrong-generation evaluations, conflicting admission configuration, and historical duplicate qualification need explicit regression coverage.

### Task 1: Approved policy and qualification

- [x] Add default-false require_gate_pass policy flag and tests; when enabled attestation cannot bypass the gate.
- [x] Add staging policy template for exact approved progression, no age/week prerequisite, once-only novel accepted contributions.
- [x] Test tier boundaries and revoked/quarantined/withdrawn/duplicate/attested input cases.

### Task 2: Independent policy startup and scheduler

- [x] Add AppState independent client trust policy and select shared applied worker rule.
- [x] Reject conflicting account admission, partial/malformed policy, missing durable/RLS/worker contracts.
- [x] Keep both staging admission ledgers off; allow the existing scheduler to record/evaluate the independent policy.

### Task 3: Account endpoint and staging rendering

- [x] Parse paired independent policy JSON/version; retain matching bounded and growth policies.
- [x] Use authenticated account lookup and existing transaction-validated effective allowance against the independent policy.
- [x] Add authenticated PostgreSQL regression for approved cap progression with admission disabled and unchanged accounting.
- [x] Wire staging policy rendering without secrets; document enable, shadow qualification, applied verification and rollback.

### Source and runtime prerequisite repair

- [x] Use verified owned-principal view for recorder/candidates/input projection, preserve durable adverse-event ownership and generation invalidation, including concurrent first-time recording and legacy-only lifecycle exclusions.
- [x] Add opt-in accepted_submission_status facts without monetary-credit prerequisites; preserve old policies and UUID dedup.
- [x] Add separate narrow same-database trust pool for shadow/applied, leaving source runtime and admission unchanged.
- [x] Prepare exact narrow-role bootstrap and secret-backed renderer settings; no broad guard grants.

### Task 4: Verification and rollout preparation

- [x] Read-only live audit of schema, runtime roles, input frontiers and actual gate/dedup labels; prepare exact settings.
- [x] Run focused pure/HTTP and real PostgreSQL tests, client regression suite, licensing, formatting and Clippy.
- [x] Independent review; fix material findings and commit concrete implementation/rollout configuration. Distinguish local, PR, deployment and live client acceptance.


## Verification and completion

- Pure account-trust rules: 30 passed, including approved thresholds and strict source precedence.
- Full isolated PostgreSQL account-trust growth suite: 11 passed, including ownership, reviewer independence, retention, legacy-only history, private helpers, disabled invalidation controls, and deterministic concurrent alias/device changes.
- Actual pre-V137 migration upgrade: one passed; existing applied generations invalidate exactly once.
- Contributor upload regressions: 106 passed. Ingest upload/trust tests: 38 passed, two ignored in that filtered run; the relevant ignored scheduler and endpoint tests were executed separately against isolated PostgreSQL.
- Dedicated scheduler and authenticated endpoint: one PostgreSQL test each passed using separate source/worker roles.
- Operator role bootstrap, repeat bootstrap and unsafe-role refusal passed against a fresh local database. No broad guard or source-role privileges were added.
- Staging-feature signup, first upload, namespace admission and wallet exclusion: four passed.
- Unchanged license boundary: four passed. Offline renderer/Caddy checks, all-target Clippy, workspace formatting and diff checks passed.
- Independent final review found ownership serialization, legacy-only exclusion and migration invalidation gaps; all were repaired, covered by the passing regressions and re-reviewed without remaining material findings.

Code and rollout artifacts are prepared locally. No live staging migration, role/configuration/secret change, worker execution, activation, release or deployment was performed. Live checks were read-only. Account-specific audit evidence remains outside the repository.
