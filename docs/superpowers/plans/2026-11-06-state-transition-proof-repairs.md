# State-transition safety repairs

> **For agentic workers:** Execute the scoped tasks in parallel where files are independent. Use systematic debugging, test-driven development and verification before completion. This continues the owner-selected flow-promise work; the owner authorized investigating and fixing all findings.

**Goal:** Fix reproduced state-publication races and complete reliable verification of all 13 selected safety hypotheses, addressing further source defects only after source reproduction.

**Architecture:** Native account/client and per-resource read generations fence obsolete replies before publication or error bookkeeping. Consent mutations invalidate overlapping reads. Proof-only type domains and inductive strengthening establish initiation, full original-step preservation and implication without changing model behaviors to improve tractability.

**Tech Stack:** Existing Swift 6 app/tests, Rust FFI, Quint 0.33.0, Apalache 0.62.1 and pinned TLC/flow-promises tools. No new application dependency.

**Spec:** `docs/flows/trace-commons/promise-candidates.md`, `proofs/results.md` and the source correspondence notes. The user's “investigate and fix all of these” authorizes repairs, preserving the selected promise wording.

## Constraints and review focus

- Work only in the existing isolated `quint-client-server` worktree; preserve the dirty primary checkout.
- Do not weaken promises, remove original model transitions, reduce identity domains or impose exploration constraints to obtain passing verdicts.
- Complete induction requires initiation, consecution over the full original step, and implication. A domain/strengthening predicate belongs only to proof obligations and must itself be preserved.
- Preserve historical counterexamples. Update source-method replays into passing regressions and retain red evidence.
- Cover old-account replies, account ABA, disconnected/replaced clients, same-account out-of-order reads, overlapping consent writes, daemon grant-change notifications, and stale failures/loading state.
- Use existing UI, with no new notices/copy or blocking unrelated controls.
- Rust commands use `RUSTFLAGS="-D warnings" CARGO_INCREMENTAL=0`. Build exact current FFI before the native suite.
- Confirm source/model correspondence after changes and obtain independent review before publishing. Do not merge or release without authorization.

## Tasks

### 1. Native publication fencing

- [x] Reproduce P1/P12 against the actual app with controlled IPC scheduling; regression tests fail before the fix.
- [x] Add minimal account/client and latest-read fencing to history and grant reads, including consent write invalidation and stale error handling.
- [x] Run focused red/green tests; run the complete native Swift suite at integration with the rebuilt FFI.

Owner: native cleanup agent. Primary files: `macos/Sources/TraceCommonsApp/AppModel.swift`, related native regression tests. Coordinate semantics with client model owner.

### 2. Client consent/contribution verification

- [x] Reflect native freshness fixes in `client_consent.qnt` and `client_contribution.qnt` without disabling background or account transitions.
- [x] Add proof-only finite domains/inductive relations as required; complete P1/P2/P3/P4/P12 checks against the original full step.
- [x] Validate observer fault controls and source-method regressions; retain the old failing traces as historical evidence.

Owner: client model agent. No edits to shared results/manifest.

### 3. Receipt and pipeline verification

- [x] Investigate incomplete P5/P6/P7/P8/P11 obligations and construct complete proof obligations over unchanged full steps.
- [x] Prove typed-domain coverage and invariant preservation; document structured-result and external-authority assumptions.
- [x] Source-replay any new reachable counterexample before repairing production code.

Owner: server model agent. Files: receipt/pipeline models and proof harnesses.

### 4. Settlement/export verification

- [x] Complete P9 and P10/P11 initiation, original-step consecution and implication.
- [x] Retain idempotency/provider and database atomicity assumptions explicitly; no external exactly-once claim.
- [x] Source-replay and fix any newly confirmed defect with a regression.

Owner: settlement/export agent. Files: settlement/lifecycle models and proof harnesses.

### 5. Skill installation verification

- [x] Complete P13 with all original skill transitions and identity bounds.
- [x] Prove confirmed lineage and reconciliation preservation; retain filesystem/crypto boundary assumptions.
- [x] Run positive and fault-injected proof controls.

Owner: skill-install agent. Files: learning model and proof harnesses.

### 6. Integration and publication

- [x] Independently review proof obligations, domain coverage and model/source alignment; reject vacuous or bounded-only universal claims.
- [x] Run final structural verification once after integration, targeted proof/fault controls and full native Swift tests; diagnose actual failures.
- [x] Regenerate result tables, coverage/evidence hashes and review surface. Check historical/current status is unambiguous.
- [x] Prepare the PR #2017 update with final code, actual proof results and CI tracking; keep merge and release separate.

## Final verification

All 13 model hypotheses have complete, independently reviewed induction certificates. Original transition coverage, domain closure and nonvacuity were checked. Native fixes use retained identity epochs, scoped status publication, grant notification refresh and initial owner preflight. The full Swift suite passed with normal AppKit/Vision access; 24 scoped regressions and four license-boundary tests passed. Historical failures and tool limits remain documented in flow validation. The final PR update is prepared for publication.
