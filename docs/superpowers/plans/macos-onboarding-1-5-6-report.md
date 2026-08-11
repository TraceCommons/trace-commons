# macOS onboarding: screens 1, 5, 6 + credit framing

Scope: build onboarding screens 1 ("What this is"), 5 ("What to watch"), 6
("Done"), and the reusable credit-framing view, per
`docs/superpowers/specs/2026-08-08-contributor-shell-shared-design.md`
("## Onboarding" screens 1/5/6, "## Credit, framed honestly"). Screens 2 and
4, and the flow chaining all six, are explicitly out of scope for this pass.

## What was built

- `macos/Sources/TraceCommonsApp/Views/OnboardingWelcomeView.swift` — screen
  1. `OnboardingWelcomeView` (ScrollView wrapper) / `OnboardingWelcomeContent`
  (layout), following the `ConsentScopesView` / `ConsentScopesContent` split
  so `ImageRenderer` can rasterize it. Copy is verbatim from the spec,
  including "That scrubbing is good and it is not perfect — which is why you
  get to look first." Two buttons: "Get started", "What gets removed?".
- `macos/Sources/TraceCommonsApp/Views/OnboardingProjectsView.swift` —
  screen 5. `OnboardingProjectsView` / `OnboardingProjectsContent`. Project
  list comes from `AppModel.projects` (`DaemonClient.listProjects()` /
  `list_projects`), never a hardcoded array. Every project starts ask-first;
  an `Ignore` toggle is offered per project. `auto_upload` is not offered —
  there is no `set_project_mode` daemon call yet (confirmed: `SettingsView`
  only ever reads `ProjectMode`, never sets it), so this screen holds the
  ignored-label set as local `@State` and hands it to the caller via
  `onContinue(Set<String>)`, mirroring how `ConsentScopesContent` hands back
  selected scopes rather than calling the daemon itself. A permanent,
  plain-English note covers sessions that never resolve to a project ("can
  never be set to upload automatically") — no filesystem path is rendered.
- `macos/Sources/TraceCommonsApp/Views/OnboardingDoneView.swift` — screen 6.
  `OnboardingDoneView` / `OnboardingDoneContent`. First line is exactly
  "You're set up. Nothing has been sent.", followed by the menu-bar
  explanation from the spec, verbatim.
- `macos/Sources/TraceCommonsApp/Views/CreditRecordView.swift` — new
  reusable `CreditRecordView`, taking `creditFinal: Double`,
  `creditPending: Double`, `lastRefreshedAt: Date?`. Verbatim "About credit."
  copy; no currency symbol, no fiat estimate, no projection, no date beyond
  what's already in the copy, no streaks/leaderboards/progress rings.
  `lastRefreshedAt == nil` renders "Not synced yet" rather than a confident
  `0.0`.
- `macos/Sources/TraceCommonsApp/Views/HistoryView.swift` — refactored its
  private `credit(_:)` to call the new `CreditRecordView` instead of
  duplicating the same copy inline, since the spec requires the two call
  sites (onboarding, History) never drift.
- `macos/Sources/TraceCommonsApp/DebugScreenshot.swift` — added render calls
  for all four new views, writing to `TRACE_COMMONS_SCREENSHOT_DIR`.

Nothing under `crates/` was touched. No new files were added under existing
large binaries; each screen is its own file, consistent with the repo's
"add new modules beside existing code" convention.

## Verification

- `swift build` (from `macos/`): succeeds, `Build complete!`.
- `RUSTFLAGS='-D warnings' cargo check --workspace --bins`: succeeds
  (`Finished` dev profile), confirming the Rust side is untouched and green.
- `./scripts/make-app-bundle.sh` was run explicitly (not left to
  `run-demo.sh`'s "only rebuild if missing" shortcut) before capturing, so
  the screenshots below are the current code, not stale bundle contents.
- `./scripts/run-demo.sh` with `TRACE_COMMONS_SCREENSHOT_DIR` and
  `TRACE_COMMONS_QUIT_AFTER_SHOT=1` rendered all four new views against the
  live fixture daemon. All four came out non-blank on the first pass — none
  hit the `ScrollView`-renders-blank problem, because each screen follows
  the split-content pattern from the start.

Screenshots (all under `docs/images/`, repo-relative from
`.worktrees/macos-shell`):

- `docs/images/macos-shell-onboarding-welcome.png` — screen 1.
- `docs/images/macos-shell-onboarding-projects.png` — screen 5. Shows "No
  projects discovered yet" because the demo fixture's two sample sessions
  had not cleared the daemon's quiescence window by the ~12s capture mark —
  this is real (empty) daemon data, not a placeholder string, and is the
  correct behavior for `list_projects` returning nothing yet.
- `docs/images/macos-shell-onboarding-done.png` — screen 6.
- `docs/images/macos-shell-credit-record.png` — the reusable credit view,
  rendered standalone. Shows "Not synced yet" because the demo's
  `HistoryRollup.lastRefreshedAt` was still null at capture time — also real
  data, not a placeholder.

## What did not survive reading the spec as originally scoped

- Nothing in the assigned screens (1, 5, 6) or the credit section required
  deviating from the spec's verbatim copy. The one design decision not
  explicitly dictated by the spec was how screen 5 reports its "Ignore"
  choice back to a caller, given no `set_project_mode` daemon call exists —
  resolved by following the existing `ConsentScopesContent` pattern
  (local state + `onContinue` callback) rather than inventing a new one or
  silently no-oping a button.
- The credit view was built as a new shared component rather than a second
  copy, and `HistoryView` was refactored to use it — this was implied by
  "as a reusable view" in the task but is a small edit to an existing file
  outside the three named screens; noting it here since it wasn't a
  brand-new file only.
