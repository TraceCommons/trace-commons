# Schema-aware privacy keys Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan.

**Goal:** Remove standalone-key false positives while preserving all real privacy holds.
**Architecture:** A compiled public-literal catalog plus contextual unknown-key inspection in the manual-review traversal; private classifier spans feed bounded trusted hash-only evidence.
**Tech Stack:** Rust, serde_json, existing SHA-256, existing privacy adapters.
**Spec:** docs/superpowers/specs/2026-11-15-schema-aware-privacy-keys-design.md

## Global Constraints

Keep permissive protocol independent of AGPL code. No new dependencies. No caller schemas or hashes authorize skipping data. Every value remains scanned. Preserve keys, identity, consent, corrections, atomic errors and all approval gates. Use RUSTFLAGS="-D warnings" CARGO_INCREMENTAL=0 on every cargo invocation.

## Review Focus

- Tool aliases, nested parameters and caller schema claims cannot broaden literal recognition.
- JSON escaping, Unicode and out-of-token findings must not be mapped to the wrong field.
- Missing backend evidence or malformed provenance cannot become a clean key.
- Private spans and raw dynamic names never serialize into diagnostics.
- Feature-disabled, self-hosted and existing stored evidence remain compatible.

## Task 1: Integrated reviewed-key inspection

- [x] Write regression fixtures for public-literal recognition, JSON-context classification, real findings, invalid evidence and hash-only diagnostics; run the focused new tests red before implementation.
- [x] Add `trace_contribution/key_privacy.rs` for compiled catalog, contextual inspection and bounded diagnostics. Extend private span provenance without changing serialized envelopes.
- [x] Integrate the review-only event and side-field traversals and fresh ReviewPrivacyRecheckEvidence. Update fixture constructors with absent diagnostics for backward-compatible evidence.
- [x] Run protocol suites under relevant features and compatibility/license/lint/format checks once after implementation. Fix failures and run focused verification for subsequent repairs.
- [x] Request an independent whole-diff security review, resolve findings, and record the actual validation and remaining limitations. Keep production unchanged.

## Ledger

Baseline: existing manual-review tests 25 passed on origin/main 919542924. Isolated worktree created with .claude/bin/tc-worktree; no eligible warm donor existed. Independent architecture review approved narrow public-literal catalog; it does not authenticate tool origin. User's explicit build request authorizes executing the agreed mitigation without another design confirmation.

Red verification: both new `schema_key_` regressions failed on current code for the expected false-positive holds. Kache is the active user-wide wrapper; all further compilation and heavy test execution use existing shared-machine turn locks.

## Final verification

- Combined near-ai-privacy-filter + self-hosted-privacy-filter protocol suites: 787 unit tests and 98 integration tests passed, including 32 generated held-out names and replayed public model controls.
- Default-feature and self-hosted-only protocol test checks passed.
- Server and ingest binary checks, all test compilation, and ingest test compatibility check passed.
- Server approval-evidence unit tests: 3 passed. License boundary: 4 passed; expected sets unchanged.
- Repository-prescribed Clippy checks passed for protocol/server/ingest all targets with documented lint allowances. Repository-wide cargo fmt and git diff whitespace checks passed.
- Independent whole-diff security review found one feature-gating blocker, which was corrected; follow-up review found no remaining blockers. Recorded-control test-only addition was reviewed separately.

Actual public-only classifier controls confirmed standalone fork_turns/explanation false positives become clean in synthetic JSON, while synthetic person-name JSON keys remain flagged. Those samples do not establish exhaustive or future model accuracy. No private trace was reuploaded, no live assessment or pricing policy was changed, and this implementation was not deployed.

Rulings: the catalog recognizes public syntax, never authenticated tool origin. All key values and descendants retain their existing scans. An end-to-end fixture had incorrectly expected private text to reach the model after deterministic unknown-tool scrubbing; that assertion now accepts prior sanitization, with a separate direct-walker test proving value scanning independently. The existing near-ai-only spans_fit helper was gated to its owning feature so self-hosted-only checks pass. Native confidence absence is retained as unavailable, not fabricated zero.
