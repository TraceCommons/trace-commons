# macOS consent-scopes screen — build report

## What was built

`macos/Sources/TraceCommonsApp/Views/ConsentScopesView.swift`: the
"How may your traces be used?" onboarding screen (shared design spec,
"### 3. Consent scopes").

- `ConsentScopesView` — public entry point, wraps the content in a
  `ScrollView`.
- `ConsentScopesContent` — the actual layout, split out of the `ScrollView`
  for the same reason `QueueContent` is split out of `QueueView`:
  `ImageRenderer` renders a `ScrollView` as blank, and this screen is too
  consequential to ship unverified.

Copy is verbatim from the spec section. The scope list (`name`,
`description`, `always_on`, `grants_data_use`) comes from
`AppModel.consentScopes`, which is populated from the daemon's
`consent_options` call — no hardcoded scope list in this file. The one
Swift-side literal mapping used for the short bold titles,
`ScopeCopy.title(for:options:)`, already exists in `Views/PreviewSheet.swift`
and is shared with `SettingsView`; it was reused rather than duplicated,
since a second copy would be exactly the drift the task warned against.

The three rules, each with its reasoning comment in the source:

1. Two visually distinct groups — "Always included" (the `always_on` scope)
   and "Optional — each one lets your traces do more" (the rest of the
   data-use scopes).
2. Nothing optional starts checked. `@State private var selected: Set<String>`
   starts empty; only a tap adds a name to it.
3. `public_attribution` (the one scope with `grants_data_use == false`) is
   pulled into its own "Credit" section below a divider, keyed off
   `grantsDataUse`, not off the scope's name.

The continue button reads "Continue with N permission(s)" where N is the
always-on count plus the live selection count, updating on every tap.

The "To pull a trace back later, use History → Withdraw." line is rendered
verbatim below the groups.

`onContinue: (Set<String>) -> Void` is exposed as a closure parameter
(default no-op) so a future onboarding flow can wire it up; nothing beyond
this screen was built, per the task's stop condition.

## Verification

- `swift build` — succeeded (`Build complete!`), twice: once for the initial
  view, once after the `ConsentScopesContent` split.
- `RUSTFLAGS='-D warnings' cargo check --workspace --bins` — succeeded.
  Nothing under `crates/` was touched.
- Screenshot: rendered via the existing `DebugScreenshot.swift` hook
  (`TRACE_COMMONS_SCREENSHOT_DIR` + `TRACE_COMMONS_QUIT_AFTER_SHOT=1`, driven
  through `macos/scripts/run-demo.sh` against fixture sessions). Saved to
  `docs/images/macos-shell-consent-scopes.png`.

  First attempt produced a blank white image — the same failure mode the
  `DebugScreenshot.swift` header already documents for `ScrollView` content
  under `ImageRenderer`. `ConsentScopesView` originally had no non-`ScrollView`
  content root, so it hit the same problem `QueueContent` was split out to
  avoid. Fixing that (splitting out `ConsentScopesContent`, same shape as
  `QueueContent`) produced a correctly rendered screenshot with real text,
  checkboxes, and the "Continue with 1 permission" button — no path names
  anywhere on screen.

  Also caught mid-verification: `macos/scripts/run-demo.sh` only rebuilds
  the `.app` bundle when it does not already exist (`.build/TraceCommons.app`
  was stale from a previous session), so the first screenshot run was
  silently exercising old code. Re-ran `scripts/make-app-bundle.sh`
  explicitly before each subsequent attempt.

## What did not survive reading the spec

Nothing. The spec section ("### 3. Consent scopes") was read in full and the
copy strings, group structure, and continue-button behavior were taken
directly from it.

## Files touched

- `macos/Sources/TraceCommonsApp/Views/ConsentScopesView.swift` (new)
- `macos/Sources/TraceCommonsApp/DebugScreenshot.swift` (added one render
  call for the new screen, mirroring the existing three)
- `docs/images/macos-shell-consent-scopes.png` (new)
- `docs/superpowers/plans/macos-consent-screen-report.md` (this file)
