# Witness Retry Fairness Implementation Plan

> **For agentic workers:** Use superpowers:executing-plans for the worker integration, with the independent transport-label task delegated. Repository instructions require one consolidated final test run.

**Goal:** Prevent one failed privacy review from starving unrelated approved traces while retaining bounded outage pacing and all privacy gates.

**Architecture:** Keep retry deadlines in queue entries, move shared outage detection into a deterministic circuit, and schedule the existing worker fairly. Preserve transport failure categories across the client pipeline.

**Tech Stack:** Rust, Tokio, chrono; existing dependencies only.

**Spec:** ../specs/2026-11-14-witness-retry-fairness-design.md

## Global Constraints

No new dependencies or UI notices. Preserve split licensing and all consent, trust, and send gates. Every cargo command uses RUSTFLAGS="-D warnings" CARGO_INCREMENTAL=0. Primary checkout remains untouched.

## Review Focus

- Repeated failures from one trace cannot manufacture correlated outage evidence.
- Jobs completing across a pause cannot reopen capacity early.
- Manual approvals arriving behind an existing queue must receive priority.
- Sleep, restart, cancellation, and untrusted/permanent outcomes cannot strand the worker.
- Classifier errors must remain retryable without weakening permanent privacy refusals.

## Tasks

- [x] Preserve classifier/redaction transport labels and update enum/registry/batch tests (delegated).
- [x] Add deterministic witness backoff and correlated-failure circuit with tests for threshold, expiry, single probe, recovery, and overlap.
- [x] Integrate independent entry retry handling, worker ordering, different recovery probe selection, and truthful existing health lifecycle. Add integrated regression tests for repeated failure plus healthy traces and retained safety gates.
- [x] Run consolidated contributor/protocol tests, license boundary, strict clippy/fmt, rebuilt FFI/native shell tests. Fix failures against the approved policy.
- [x] Fresh independent branch review, resolve substantive findings, commit and open a draft PR with exact validation. No merge or release is included in this implementation request.

## Decisions

Three distinct failures in five minutes and 30–300 second shared pauses are implementation defaults for the approved “several distinct traces in quick succession” policy. No extra approval gate is needed: the user explicitly requested implementation of the proposed policy.

Capacity correction: prior trace attempts formerly multiplied a server Retry-After up to an hour. The shared delay now follows the explicit hint; per-entry jitter remains. This is needed to satisfy the approved independent-retry and capacity policy.

## Validation record

- Contributor/protocol suite with `trace-commons-protocol/near-ai-privacy-filter`: 6,502 passed, 10 ignored, no failures.
- Strict contributor/protocol clippy (all targets, same feature; repository allow-list): passed.
- Server binaries and all server test targets: compiled successfully.
- Server license-boundary tests: 4 passed.
- Contributor FFI: rebuilt successfully before native tests.
- Formatting and diff whitespace checks: passed.

The first consolidated run found two new-test fixture problems: an unconstrained boolean closure argument, then a retry fixture missing its persisted failure label. Both were corrected; the full Rust suite above is the subsequent clean run. The reused worktree also contained Swift build metadata pointing at its former path; native validation uses a fresh scratch directory.

Native validation passed using the rebuilt FFI and `swift test --scratch-path /private/tmp/tc-witness-retry-swift-build`: 3,621 XCTest cases executed (10 skipped, zero failures), plus 8 Swift Testing cases passed. Apple Swift 6.4.2 emitted existing weak-capture warnings at `AccountLoginChangeViews.swift:28-29`; that file is byte-for-byte unchanged from the base commit. This was a normal native test run, not a warnings-as-errors Swift build.

## Review resolutions

The independent review found two scheduling defects: a pre-pause success arriving after saturation expiry could reopen both slots, and a canceled/reapproved queued entry retained its old snapshot and automatic priority. Both are repaired with regressions. The new failure labels retain the existing native interactive message and Rust/Swift pending-review classification; no wording was added.

A separate pre-existing outer-watchdog issue was identified: a first-attempt pipeline timeout can write a deadline without a retry reason, so the next poll may disregard the deadline. The normal witness transport has a shorter timeout and follows the retry policy implemented here. The unchanged outer 15-minute watchdog and its deferral marker require a separate repair; this branch does not claim that path is fixed.

Final post-review validation repeated the full contributor/protocol suite (6,502 passed, 10 ignored), strict clippy, FFI build, and full native suite successfully. Logs: `/private/tmp/tc-witness-review-{tests,clippy,ffi,swift}.log`.

Focused review of the fixes confirmed both Important findings closed and no new blockers within scope.
