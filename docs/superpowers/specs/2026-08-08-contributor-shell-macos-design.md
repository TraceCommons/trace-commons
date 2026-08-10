# Contributor shell — macOS

Date: 2026-08-08
Status: approved for planning
Scope: sub-project 3. Platform mechanics only.
Reads with: `2026-08-08-contributor-shell-shared-design.md`, which carries all
flows, screens, and copy. Anything not named here is identical across
platforms by intent.

## Shape

**One application bundle. No second binary.** The app links the C-ABI library
from daemon core v1.1, calls `tc_daemon_start` at launch, and hosts the watch,
upload, digest, and history loops in-process. It also serves the control
socket, so `trace-commons-contributor daemon status` keeps working from a
terminal while the app runs.

This is the whole reason the process model changed. A separate daemon would
mean two things to code-sign, notarize, update, and keep in sync, and the
supported way to ship a real helper (`SMAppService.agent` with an embedded
LaunchAgent plist) is more machinery than being the app.

`LSUIElement = true`: menu-bar item, **no Dock icon**. macOS users expect
exactly this shape from a background utility, and a Dock presence is the tell
that something was ported rather than designed here.

## Technology

- **SwiftUI**, `MenuBarExtra` for the tray, a normal `Window` for the main
  surface. Minimum macOS 14.
- The C header from the FFI crate is exposed through a bridging header; a thin
  Swift wrapper turns the C surface into Swift types and is the only file that
  touches raw pointers. Every `char*` from the library is freed with
  `tc_string_free` in a `defer`.
- `tc_subscribe` delivers events on a background thread; the wrapper hops to
  `@MainActor` before touching any observable state.

## Login item

`SMAppService.mainApp.register()`. Users audit background software in System
Settings → General → Login Items and will look; registering there is what
makes the app legible rather than suspicious. Offer it at the end of
onboarding, not silently:

> **Start Trace Commons when you log in?**
> It needs to be running to notice finished sessions.
> [ Not now ]   [ Start at login ]

The app never writes a LaunchAgent plist directly. `daemon install` remains
Linux-only and stays out of the macOS path entirely.

## Notifications

`UNUserNotificationCenter`, with a category carrying exactly two actions:
`Review` (foreground, opens the window on the queue) and `Not now`
(destructive: false, dismiss only). **No action may upload.** Request
authorization at the end of onboarding with an explanation, not at first
launch.

The app sets `local_notifications: false` in daemon settings and renders
notifications itself, so it controls the actions. The daemon's `digest_due`
event is the trigger.

Respect Focus: `interruptionLevel = .passive` for the digest; the superseded
and queue-full notifications use `.active`.

## Windows and sheets

The review surface is a **sheet on the main window**, not a modal — the user
is inspecting one item in a list, and a sheet keeps that context. Search in
the preview uses the standard `.searchable` treatment so the keyboard shortcut
is what a Mac user expects.

## Packaging

Developer ID signed, hardened runtime, notarized, stapled. Distributed as a
notarized DMG. Sparkle or equivalent for updates is out of scope for v1; the
first release can be a manual download.

No entitlements beyond the default set are required: the app reads files under
the user's home, opens a unix socket in its own config directory, and makes
outbound HTTPS connections. **App Sandbox is not enabled**, because reading
`~/.claude/projects` and `~/.codex/sessions` is the product and a sandboxed app
cannot without user-selected file access for each. This must be stated plainly
in release notes rather than discovered.

## Acceptance

The shared checklist, plus: no Dock icon appears; the app shows in Login Items
after opting in; `daemon status` in a terminal answers while the app runs;
quitting the app with "Quit and stop watching" releases the lock so a
subsequent `daemon run` succeeds; notification actions are exactly Review and
Not now.
