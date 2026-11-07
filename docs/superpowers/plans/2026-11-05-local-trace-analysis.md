# Local trace analysis implementation plan

> **For agentic workers:** Use superpowers:executing-plans inline.

**Goal:** Offer the approved local-analysis enable action in the Insights sample banner.
**Architecture:** Track explicit enabled state in InsightsModel and reuse AppModel's confirmed setting writer. Keep words in analytics_copy.rs and compose the existing banner in SwiftUI.
**Tech Stack:** Rust shared copy, SwiftUI, XCTest.
**Spec:** docs/superpowers/specs/2026-11-05-local-trace-analysis-design.md

## Constraints

Preserve collection and sharing settings. Unknown state does not mean disabled. Failed writes stay retryable. No new dependencies.

## Review focus

Explicit off versus unavailable; confirmed versus rejected writes; repeated clicks; already enabled; responsive banner and accessible action.

## Task

- [x] Add model regression tests for disabled/unavailable state and successful/refused enable writes; observe failure.
- [x] Add shared copy and wire model, banner and Monitor action to the existing confirmed setting writer.
- [x] Run focused Swift model/render tests, shared-copy Rust tests, and formatting checks; review diff.

## Verification

690 Rust Insights tests passed, 3 ignored. Swift: 9 render/copy tests and 25 final model/settings tests passed. Banner exports inspected at 400 and 760 points. FFI rebuilt from branch. Final independent review found no blockers.
