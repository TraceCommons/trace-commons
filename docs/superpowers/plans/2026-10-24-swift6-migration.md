# Swift 6 Migration Implementation Plan

> **For agentic workers:** Use subagent-driven-development to implement the independent app, library, and test scopes, then integrate and verify them together.

**Goal:** Run every native macOS Swift target in Swift 6 language mode, resolve the observed concurrency diagnostics, and open a reviewable PR.

**Architecture:** Keep existing main-actor UI ownership and the synchronized Rust callback boundary. Make immutable geometry independent of actor isolation where correct, preserve Sendable callback contracts, and migrate the remaining app/app-test/demo targets. Shared native components already use Swift 6.

**Tech Stack:** SwiftPM, SwiftUI/AppKit, XCTest/Swift Testing, existing Rust FFI.

**Spec:** User-approved scope in this conversation: a separate migration PR while staging build 22 continues.

## Global Constraints

- Keep macOS 14 minimum, UI behavior and animations, Rust ABI, release channel identities, and existing dependencies.
- Resolve isolation and callback ownership explicitly; add no blanket unchecked Sendable, preconcurrency, or warning suppression to silence failures.
- Preserve the existing lock/lifetime guarantees of C callbacks and subscription teardown.
- Validate on the CI-supported Xcode/Swift toolchain as well as the available local compiler; do not rely on Swift 6.4-only APIs.
- Keep staging build 22's frozen source commit and installation task independent.

## Tasks

- [x] Flip the three remaining Swift 5 targets to Swift 6, reset relocated Swift artifacts, rebuild FFI, and capture compiler diagnostics as the failing baseline.
- [ ] Repair app callback/isolation diagnostics without changing event ordering or UI work placement.
- [ ] Repair design-library pure geometry and test actor isolation, with focused regressions for meaningful behavior changes.
- [ ] Repair app-test actor/discovery compatibility and the FFI demo under the stricter mode.
- [ ] Run clean builds, affected tests and the broad suite; check concurrency/lifecycle regressions and review independently.
- [ ] Add a useful CI guard for migrated targets, open a PR with exact validation results, and leave merging to explicit user authorization.

## Review Focus

- Native callbacks arriving from non-main threads must hand off UI updates safely and preserve teardown barriers.
- Refresh/subscription callbacks must not publish stale state after detach or cancellation.
- Geometry/alignment callbacks must remain pure and preserve measured layout.
- XCTest discovery must execute actor-isolated cases without unsafe casts or runtime crashes.
- Build guards must not raise deployment floors or rely on a newer toolchain than release CI.

## Initial PR checkpoint

Draft PR #1635 contains the language-mode switch, explicit app/bridge concurrency contracts, pure-geometry isolation fixes, initial test actor fixes, and warnings-as-errors CI guards. The migration is not complete. Finish compiler diagnostics, synchronize remaining test doubles without weakening assertions, run test discovery and the suite, verify Debug/Release and FFI teardown stress, and validate the CI-supported toolchain before marking it ready.

Local compiler: Apple Swift 6.4. The only installed Xcode is newer than release CI, so a local pass alone cannot establish CI compatibility. The existing `TCDaemon` condition-variable ownership gate has been independently reviewed to justify its narrow unchecked conformance; `TCPreview` remains non-Sendable.

The observer cleanup gates in `MenuBarTopEdgePin.Pin` and `WindowBreadcrumb.Probe` are repaired with a shared nonactor owner of immutable notification tokens and their registering center. It removes registrations on detach/rebind/release without accessing actor state in deinit. Compiler checking has reached app tests; lifecycle, preview, accessory, discovery, and broad-suite verification remain underway. No `@preconcurrency` suppression was applied.
