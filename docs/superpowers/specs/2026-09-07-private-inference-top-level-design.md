# Private inference as a top-level destination — design

## Why

Private inference is a decision a contributor makes about their own machine, and
today it is a switch buried in **Settings > Tools**. It was found by accident on
a fresh 0.9.0 upgrade: IronWire was running and the discovery pointer existed,
but the app showed nothing until the toggle was hunted down and `Connect`
pressed.

Two things follow from that. A person who has not gone looking does not know the
feature exists, and a person who has cannot turn it on or off quickly — which is
the thing they most want to do, because the exposure it creates is real and
situational:

> While it is on, anything else running on this computer can send calls through
> it as well, charged to the accounts you have set up here. On a computer only
> you use that is your own software; on a shared one it is anyone who can log
> in.

That is `OFFER_EXPOSURE` (`crates/trace-commons-contributor/src/private_inference_copy.rs:61`).
A sentence like that describes a decision, not a preference, and decisions do not
belong three levels down a settings tree.

## What exists already

This design mostly promotes existing machinery. It does not invent a subsystem.

- **A fourth top-level tab already takes its copy from Rust.**
  `MainWindowView.Section` (`macos/Sources/TraceCommonsApp/Views/MainWindowView.swift:10`)
  has `queue`, `history`, `settings` and `compute`, and `compute` alone draws its
  label and subtitle from `compute.copy?.destination` / `compute.copy?.subtitle`
  rather than a hardcoded Swift string (lines 150, 262). That is the pattern this
  design follows.
- **A shared cross-shell surface exists.**
  `macos/Sources/TCShellCore/PrivateInferenceSurface.swift` already provides
  `stateLine`, `tone`, `servingLine`, `shouldOffer`, `offerParams(accepted:)` and
  `settingsParams(on:)`. The controls are built; they are wired only into
  `SettingsView`.
- **The copy is already centralised in Rust.**
  `private_inference_copy.rs` is 687 lines and owns the wording. No shell writes
  its own.
- **All three shells already have a tray or menu-bar surface.** macOS
  `MenuBarExtra` (`macos/Sources/TraceCommonsApp/TraceCommonsAppMain.swift:35`,
  `Views/MenuBarView.swift`), Windows `windows/src/TraceCommons.App/TrayIcon.cs`,
  GTK `crates/trace-commons-contributor-gtk/src/tray.rs`.

What is missing: private inference appears in **none** of the three tray
surfaces, and there are no application shortcuts at all — every
`keyboardShortcut` in the macOS tree is `.defaultAction`, i.e. Enter on a dialog.

## The load-bearing rule

**Only `Clear` may be painted as working.**

`PrivateInferenceTone` is `Neutral | Held | Clear | Attention | Refused`
(`private_inference_copy.rs:193`), and `Clear` is documented there as "On,
answering, and with somewhere to pass calls on to. The only value that may be
painted as working."

The Swift mirror deliberately does **not** reuse the routing mapper, and says why
(`PrivateInferenceSurface.swift:73`): `RoutingTone.fromABI` "would answer
`.neutral` for every value here, turning a refusal into 'nothing to say'." That
separation must survive this change.

A tab and a tray icon both invite a green dot, and this is exactly where a
fail-open would be introduced by accident. Therefore:

- Every indicator — tab badge, tray glyph, menu-bar state line — derives from the
  **tone**, never from the settings boolean.
- The switch reports what the user asked for. The indicator reports what is
  true. When they disagree, the indicator wins and the copy explains.
- `Refused` and `Held` must be visually distinct from `Clear`. They are not "on".
- Unknown ABI values degrade to `Neutral`, never to `Clear`.

## Scope

**Cut 1 (this spec):** the tab, the tray/menu-bar section, and in-app keyboard
shortcuts, on macOS, Windows and GTK together.

**Amended twice. The shortcuts now exist on all three shells.**

The first amendment recorded that they had landed on macOS and Windows only:
GTK had no keyboard accelerators anywhere in the crate -- no
`set_accels_for_action`, no `ShortcutController` -- and the only `accel`
mention was a comment in `preview.rs` saying there deliberately is not one.

That gap has since been closed. GTK has `Ctrl-1..N` per screen, derived from
the `SCREENS` array by position so a screen added later gets its accelerator
without anyone remembering to, and `Ctrl-Shift-M` for the switch -- the same
chord Windows uses. The toggle asks `tray::toggle_request`, the same table the
tray asks, so the asymmetry is stated once: turning it off acts, turning it on
opens the destination and writes nothing.

The `preview.rs` comment still stands and was not touched. Its reason -- "the
one irreversible action in the product is reachable by pointing at it and
nothing else" -- is scoped to the Contribute button, and it argues FOR the
toggle's asymmetry rather than against accelerators generally.

Both amendments are kept rather than the first being deleted, because the
original scope cut had to be inferred from a coverage line and nothing said so.
A spec that quietly narrows and then quietly widens teaches a reader nothing.

**Cut 2 (deferred, not specified here):** a global system-wide hotkey. It is the
only piece requiring per-platform permission work — macOS Accessibility / Input
Monitoring with a user-facing prompt, `RegisterHotKey` on Windows, and
compositor- and portal-dependent behaviour on GTK. Deferring it keeps cut 1
identical across the three shells.

All three shells ship together. The Rust copy module and the shared surface
already force parity, and shipping one shell first is how the three drift.

## Design

### 1. Navigation

Private inference becomes a fifth top-level destination beside Waiting, History,
Computer and Settings.

- **macOS** — a `privateInference` case in `MainWindowView.Section`. Label and
  subtitle come from Rust copy, following `compute`'s precedent, not from a
  Swift literal. It needs its own glyph; `compute` currently reuses `.monitor`,
  which is already a collision with `queue` and should not be extended to a
  third destination.
- **Windows** — a nav item in `windows/src/TraceCommons.App/MainWindow.xaml`,
  with its label sourced from Rust through the interop layer, consistent with
  the existing rule that Windows copy comes from Rust.
- **GTK** — a new `crates/trace-commons-contributor-gtk/src/ui/private_inference.rs`
  beside `queue.rs`, `history.rs` and `settings.rs`. GTK bypasses the C ABI and
  reaches the Rust crate directly, so it consumes `private_inference_copy`
  natively rather than through the ABI.

**The Settings entry stays**, rewritten as a pointer to the new destination.
Removing it would break muscle memory and any existing instructions, and a
pointer costs nothing.

### 2. What the destination contains

Composed from what `PrivateInferenceCalls` already computes:

- the state line and serving line;
- the on/off control, calling the existing `settingsParams(on:)`;
- the tone-derived indicator, subject to the rule above;
- the exposure sentence, shown in full — this is the surface where the decision
  is actually made, so the consequence is stated rather than linked.

No new copy strings are authored in any shell. Anything needed that
`private_inference_copy.rs` does not yet provide is added there first, with the
three shells reading it.

### 3. Tray and menu bar

A private-inference section in each of the three existing surfaces: current
state as text, and a direct toggle.

This is the part that most directly answers "easy access to turn it on and off",
because it works without focusing the app — which is the real situation, mid
session in an editor. The macOS `MenuBarView` already has `waitingSection` and
`healthSection` to pattern-match against.

### 4. Keyboard shortcuts (cut 1)

- A toggle shortcut for private inference.
- `Cmd/Ctrl-1..5` to reach the five destinations.

Both are in-app, so they need no permissions and behave identically on the three
shells. Nothing here exists today, so all of it is additive.

## Testing

- **The tone rule is the thing to test hardest.** A test per shell asserting
  that `Held`, `Attention`, `Refused` and any unknown ABI value do **not** render
  as working, and that only `Clear` does. This is the regression that would
  otherwise ship silently.
- The switch and the indicator disagreeing renders the indicator's truth, not
  the boolean.
- Copy parity: no shell contains a private-inference user-facing string literal;
  all come from `private_inference_copy.rs`.
- The existing offer behaviour is unchanged — `shouldOffer(answered, on) ==
  !answered && !on` still holds, and the first-run offer still appears exactly
  once. This spec must not disturb it.
- macOS Swift tests run in CI (`macOS app tests`, `swift test` on `macos-26`),
  so shell-level assertions are enforced there.

## Risks

- **A green dot for a non-`Clear` state.** The central risk, mitigated by the
  rule above and by tests that assert it per shell.
- **Three shells drifting.** Mitigated by shipping together and by keeping every
  string in Rust.
- **Glyph collision on macOS.** `queue` and `compute` already share `.monitor`;
  a third reuse would make the sidebar ambiguous. A distinct glyph is required.
- **Scope creep into cut 2.** Global hotkeys are explicitly out. A per-platform
  permission prompt is a different piece of work with a different review.
