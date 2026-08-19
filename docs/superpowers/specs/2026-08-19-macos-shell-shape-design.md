# macOS shell shape — Dock icon, and the mark that never drew

Date: 2026-08-19
Status: design, not yet implemented.
Scope: sub-project C of three. macOS shell surfaces only.
Reads with: `2026-08-08-contributor-shell-macos-design.md`, whose `LSUIElement`
decision this reverses, `2026-08-17-desktop-clients-design.md` for the mark
itself, and `2026-08-17-onboarding-parity-windows-linux.md` for the deep-link
contract Part 3 has to keep.

## The problem

Three defects in the same surface, found while installing the notarized 0.3.0
DMG on macOS 26.5.2 (build 25F84).

**The menu-bar mark draws nothing.** The status item exists, is laid out, and
is fully described to accessibility — it simply has no pixels. A contributor
who installs the app has no way to find it, because the one affordance the
whole shell hangs off is invisible.

**There is no Dock icon, and no icon at all.** `LSUIElement` is set, so the
app has no Dock presence by design. It also ships no artwork of any kind:
`macos/scripts/make-app-bundle.sh:86` creates `Contents/Resources` and
nothing is ever copied into it, and `macos/scripts/info-plist.sh` sets no
`CFBundleIconFile`. Those are separable — the first is a choice, the second
is a gap — and this slice closes both.

**Invite deep links are dead.** `OnboardingConnectView.swift:78` handles
`.onOpenURL`, and `macos/scripts/info-plist.sh` declares no
`CFBundleURLTypes` — `grep -c CFBundleURLTypes macos/scripts/info-plist.sh`
returns 0. Nothing registers `tracecommons://` with LaunchServices, so no URL
event is ever delivered and the handler is unreachable code. A contributor
who clicks an invite in mail gets nothing.

## What this reverses

`2026-08-08-contributor-shell-macos-design.md`, "## Shape", is explicit:

> `LSUIElement = true`: menu-bar item, **no Dock icon**. macOS users expect
> exactly this shape from a background utility, and a Dock presence is the
> tell that something was ported rather than designed here.

Its acceptance list opens with "no Dock icon appears". That decision is being
overturned deliberately, not drifted away from, and this document is the
record of the reversal. The argument that displaces it is discoverability:
the shape is only correct if the menu-bar item can actually be found, and on
the shipped build it cannot be seen at all. A background utility whose sole
surface is invisible is not a background utility, it is a missing app.

The user chose to keep **both** surfaces over Dock-only and over a
user-settable preference, and parity is part of why. The GTK client carries a
tray item (`crates/trace-commons-contributor-gtk/src/tray.rs`) and the Windows
client carries a notification-area icon
(`windows/src/TraceCommons.Interop/MarkRaster.cs`), both alongside an ordinary
windowed presence. Dropping the macOS status item would make macOS the only
platform with no at-a-glance state, and would leave `decisionsOwed` — the one
countable figure in the chrome — with nowhere to live.

The old spec's sentence is not wrong about macOS convention. It is
outweighed. Update it in place rather than leaving two specs that contradict
each other.

## What this is not

Not sub-project A (fail-closed roots parity and the macOS state-directory
resolution) and not sub-project B (the cross-platform icon pipeline). Those
have their own specs. C changes how the app presents itself and how it can be
reached; it changes nothing about what the daemon watches.

Part 3 is not the onboarding port either. The screens, the copy, and the
enrolment calls are settled in
`2026-08-17-onboarding-parity-windows-linux.md`. C only makes macOS able to
receive the link that opens them.

## Part 1 — the invisible mark

### What is established

Re-verified against the running 0.3.0 bundle, not taken from the earlier
session:

- The process is `/Applications/TraceCommons.app/Contents/MacOS/TraceCommonsApp`,
  `CFBundleShortVersionString` 0.3.0, `LSUIElement` 1.
- `System Events` reports exactly one item on menu bar 2, described as
  `status menu`, with a real frame of 18 x 24 points.
- Its `AXTitle` is "Trace Commons. Nothing waiting." — the
  `decisionsOwed == 0`, `health == nil`, not-paused branch of
  `MenuBarLabel.accessibilityLabel`
  (`macos/Sources/TraceCommonsApp/Views/MenuBarView.swift:51-58`).
- A screen capture of the item's own frame is empty desktop background. The
  nearest drawn pixels belong to the next application's status item, past the
  item's right edge.

So `MenuBarExtra` is installed, the label's layout runs, the state machine
resolves, and accessibility is served. Only rasterization fails.

Do not pin an x coordinate in a test or a runbook. The item was observed at
x=896 and later at x=893 in the same session; status-item origins move when
any neighbour changes width.

### Why this state is the least informative one

`MenuBarLabel` (`macos/Sources/TraceCommonsApp/Views/MenuBarView.swift:25-49`)
is an `HStack` of the mark plus **at most one** trailing glyph, and the
trailing glyph is present only when there are decisions owed, or health is
bad, or the app is paused. The `AXTitle` proves the app is in none of those
states. The label therefore currently contains nothing but
`BrandMark(size: 15, variant: .template)`.

That matters for diagnosis: the observation "nothing renders" cannot yet
distinguish between the mark failing and the whole label failing, because in
this state the mark **is** the whole label. Any bisect has to break that tie
explicitly.

### The hypothesis, and why it is only that

`BrandMark`'s `.template` variant is the only variant that draws no
background: the `Rectangle().fill(TC.surface)` and the frame stroke are both
inside a `variant != .template` guard
(`macos/Sources/TraceCommonsApp/Views/BrandMark.swift:66-69`), leaving just
two stroked `Bracket` shapes (`BrandMark.swift:70-71`) whose ink resolves to
`.primary` in this variant and only this one (`BrandMark.swift:100-107`). The hypothesis is that macOS 26
rasterizes a `MenuBarExtra` label into a template image, and a view whose
entire content is `Color.primary` strokes masks out to nothing.

This is plausible and unconfirmed. It is not the only candidate: a
`Shape.trim(from:to:)` evaluating to an empty path, the `.environment(\.colorScheme, …)`
override at `BrandMark.swift:74`, or the `.frame(width:height:)` interacting
with the status item's own sizing would all produce the same empty 18 x 24
frame. **Establish the cause before building the fix.**

### The bisect, which is step one

Build a development bundle via `macos/scripts/make-app-bundle.sh` for each
rung and record what the status item draws. One change at a time, no
combinations:

1. Label is `Image(systemName: "tray.full")` and nothing else. If this is
   also invisible, the failure is at the label level and the mark is
   incidental — stop and re-scope.
2. Label is a solid `Rectangle().fill(.primary).frame(width: 15, height: 15)`.
   Separates "strokes do not survive" from "`.primary` does not survive".
3. Label is `Rectangle().fill(.black)`. Separates `.primary` specifically
   from fills generally.
4. Label is `BrandMark(size: 15, variant: .light)` — the framed, two-colour
   variant, which has a background rectangle. If this draws and `.template`
   does not, the guard at `BrandMark.swift:66` is implicated directly.
5. Label is the current `MenuBarLabel`, as a control.

Record the outcome in the implementation report. Every fix below is
conditional on it.

### The parity answer, which points at the fix

Both other clients had this exact problem and both solved it the same way:
**rasterize the mark from its own geometry, at the size the platform asks
for.** Neither hands a live view to the shell.

Windows renders to a BGRA buffer in
`windows/src/TraceCommons.Interop/MarkRaster.cs`, and its remarks say why
plainly — the notification area takes an `HICON`, "so the mark has to become
pixels somewhere". It also argues explicitly against a baked size ladder: an
`.ico` full of PNGs "is a second description of the mark that has to be kept
in step with the XAML by hand. Rendering from the same numbers keeps one
description." Linux does the same thing in a different medium, generating a
scalable SVG through `mark::template_svg` and installing it into the icon
theme, deliberately "`scalable` and not a size ladder"
(`crates/trace-commons-contributor-gtk/src/tray.rs:241`, `:265-268`).

The Windows file even names the assumption that turned out to be false here:
"unlike macOS the Windows notification area does not recolour what it is
given". macOS's template mechanism was trusted to handle the mark, and the
mark was handed over as a live SwiftUI view rather than as pixels. macOS is
the only one of the three that does it that way, and it is the only one where
the mark does not appear.

If the bisect confirms the hypothesis, the fix is to follow the other two:
render `BrandMark`'s template geometry through `ImageRenderer` at the
required point size, wrap it in an `NSImage` with `isTemplate = true`, and
give `MenuBarExtra` that. This keeps the Windows principle intact — one
description of the geometry, rasterized on demand — rather than introducing a
baked asset that has to be kept in step by hand.

### What C needs from B, and what it does not

The brief for this slice assumed the menu-bar fix would consume sub-project
B's generated template artwork. On the evidence above, it should not, and the
dependency is narrower than stated:

- **The menu-bar mark does not need B.** Rendering from `BrandMark`'s
  existing geometry at runtime is both the fix and the thing that matches
  Windows and Linux. A pre-generated template PNG would be the size ladder
  `MarkRaster.cs` argues against. Part 1 can land before B.
- **The Dock icon does need B.** `CFBundleIconFile` wants a real `.icns` or
  `.icon`, and producing it from the shared vector source is exactly what B
  is for. Part 2 cannot finish before B.

Split the slice on that seam. Part 1 is shippable on its own and fixes the
defect that makes the app unfindable; Part 2 waits on B.

## Part 2 — the Dock icon

### The bundle changes

- Remove `<key>LSUIElement</key><true/>` from `macos/scripts/info-plist.sh:67`.
- Add `CFBundleIconFile` (and `CFBundleIconName` if B produces an `.icon`),
  and copy B's artwork into `Contents/Resources` in
  `macos/scripts/make-app-bundle.sh`, which already creates that directory at
  line 86 and currently puts nothing in it. The copy must land before the
  `codesign` calls at `make-app-bundle.sh:155-166`, since the signature seals
  bundle resources.
- Set the activation policy explicitly rather than relying on the absence of
  `LSUIElement`. There is currently no `setActivationPolicy` call, no
  `NSApplicationDelegate`, and no `@NSApplicationDelegateAdaptor` anywhere
  under `macos/Sources/TraceCommonsApp/`; all of that is new surface.

### A pinned test asserts the opposite of this

`crates/trace-commons-contributor/tests/release_pipeline.rs:50-56` fails the
build if `LSUIElement` goes missing, and its comment states the old intent
exactly: "Regressions here are silent and severe: without LSUIElement the
menu-bar app grows a Dock icon, and without the bundle id notifications
break."

Do not delete that assertion. Invert it, so the new intent is pinned as
firmly as the old one was: assert `LSUIElement` is **absent** and that
`CFBundleIconFile` is present. A deleted assertion leaves the plist
unguarded; a rewritten one keeps the property tested and leaves a diff that
says the decision changed on purpose. The neighbouring `CFBundleIdentifier`
assertion at `:56-59` stays untouched.

### What a Dock icon changes beyond appearance

An `LSUIElement` app has no App menu, no Cmd-Tab entry, and no Dock menu. A
`.regular` app gets all three for free, and each one is a new path into
behaviour that today only exists behind the menu-bar item.

**Quit must not lose its confirmation.** `MenuBarView.swift:97` routes
"Quit…" through `confirmQuit()` (`:195-211`), which raises an `NSAlert`
saying the watcher stops with the app, and only then calls
`NSApp.terminate(nil)` at `:209`. A `.regular` app gains a standard App menu
whose Quit item, its Cmd-Q shortcut, and the Dock icon's context menu all
call `NSApp.terminate(_:)` directly — bypassing that alert entirely. Route
them back through it, either with an `NSApplicationDelegate` implementing
`applicationShouldTerminate(_:)` and returning `.terminateLater` until the
alert answers, or by replacing the Quit item through a `CommandGroup`. The
delegate is the safer of the two because it catches every path, including
ones SwiftUI does not own. Whichever is chosen, the alert's copy stays as it
is: it was written specifically because the macOS app *is* the daemon, and
that reasoning is unchanged.

**Termination cleanup already depends on the notification.**
`TraceCommonsAppMain.swift:79-85` observes
`NSApplication.willTerminateNotification` to run `model.shutdown()`. That
notification still fires for every terminate path, so the cleanup survives —
but a `.terminateLater` design must be careful to complete the alert and let
termination proceed, or shutdown never runs.

**Reopen behaviour is new.** Clicking a `.regular` app's Dock icon when no
window is open sends `applicationShouldHandleReopen(_:hasVisibleWindows:)`.
With no delegate the app will appear to do nothing, which reads as a hang.
Wire it to the same path the menu already uses — `OpenMainWindow.request()`
(`TraceCommonsAppMain.swift:116-123`), which activates and opens
`WindowID.main`.

**Launch presentation is new.** Today the app launches silently because it
cannot do otherwise. As a `.regular` app it will activate on launch, and the
`Window` scene may open unbidden. Decide deliberately whether first launch
shows the window; the existing `TRACE_COMMONS_SHOW_WINDOW` hook
(`TraceCommonsAppMain.swift:100-104`) exists precisely because "a menu-bar app
otherwise shows nothing until asked", and that comment stops being true.

### Login item

`LoginItemManager` uses `SMAppService.mainApp`
(`macos/Sources/TraceCommonsApp/LoginItemManager.swift:49`, `:73`). The bundle
identifier does not change, so registration continues to work and the
Homebrew cask's `uninstall quit: "ai.tracecommons.shell"` stanza keeps
matching — the cask needs no edit.

What does change is what login looks like. `SMAppService.mainApp` launches
the bundle; for an `LSUIElement` app that is invisible, and for a `.regular`
app it means the app activates and takes a Dock slot at every login, possibly
opening a window. `SMAppService` offers no "launch hidden" flag for
`mainApp`, so if silent login is wanted it has to be implemented — the
conventional approach is to detect a login launch and call `NSApp.hide(nil)`
before any window opens.

Decide this explicitly. An app that grabs focus at every login after the user
opted into "Start Trace Commons when you log in?" — copy from the macOS shell
spec's "## Login item" — is a worse bargain than the one they agreed to. The
recommendation is to launch hidden to the menu bar and leave the Dock icon as
a way to *find* the app, not a thing that greets you.

## Part 3 — registering `tracecommons://`

### Why this is in C rather than filed elsewhere

An earlier draft filed this as out of scope, against
`2026-08-17-onboarding-parity-windows-linux.md`. It belongs here. That spec
is about porting onboarding *to* Windows and Linux; this is macOS's own
declaration missing, in `macos/scripts/info-plist.sh` — a file this slice is
already editing for `LSUIElement` and `CFBundleIconFile`. And it is a hard
blocker rather than a nicety: an invite link is one of the two ways a
contributor can enrol, and on macOS it currently does nothing at all.

### Parity: macOS is the outlier, again

Both other clients register the scheme, and both were written believing macOS
already had it covered.

Windows registers `Software\Classes\tracecommons` under HKCU for the
unpackaged build it actually ships
(`windows/src/TraceCommons.App/UrlSchemeRegistration.cs`), setting the
`URL Protocol` marker and an `open\command` of `"<exe>" "%1"`. Under MSIX it
returns early and the OS owns the registration through a `windows.protocol`
extension in `windows/packaging/Package.appxmanifest`, so exactly one of the
two paths is live. Linux registers `x-scheme-handler/tracecommons` in the
desktop entry and picks the URL out of argv
(`crates/trace-commons-contributor-gtk/src/main.rs:58-78`).

Both files then cite macOS as the platform that does this properly.
`UrlSchemeRegistration.cs` says "macOS does not have this problem: it
delivers URL events instead," and the GTK comment says "unlike the macOS
URL-event path, a scheme handler receives this as argv." Both are true about
the *mechanism* and both are wrong about this build, because the mechanism is
never switched on. This is the same shape as Part 1, where
`MarkRaster.cs` reasons from "unlike macOS the Windows notification area does
not recolour what it is given" while the macOS recolouring path draws
nothing. Twice now, a cross-platform comment has recorded macOS as the
reference implementation for something macOS does not actually do. Treat that
pattern as a review signal, not a coincidence.

### The contract the three must keep

One invite mail goes to contributors on all three platforms, so the link
shape is a contract, not a per-shell detail. All three parse it today and
agree:

- Scheme `tracecommons`, compared case-insensitively.
- Host `enroll`, compared case-insensitively.
- Query parameter named `invite`, matched case-**sensitively**, value
  percent-decoded.

macOS is `DeepLink.inviteURL`
(`macos/Sources/TraceCommonsApp/Views/OnboardingConnectView.swift:223-231`),
Windows is `DeepLink.InviteFrom`
(`windows/src/TraceCommons.Interop/DeepLink.cs:35-62`), Rust is
`invite_from_deep_link`
(`crates/trace-commons-contributor/src/commands.rs:1814-1826`), which the GTK
shell re-exports rather than re-implements
(`crates/trace-commons-contributor-gtk/src/ui/onboarding.rs:189`).

The canonical vector, pinned in
`crates/trace-commons-contributor/src/commands.rs:1834-1837`:

    tracecommons://enroll?invite=https%3A%2F%2Fissuer.example%2Fonboard%23CODE

decodes to `https://issuer.example/onboard#CODE`.

**One divergence to fix while here.** Rust drops an empty invite
(`commands.rs:1825`, `.filter(|v| !v.is_empty())`) and Windows returns null
for one (`DeepLink.cs:60-61`). macOS does neither: `inviteURL` returns the
query item's value as-is, so `tracecommons://enroll?invite=` yields `Some("")`
and drives the Connect screen with an empty field and a resolve attempt.
Match the other two.

**A deep link must not enrol.** This is a deliberate product rule, stated in
`crates/trace-commons-contributor-gtk/src/main.rs:61-65` — the invite is
filled in and the button is left for a person to press, "because which
commons to join is the decision that screen exists to ask" — and repeated at
`crates/trace-commons-contributor-gtk/src/ui/onboarding.rs:192-197`. macOS
already complies: `.onOpenURL` sets `inviteText` and calls `resolve()`, which
shows the issuer, and stops there. Preserve that when the delivery path
changes below.

**The invite never reaches a log.** It is a credential with a `max_uses` in
the thousands. Both other clients say so at the registration site; the same
rule applies to whatever new plumbing Part 3 adds.

### The declaration

Add to `macos/scripts/info-plist.sh`, alongside the existing keys:

    <key>CFBundleURLTypes</key>
    <array>
      <dict>
        <key>CFBundleURLName</key><string>ai.tracecommons.shell.invite</string>
        <key>CFBundleURLSchemes</key><array><string>tracecommons</string></array>
        <key>CFBundleTypeRole</key><string>Viewer</string>
      </dict>
    </array>

`CFBundleURLName` is derived from the bundle identifier already asserted at
`crates/trace-commons-contributor/tests/release_pipeline.rs:56-59`, so the two
stay legible together. `Viewer` and not `Editor`: the app displays what the
URL names and asks a person to act, which is exactly the distinction the role
draws, and it is the honest declaration given the no-auto-enrol rule above.

### Yes, it interacts with the activation-policy change

Not in the way "LSUIElement versus regular" suggests — a background app
receives URL events perfectly well. The interaction is that **`.onOpenURL`
only fires on a view that is currently mounted**, and the only place it is
attached is inside `OnboardingConnectView`
(`macos/Sources/TraceCommonsApp/Views/OnboardingConnectView.swift:78`; it
appears nowhere else under `macos/Sources/`).

The app's normal resting state is running with no window open. In that state
the Connect view does not exist, so a delivered URL lands on nothing and is
dropped silently. Declaring `CFBundleURLTypes` without fixing this converts a
dead link into an intermittent one, which is worse: it will work in a
developer's session, where onboarding happens to be on screen, and fail for
the contributor who clicked the link with the app already running.

Handle the URL above the view, then. `NSApplicationDelegate`'s
`application(_:open:)` — or an `onOpenURL` on the `App` scene — parses it,
stashes the invite, calls `OpenMainWindow.request()`
(`macos/Sources/TraceCommonsApp/TraceCommonsAppMain.swift:116-123`) to bring
the window up, and lets the Connect screen consume the pending value on
appear. Linux already has exactly this shape: a `PENDING_INVITE` thread-local
with `set_pending_invite`, held until onboarding is built
(`crates/trace-commons-contributor-gtk/src/ui/onboarding.rs:180-200`). Mirror
it rather than inventing a second pattern.

This is the same `NSApplicationDelegate` Part 2 already introduces for
`applicationShouldTerminate` and `applicationShouldHandleReopen`. Build it
once, for all three reasons, and the Dock-icon work and the deep-link work
land together rather than fighting over who owns the delegate.

### The dev-bundle hazard, measured

Do **not** give the development bundle a different scheme or a different
bundle identifier. A dev build that declares something other than what ships
stops testing the thing that ships, and the identifier in particular is load
bearing elsewhere — notifications and the Homebrew cask's
`uninstall quit: "ai.tracecommons.shell"` both key off it.

That leaves a real hazard, and it is not theoretical. LaunchServices on this
machine currently has **five** bundles registered under
`ai.tracecommons.shell`: the installed `/Applications/TraceCommons.app`, and
four development bundles built into worktrees under
`.claude/worktrees/*/macos/.build/` and `.worktrees/*/macos/.build/`. Two of
those four no longer exist on disk and are still registered. Earlier in the
session that produced this spec, a worktree dev build won the bundle-identifier
race and was the process actually running after an `open` that was meant to
launch the installed app.

The obvious diagnostic under-reports it. `mdfind
"kMDItemCFBundleIdentifier == 'ai.tracecommons.shell'"` returns exactly one
result, because `.build` directories are not Spotlight-indexed —
`mdls -name kMDItemCFBundleIdentifier` on the worktree bundle returns
`(null)` — while LaunchServices registers bundles independently of Spotlight.
Use `lsregister -dump` and grep for the bundle path when diagnosing, never
`mdfind`.

Once the scheme is declared, every one of those claimants claims
`tracecommons://` and LaunchServices picks the winner. So:

- Every deep-link verification must assert **which** bundle handled the URL,
  not merely that something did. A pass that was actually served by a stale
  worktree build is a false pass, and this is the specific way it will happen.
- `macos/scripts/run-demo.sh` and the dev loop need a documented hygiene step:
  unregister the dev bundle when finished
  (`lsregister -u <path>`), and know that `lsregister -kill -r -domain local
  -domain user` rebuilds the database when it has drifted.
- Rebuilding into a worktree re-registers it. This is not a one-time cleanup;
  it is a standing property of developing a URL-handling app in worktrees, and
  it should be written down where someone running the demo will see it.

### Pin the declaration in `release_pipeline.rs`

Yes — add a third assertion, next to the `LSUIElement` inversion and the
untouched `CFBundleIdentifier` check
(`crates/trace-commons-contributor/tests/release_pipeline.rs:50-59`). A dead
deep link is precisely the silent, severe regression that test exists to
catch: nothing crashes, nothing logs, the app simply stops answering invite
mail, and no one notices until a contributor reports that a link did nothing.
That is how the current gap survived to 0.3.0.

Assert two things, not one:

- `CFBundleURLTypes` is present.
- The literal scheme string `tracecommons` is present.

Presence alone is not enough. A declaration carrying the wrong scheme — a
rename, a typo, a copy from another product — passes a presence-only check
while leaving the link just as dead.

State the limit in the test's comment: it proves the plist *declares* the
scheme, and cannot prove LaunchServices *routes* it. Routing depends on the
registration database and on which bundle wins, neither of which is visible
from a plist. That half is the manual gate below, and the comment should say
so, so nobody reads a green test as proof the feature works.

### Manual verification, since there is no macOS CI

macOS has no CI job in this repository, so this goes in the manual release
gate beside signing and notarization. Run it against the **installed** bundle,
not a `.build` one.

Before anything else, confirm the field: `lsregister -dump | grep TraceCommons.app`
should list only `/Applications/TraceCommons.app`. If a worktree bundle is
listed, unregister it first or the run proves nothing.

Then, with the app **running and no window open** — the state that breaks a
view-level handler, and therefore the case that matters:

    open 'tracecommons://enroll?invite=https%3A%2F%2Fissuer.example%2Fonboard%23CODE'

A correct result is all of:

1. `/Applications/TraceCommons.app` is the process that handled it. Check
   with `pgrep -lf TraceCommonsApp` and confirm the path; a worktree path is a
   failure even if the UI looked right.
2. The app comes forward and the main window opens on the Connect screen.
3. The invite field contains the decoded `https://issuer.example/onboard#CODE`
   — decoded, not the percent-encoded form.
4. The issuer host is displayed, from `resolve()`.
5. **No enrolment has occurred.** The button is un-pressed and
   `status.logged_in` is unchanged. This is the product rule from
   `main.rs:61-65`, and it is the one a delivery-path change is most likely to
   break.

Repeat with the app **not running** at all, which exercises launch-time
delivery rather than delivery to a live process. Both paths must reach the
same screen.

Two negative checks:

- `tracecommons://enroll?invite=` opens nothing and enrols nothing, matching
  Rust and Windows once the empty-value divergence above is fixed.
- The invite string appears in no log, no window title, and no crash report.
  Grep the new delegate code for the invite reaching anything other than the
  field itself — the same check
  `2026-08-17-onboarding-parity-windows-linux.md` requires of the other two
  shells.

## Screenshots and the capture harness

**Every checked-in menu-bar image is already stale, and that is how this
shipped.** `docs/images/macos-shell-menu-bar.png` and
`docs/superpowers/plans/macos-design-pass/after-dark/macos-shell-menu-bar.png`
both date from commit 33f25a34 (2026-08-10, "Add the macOS contributor app").
`BrandMark.swift` did not exist until 8a285e6c (2026-08-17, "Adopt The Turn
and the desktop design spec across all three clients"). The committed images
still show the pre-Turn SF Symbol tray glyph. No image in this repository has
ever shown the mark in a menu bar, and the design pass that put it there never
re-captured one.

The harness would not have caught it either. `DebugScreenshot` rasterizes with
`ImageRenderer` rather than photographing windows, for a stated and good
reason — `screencapture` and `cacheDisplay` return blank on a locked session
(`macos/Sources/TraceCommonsApp/DebugScreenshot.swift:8-13`). But that means
`MenuBarPreview` (`:168-186`) renders `MenuBarLabel` into an ordinary SwiftUI
context, not through `MenuBarExtra`'s template-image path. It exercises the
view and not the surface, so it renders the mark happily while the real status
item stays blank. This is the fixture agreeing with the code and both being
wrong about the product.

So:

- Re-capture `docs/images/macos-shell-*.png` after the fix, and add a Dock and
  App-menu view to whatever now shows the app's shape.
- Add one verification that goes through the real surface rather than
  `ImageRenderer` — a `screencapture` of the status item's own accessibility
  frame on an unlocked machine, asserting the crop is not uniform. Keep it out
  of CI, where there is no session to composite; it belongs in the manual
  release gate next to the signing and notarization checks. `ImageRenderer`
  stays for everything else.
- `macos/scripts/run-demo.sh:109-118` passes `TRACE_COMMONS_SHOW_WINDOW=1` and
  launches with `open -n`. Neither breaks, but its closing line — "quit from
  the menu bar when done" (`run-demo.sh:120`) — stops being the only way, and
  `open -n` on a `.regular` app puts a second Dock icon on screen for every
  run. Revisit both.

## Adjacent findings, deliberately out of scope

One thing surfaced during verification that belongs to another slice.
Recorded here so it is not lost, and not fixed here.

- **The menu bar is silent about a refused start.** `AppModel.health` derives
  from `status.health.lastErrorLabel` (`AppModel.swift:90-91`), which comes
  from a running daemon. When `DaemonHost` refuses to start there is no
  daemon, so `health` is `nil` and the item reports "Nothing waiting" — while
  the window says the watcher is not running. Sub-project A owns the refusal;
  it should also decide how the status item shows it.

The `tracecommons://` registration was filed here in an earlier draft and has
been pulled into scope as Part 3, at Zaki's direction.

## Work

Part 1, independent of B:

1. Run the five-rung bisect and record the result.
2. Fix the label per the outcome; if the hypothesis holds, rasterize
   `BrandMark`'s template geometry via `ImageRenderer` into a template
   `NSImage`.
3. Confirm the badge, unhealthy, and paused states all still draw, since the
   current evidence only covers the idle state.

Part 3, also independent of B, and sharing Part 2's delegate:

4. Add `CFBundleURLTypes` to `info-plist.sh`.
5. Move deep-link handling above the view: parse in the delegate, stash a
   pending invite the way `set_pending_invite` does on Linux, open the window,
   let Connect consume it on appear.
6. Drop the empty-invite divergence so macOS matches Rust and Windows.
7. Add the third `release_pipeline.rs` assertion, pinning both
   `CFBundleURLTypes` and the `tracecommons` scheme string, with a comment
   naming what it cannot prove.
8. Write the LaunchServices hygiene step into the dev loop.

Part 2, after B:

9. Drop `LSUIElement`, set the activation policy, add `CFBundleIconFile`, copy
   B's artwork in `make-app-bundle.sh` before signing.
10. Invert the `release_pipeline.rs` `LSUIElement` assertion.
11. Add the delegate: `applicationShouldTerminate` routing to `confirmQuit`'s
    alert, and `applicationShouldHandleReopen` routing to
    `OpenMainWindow.request()`. This is the same delegate Part 3 needs, so
    whichever part lands first builds it.
12. Decide and implement launch-at-login presentation.
13. Re-capture the docs images; add the real-surface capture check to the
    release gate.

## Verification

- The bisect result is written down, and the fix cites it.
- On a real unlocked macOS 26 desktop, the status item's accessibility frame
  is captured and is not uniform, in light and dark appearance, with the
  system accent both default and changed.
- All four label states draw: idle, badge, unhealthy, paused.
- The app appears in the Dock and in Cmd-Tab, with artwork rather than the
  generic placeholder.
- Cmd-Q, the App menu's Quit, and the Dock menu's Quit each raise the
  confirmation alert. Cancelling each one leaves the daemon running; a
  terminal `trace-commons-contributor daemon status` still answers.
- Confirming quit runs `model.shutdown()` and releases the lock, so a
  subsequent `daemon run` succeeds. This is the existing macOS acceptance
  criterion and it must survive the new termination path.
- Clicking the Dock icon with no window open opens the window.
- Registering as a login item, logging out and back in produces the agreed
  presentation, and the app still appears in System Settings > General >
  Login Items.
- `brew uninstall --cask trace-commons` still quits the running app, proving
  the cask's bundle-identifier assumption survived.
- `cargo test -p trace-commons-contributor --test release_pipeline` passes
  with the inverted `LSUIElement` assertion and the new `CFBundleURLTypes`
  and scheme-string assertions.
- The deep-link gate in "### Manual verification, since there is no macOS CI"
  passes in full, including the which-bundle-handled-it check, both the
  app-running and app-not-running paths, and the no-enrolment rule.

## Implementation notes, 2026-08-19

What was built, what was proven, and the one thing that blocked the rest.

### Two things in this spec were wrong

**The login-launch detection does not work.** "### Login item" recommends
detecting a login launch and hiding, and the obvious test -- a login item is
started by `launchd`, so `getppid() == 1` -- was implemented and then removed,
because it is not a test of anything. *Every* GUI launch is reparented to
launchd. Measured: launching the built bundle with `open` gives ppid 1, exactly
as a login start would. The first build using that test hid the app on every
launch (`visible: false` from System Events after a plain `open`).

`SMAppService.mainApp` exposes no launch-hidden flag and no "you were started
at login" signal, so the implementation does not guess. Launch behaviour is
uniform: come up quietly, which is what this app has always done. That is
correct at login -- the contributor agreed to a promise to be running, not a
request to be greeted -- and recoverable everywhere else, because there is now
a Dock icon and clicking it opens the window. Anyone reviving the hidden-launch
idea needs a signal that actually distinguishes the two cases; ppid is not it.

**`applicationShouldTerminate` does not need `.terminateLater`.** `NSAlert`'s
`runModal` is synchronous, so the delegate answers `.terminateNow` or
`.terminateCancel` directly. That also removes the hazard the spec flagged,
where a `.terminateLater` that never gets its reply leaves `model.shutdown()`
un-run.

### A pre-existing defect blocks the rest of the verification

**The main window does not open while the daemon is running.** Every route to
it fails -- the invite deep link, a Dock-icon click, and the
`TRACE_COMMONS_SHOW_WINDOW=1` hook. The window opens normally when startup is
`.refused`.

This is not caused by this slice. The shipped `/Applications/TraceCommons.app`
0.3.0 build, which predates all of it, fails identically: pointed at a
throwaway state directory with a valid `daemon-settings.json`, it creates
`daemon.lock` (so the daemon started, and `AppModel.startup` is `.running`) and
`TRACE_COMMONS_SHOW_WINDOW=1` still yields zero windows. No crash report is
produced and nothing is logged; the window is simply never created.

`MainWindowView` renders `CenteredNotice` for `.refused` and the onboarding
coordinator for `.running`, which is where suspicion falls, but the cause was
not chased down -- it belongs to whoever owns that screen, not to this slice.

It has an ugly consequence worth stating plainly: on this machine a
contributor whose daemon starts correctly cannot open the app window at all.
That is the state a newly-configured contributor is in.

### What was actually verified

Against a bundle built by `macos/scripts/make-app-bundle.sh` and launched by
explicit path:

- No `LSUIElement`, `CFBundleIconFile` is `AppIcon`, and
  `CFBundleURLTypes[0].CFBundleURLSchemes[0]` is `tracecommons`, all read back
  from the built `Contents/Info.plist` with `plutil`.
- The app is a regular app: it has an `AXMenuBar`, and a Dock tile named
  "TraceCommons".
- **The Dock icon carries the mark.** `NSWorkspace.icon(forFile:)` on the
  built bundle resolves to The Turn -- green bracket upper left, blue lower
  right -- not the generic placeholder. Rendered at 1024 and looked at, 1516
  unique colours.
- LaunchServices accepts the declaration: `lsregister -dump` shows
  `claimed schemes: tracecommons:`, claim id `ai.tracecommons.shell.invite`,
  `roles: Viewer`.
- **Deep-link delivery works in the state that used to drop it.** With the app
  running and no window open, `open 'tracecommons://enroll?invite=...'` was
  handled by the running process -- same pid, no second instance -- and the
  window went from 0 to 1 with the app frontmost. This is the case a
  view-level `onOpenURL` cannot serve.

### What could not be verified, and why

All of it downstream of the window defect above:

- That the invite field shows the *decoded* value, that the issuer host is
  displayed, and that no enrolment occurs. The Connect screen is only
  reachable under `.running`, which is the state whose window will not open.
  The delivery half is proven; the consumption half is not.
- The app-not-running deep-link path, for the same reason.
- The empty-invite negative check.
- Every quit path (Cmd-Q, App menu, Dock menu) reaching one confirmation, and
  the login-item presentation, which needs a real logout.

These stay open. The deep-link manual gate above is written as the release
check and should be run once the window defect is fixed -- most likely
alongside sub-project A, which is what makes `.running` reachable for a fresh
contributor in the first place.
