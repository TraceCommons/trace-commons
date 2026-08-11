# macOS login-item registration -- report

Scope: `SMAppService.mainApp` registration for the macOS contributor shell,
offered at the end of onboarding, plus a Settings toggle reflecting live
status. Per
`docs/superpowers/specs/2026-08-08-contributor-shell-macos-design.md`,
"## Login item".

## What was built

- `macos/Sources/TraceCommonsApp/LoginItemManager.swift` (new): a thin
  `@MainActor` wrapper around `SMAppService.mainApp`. `currentState` reads
  `SMAppService.mainApp.status` fresh on every call (never cached), mapped to
  a local `State` enum (`notRegistered`, `enabled`, `requiresApproval`,
  `notFound`) so callers don't need to import `ServiceManagement`.
  `register()` / `unregister()` call through and report a
  `RegisterOutcome`/`UnregisterOutcome` rather than throwing, so
  `.requiresApproval` can be rendered as guidance instead of an error.

- `macos/Sources/TraceCommonsApp/Views/OnboardingDoneView.swift`: added the
  offer to `OnboardingDoneContent`, verbatim spec wording -- "Start Trace
  Commons when you log in? It needs to be running to notice finished
  sessions." with "Not now" / "Start at login" buttons. Shown only when the
  app is not already an enabled login item. After a choice, shows one of
  three outcome sentences: started successfully, needs approval in System
  Settings -> General -> Login Items (not treated as an error), or a plain
  failure message. `ImageRenderer` (used by `DebugScreenshot` for
  `OnboardingDoneContent`) never fires button taps, so the screenshot path
  only reads `currentState`, never calls `register()`.

- `macos/Sources/TraceCommonsApp/Views/SettingsView.swift`: added a
  "Startup" section above Consent. Renders a `Toggle` bound to live
  `SMAppService` state (`@State private var loginItemState`, refreshed in
  `.onAppear` -- not a cached bool, since the user can flip this from System
  Settings while the window is open). `.requiresApproval` renders as text
  pointing at System Settings -> General -> Login Items, not a toggle, since
  there is nothing this app can flip on the user's behalf from that state.
  Toggling off calls `unregister()`; a failure on either direction surfaces a
  plain-language message under the control, not a silent no-op or retry.

- No LaunchAgent plist is written anywhere. `daemon install` was not touched
  and stays out of the macOS path entirely, per the spec's explicit
  instruction.

## Constraints honored

- Nothing under `crates/` was touched (`git status` shows only the three
  `macos/` files below).
- `RUSTFLAGS='-D warnings' cargo check --workspace --bins` passed (this
  worktree's Rust surface is unaffected by a Swift-only change).
- No emojis in code or this report.

## Verified

- `swift build` -- succeeds, compiles `LoginItemManager.swift`,
  `OnboardingDoneView.swift`, and `SettingsView.swift` cleanly, no warnings
  from the new code.
- `macos/scripts/make-app-bundle.sh` (run explicitly, not via `run-demo`'s
  "only rebuild if missing" shortcut) -- succeeds, produces
  `.build/TraceCommons.app` with the new binary and dylib re-copied and
  re-signed (ad hoc, `codesign --sign -`).
- Launched the real bundled executable directly
  (`.build/TraceCommons.app/Contents/MacOS/TraceCommonsApp`, not via `open`)
  for a bounded few seconds against the live WindowServer session on this
  machine. Confirmed via `log show --predicate 'process == "TraceCommonsApp"'`
  that it starts cleanly as a menu-bar (`LSUIElement`) app: status-item
  scenes are created, `UNUserNotificationCenter` authorization is requested
  (denied in this headless launch, which is expected outside Finder/LaunchServices
  invocation and orthogonal to this task), and there is no crash.

## Not verified

- Did not click through onboarding to actually reach the Done screen and
  press "Start at login," so `SMAppService.mainApp.register()` was never
  exercised end-to-end in this session and I did not observe a real
  `.enabled` / `.requiresApproval` / error outcome. Reaching that screen
  needs a live invite/enrollment flow, which is outside this task's scope
  ("do not build anything else") and outside the tools available to me here
  (no UI-automation tool was loaded for this task). This is the honest gap:
  the code path is exercised by `swift build` and by launching the real
  bundle, but the actual `SMAppService` registration call has only been
  read-reviewed against Apple's documented behavior, not run.
- `sfltool dumpbtm` (system-wide login-item/background-task dump, which
  would have shown live registration state independent of the app) hung
  past a 120s timeout in this environment -- likely wants an interactive
  authorization prompt that isn't answerable headlessly -- and was
  abandoned rather than worked around.
- Ad-hoc signing means this has not been tested as it would behave under a
  Developer ID + notarized build. `SMAppService` is documented to be more
  permissive about ad-hoc/unsigned local builds during development, but
  that is Apple's documented behavior, not something this session
  confirmed empirically.

## Files touched

- `macos/Sources/TraceCommonsApp/LoginItemManager.swift` (new)
- `macos/Sources/TraceCommonsApp/Views/OnboardingDoneView.swift`
- `macos/Sources/TraceCommonsApp/Views/SettingsView.swift`
