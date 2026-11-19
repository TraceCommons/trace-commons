# OpenCode and delegated skill usage implementation plan

> **For agentic workers:** Use the authorized dynamic workflow: parallel scoped extraction/backend/UI implementers, root integration and independent review.

**Goal:** Extend PR #2074 to support OpenCode and delegated trace skill observations.
**Architecture:** Reuse normalized OpenCode readers and bounded local background processing. Extract child evidence from existing spawned rows, but preserve separate direct/delegated populations and parent relationships. Do not alter ordinary token/spend accounting.
**Tech stack:** Existing Rust sources/insights/daemon, typed IPC, native SwiftUI, Quint/TLC.
**Spec:** 2026-11-11-local-skill-index-design.md plus user approval to implement both coverage extensions.

## Global constraints

- Existing insights_skills flag remains default off and grants no reading permission.
- No new dependencies; use existing OpenCode source adapters and known source roots.
- Preserve Never-folder exclusions, clear/feature invalidation, bounded processing and partial coverage.
- Keep direct/delegated and invocation/load figures distinct. Deduplicate database/export copies by stable source session/event identities.
- No billing/token rollup changes. User-facing copy says traces, not sessions.
- All cargo commands use RUSTFLAGS="-D warnings" CARGO_INCREMENTAL=0.
- Write tests per task, run integrated suites after tasks per repository guidance.

## Review focus

- Same OpenCode trace found in database and export contributes each event once.
- Failed/running calls and catalog listings cannot become successful definition loads.
- Children inherit exclusions and remain attributable to their actual parent.
- A new extractor version backfills unchanged retained children without duplicating totals.
- Clear/disable/exclusion during either extraction path rejects old writes/results.

## Task 1: Normalized OpenCode evidence

Owner: normalized_usage. Files: contributor-insights/src/insights/skill_usage.rs and focused sibling module if needed.
- [x] Define public extraction interface consuming source SessionEvent records and returning SkillUsageEvidence.
- [x] Match explicit skill calls and successful definition reads using structured IDs/status/timestamps.
- [x] Add fixtures for invocation/load separation, incomplete/failed calls, duplicate events and unsupported records.

## Task 2: Bounded daemon processing and aggregation

Owner: backend_coverage. Files: daemon/insights_week.rs, insights_skills.rs, background scheduling integration and dedicated helper modules/tests.
- [x] Enable child extraction behind flag; backfill on extractor-version change; retain parent references.
- [x] Add OpenCode skill-only processing using existing adapters without reclassifying token/spend analytics.
- [x] Deduplicate database/export copies; retain readable evidence drill references.
- [x] Add population and parent evidence fields to wire and notify UI owner before consumers change.
- [x] Cover retention, Never, clear/disable, incremental updates and partial/missing children in regression tests.

## Task 3: Native direct/delegated coverage

Owner: coverage_ui. Files: SkillsModels.swift, InsightsSkillsModel.swift, InsightsSkillsTab.swift, Skills copy and tests.
- [x] Decode additive fields conservatively and preserve older snapshots.
- [x] Show separate direct/delegated per-agent observations with expandable parent/child evidence.
- [x] Test no combined-count inflation, lineage rendering and partial/unsupported states.

## Task 4: Model extension and integration

Owner: root/flow reviewer.
- [x] Extend existing state model with parent/child lifecycle and incomplete/deduplicated normalized events; retain the five selected promises.
- [x] Generate charts from full model; run selected invariants and negative controls, document abstraction limits.
- [x] Run source/insights/contributor tests, FFI/native checks, lint/format and license boundary as relevant.
- [x] Independent review, fix substantive findings, rerun affected checks.
- [ ] Commit/push and update PR #2074 description and CI status.

## Ledger

- Clean existing isolated worktree, starting product commit ac8fc7ec7.
- Targeted already-merged reader prerequisites cherry-picked: #2071 as a15b61b64 and #2072 as 564bfc841. Unrelated IronWire vendoring commit not included.
- User approved both coverage extensions; no further design approval needed.

## Integration evidence

- Source-reader suite: 285 passed. Insights suite: 730 passed, 3 ignored.
- Broad contributor library run: 5,267 passed, 8 ignored; one new child backfill assertion compared opaque drill IDs across reanalysis. Corrected the test to compare all substantive fields while allowing those IDs to rotate; affected rerun: 150 passed, 3 ignored.
- Client lint and FFI build passed. First native selected run: 41 passed. Final checks after review: affected contributor 152 passed, 3 ignored; license boundary 4 passed; lint, format, FFI build and 41 selected native tests passed.
- Independent review found child drill references could bypass the Skills flag through the existing trace RPC. Fixed with a live scope check before/after child drill; regression covers never-enabled and enabled-to-disabled access, preserving ordinary parent drill. Reviewer verified the fix statically.
- Added real configured OpenCode export worker integration, including discovery/load/recheck, extraction, safe detail, clear and Never invalidation.
- Quint/TLC baseline: 74,288 states and 1,578,878 transitions; all five selected guarantees plus NoDeadlock hold. Seven negative controls detect expected violations. Model source hashes verified. This is bounded design verification, not product proof.
- Repaired inherited Windows-native provenance path handling without relaxing external manifest path validation. Updated two stale production plist digest fixtures to the existing approved digest; channel plist suite passed.
- No dependencies, rollout manifests, signing keys, billing/token logic, merge or deployment changes. OpenCode lineage stays unknown; existing Claude spawned traces are the supported delegated population.

- Final verification logs are local `/private/tmp/skills-coverage-review-{tests,clippy,ffi,fmt,swift}.log`, `/private/tmp/skills-coverage-license.log`, and `/private/tmp/skills-coverage-plist.log`. New-head hosted CI is a separate pending check at push time.
