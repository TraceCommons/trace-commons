# Question-card validation matrix

This records the deterministic question-card slice, not completion of the full
[Insights program](2026-09-11-trace-insights-program.md). The
[implementation plan](2026-09-11-insights-deterministic-question-cards.md) defines
the common contract and platform requirements. No release is qualified by this
matrix yet; Astra review and platform CI remain outstanding.

## Implemented behavior

The CLI and three native views request cards from explicitly selected saved
snapshots and episode groups. Empty selection means no evidence. The shared
service resolves the selection from one locked store view and validates every
returned card against its versioned deterministic calculation.

Cards describe recorded activity, reported episode outcomes, observed model
labels, and unavailable cost. Missing facts stay distinct from zero. Episode
overlap is visible; tool failures do not establish code rejection. Recorded
spans include idle gaps and do not establish active work or time saved. The
first-party provider remains the product's selected implementation.

## Evidence by layer

| Layer | Local evidence | Outstanding qualification |
| --- | --- | --- |
| Shared Rust service and provider | Warning-denied workspace tests at `68f58850`: 5,567 passed, 0 failed, 21 ignored in 108 suites. Fourteen protocol card tests, eight projector/dispatch tests, CLI lifecycle test, and contributor all-target Clippy passed. | Current-head CI and Astra review. No independent provider installation or isolation is implemented. |
| Shared follow-up | `b38b692e` / canonical `2c54c70e`: concurrent read/membership mutation regression and contributor Clippy passed. Added common native selection copy. Fresh FFI build passed. | This follow-up was checked with focused tests, not a repeated full workspace run. |
| FFI | Sol's complete FFI suite: 169 tests passed. New tests cover empty/missing/duplicate selections, saved-evidence binding, a 1001 ms span, model metadata, unavailable cost, and private-content exclusion. | Platform-native paths and final stack CI remain separate checks. |
| macOS | Final `c4ffa23e` + `ac8aa126`: 911 XCTest cases executed, 1 skipped, 0 failures; Swift Testing 8 passed. Focused card/wording tests 10 passed. Current FFI empty-store and shared-copy roundtrip verified. Release SwiftPM app build passed. | macOS CI on the published stack; universal application bundle, signing, packaging, and interactive visual acceptance were not run. |
| Windows | `2c539fd2`: macOS-hosted managed/native bridge suite 1,133 passed, 1 Windows-only skipped. Final `55f735c1` shared-copy follow-up: six focused card tests passed. XAML is XML-valid. | WinUI compilation and control lifecycle require actual Windows CI. The macOS compiler attempt failed at Windows `XamlCompiler.exe` with code 126. Packaging and interactive visual acceptance were not run. |
| GTK | `ca9d5d38`: library tests 529 passed, 0 failed, 2 display tests ignored. Focused Insights tests 9 passed, 0 failed, 1 display test ignored. Formatting and Clippy with the existing allowance passed. The display fixture now requests cards and checks rendered evidence controls. | Actual Weston/portal display CI must execute the ignored lifecycle test. Packaging and interactive visual acceptance were not run. |

Test counts describe their executed suites, not a count of product stories.
Native tests cover decoding, selection, stale-result rejection, and evidence
invalidation to different depths. Passing these tests does not establish that
every interactive combination has been exercised.

## Remaining program work

- Persist native usage with per-model attribution and versioned pricing before
  displaying cost estimates. A source's cumulative token total cannot be
  allocated to model labels by guesswork.
- Define and calibrate matched task comparisons before claiming a model is best
  for refactors, tests, documentation, or rejected-code outcomes.
- Establish a measurement design and baseline before claiming time saved.
- Implement prompting coaching, provider installation and permissions, optional
  hosted analytics, and the mission/scout workflow in the full program plan.
- Complete Astra review, resolve findings, and verify platform gates before
  calling this slice ready to merge or release.
