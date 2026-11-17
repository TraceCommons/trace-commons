# Skills Home Default Creators Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Home has useful default creator content without silently following anyone or weakening evidence rules.

- [x] Add daemon regression tests: empty explicit follows exposes three defaults and their existing public events; no fake events on baseline; explicit follows replace defaults; following state unchanged; disabled analysis/feature stays closed; background refresh serves defaults.
- [x] Implement default selection and separately bounded home_feed in insights_skills.rs. Preserve existing feed semantics.
- [x] Add backward-compatible Swift snapshot fields and regression tests for default cards/news, explicit follows, missing catalog rows, older snapshots and partial usage. Build the existing Home creator card styling and Follow actions with shared short copy.
- [x] Verify meaningful focused daemon and Swift suites after both tasks; keep RUSTFLAGS=-D warnings and CARGO_INCREMENTAL=0 and use shared compile/test locks. Check formatting and license boundary. No dependencies.
- [x] Replay the retained staging snapshot/public cache through the new projection to establish that this machine gets real Home content. Review the combined diff and report source/build/install states separately.

## Verification

- Expected RED: three daemon default-feed/refresh tests and two Swift baseline/news regressions failed before implementation.
- GREEN: 62 Skills daemon tests passed, one explicit live-network test ignored; five shared-copy tests passed; four license-boundary tests passed.
- Native: 58 focused Swift tests passed against the rebuilt FFI, including the retained staging snapshot; a render-only rerun passed after matching the app ScrollView in the test host.
- Retained replay: 3 default creators, 9 existing public catalog examples, 5 actual recorded changes. Explicit follows remain empty and partial usage does not create weekly totals/trends.
- Clippy on both changed Rust packages/all targets, Rust formatting and diff whitespace pass. No dependencies changed. Independent review found no blockers.
- Source implementation only: no app installation, relaunch, release publication or account/feature-flag changes. The installed staging app still needs a build containing this change.

## Catalog-budget review follow-up

A regression with sixteen valid 16 KiB Obra descriptions reproduced the review finding: Poteto was omitted by the 180 KiB bound. Oversized responses now reserve one real example per default creator before the remaining rows; catalogs that fit preserve their original active/installed ordering. The same byte budget and explicit follow state are retained. All 63 Skills daemon tests pass, one live-network test remains ignored; the existing server-catalog ordering case passes as well.
