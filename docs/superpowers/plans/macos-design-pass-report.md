# macOS shell: design and aesthetic pass

Screenshots referenced below are committed beside this file:

- `macos-design-pass/before/` — the shell as it was.
- `macos-design-pass/after-light/` and `macos-design-pass/after-dark/` — after,
  in both appearances.

Regenerate them with:

```bash
TRACE_COMMONS_APPEARANCE=dark \
TRACE_COMMONS_SCREENSHOT_DIR=<dir> \
TRACE_COMMONS_SELFTEST_OUT=<file> \
TRACE_COMMONS_DEMO_PREVIEW=1 \
TRACE_COMMONS_QUIT_AFTER_SHOT=1 \
  bash macos/scripts/run-demo.sh
```

`TRACE_COMMONS_APPEARANCE` (`light` / `dark` / unset) is new in this pass. See
"Verification" for why it had to touch a view modifier and not only `NSApp`.

---

## The reported bug: it was not a missing SF Symbol

The brief identified the yellow box with a red prohibition sign in
`macos-shell-menu-bar.png` as a broken image from an SF Symbol that does not
exist on macOS 14. It is not. Every symbol in the app resolves, and the yellow
placeholder is `ImageRenderer` refusing to rasterize an AppKit-backed control.

Two checks, both re-runnable:

**1. Every symbol name in the app resolves.** `NSImage(systemSymbolName:)`
returns non-nil for all of them — `tray.full`, `tray`,
`tray.and.arrow.down.fill`, `exclamationmark.triangle`, `chevron.left`,
`largecircle.fill.circle`, `circle`, `checkmark.square`, `square`,
`checkmark.circle`, `clock.arrow.circlepath`, `gearshape`, `clock`,
`circle.dotted` — and for every symbol added in this pass.

**2. The placeholder tracks control type, not symbol name.** A probe rendering
a mixed view through `ImageRenderer` at scale 2:

| View in the probe | Rendered |
| --- | --- |
| `Text` | correctly |
| `Image(systemName: "tray.full")` | correctly |
| `Button` | correctly |
| `Menu` | yellow prohibition placeholder |
| `TextField` | yellow prohibition placeholder |
| `Picker(.segmented)` | yellow prohibition placeholder |
| `Toggle` | yellow prohibition placeholder |

Those four are NSView-backed on macOS and `ImageRenderer` cannot rasterize
them. The placeholder positions in the old screenshots correspond exactly:

- `macos-shell-menu-bar.png` — the `Menu("Pause")` submenu.
- `macos-shell-onboarding-connect.png` — the invite `TextField`.
- `macos-shell-preview-sheet.png` — the segmented `Picker` (wide bar) and the
  search `TextField` (second bar).

So there was no broken image in the running app. There was a **verification
hole**: three controls were invisible to the only tool being used to check the
UI, and one of them was the tab bar of the most important screen in the
product. `OnboardingConnectView` already carried a comment guessing this was
the text caret; that guess was wrong in mechanism and right in conclusion.

What was done about it:

- The preview sheet's segmented `Picker` is now a SwiftUI segmented control
  built from `Button`s. It renders in captures, and it earns the change
  independently: each tab can now carry a glyph and a count, so "What's in it 4"
  tells you there is something to look at before you click.
- `TextField` and `Menu` stay as they are. They are the correct controls and
  there is no SwiftUI-native substitute that behaves properly. Their
  placeholders in the captures are expected and are now documented rather than
  mistaken for a defect.

---

## The design system

One new file, `macos/Sources/TraceCommonsApp/Views/DesignSystem.swift`, holds
everything: a 4pt spacing scale, two reading measures, the type scale, the
palette, the colour roles, the card treatment, the brand mark, and the small
shared parts (`TCFieldLabel`, `TCTag`, `TCSectionHeader`). No view writes a raw
padding number, a font size, or a colour literal any more.

**Direction: a customs declaration, not a feed.** Every waiting session is one
card, every card carries the same fields in the same order, and each ends in a
fixed manifest strip set in monospaced type — payload size and what scrubbing
removed, under heavy uppercase field labels, recessed behind a rule. Figures
that describe what leaves the machine are always monospaced; prose never is.
That is the one bold move, and everything else is kept quiet so it can carry:
reading the third card should not require reading it, only checking whether the
figures in the two familiar slots look like the ones above.

Concretely, on the queue:

- Card hierarchy is now project name → prompt → manifest → decision, instead of
  four things at the same weight.
- The two actions are adjacent at the trailing edge of the manifest band,
  default action last, instead of thrown to opposite ends of a very wide row.
- A session where scrubbing matched **nothing** breaks the rhythm on purpose:
  gold card border, a `nothing matched` pill, and a different sentence. That is
  the row most worth a second look and the old constant caveat flattened it away.
- The window has real structure: sidebar, `navigationTitle` +
  `navigationSubtitle`, and a toolbar carrying a permanent Watching / Paused
  readout plus the Pause menu. Paused is a state people forget they chose.

---

## The repeated caveat

"Scrubbing is pattern-based. It misses things it hasn't seen before." was
printed character-for-character on every card. It is the sentence that makes
every other claim in the app credible, and stamping it identically on every row
is exactly how a reader learns to skip it — identical text in an identical slot
reads as furniture within about two rows.

It is not deleted and not softened. It now lands in three places, each doing
one job (`Views/ScrubbingCaveat.swift`):

1. **On the card, as a fact about that session rather than a constant.** With
   redactions: "Removed by pattern matching. Anything the patterns don't know is
   still in there." With none: "Nothing matched a pattern. That is not the same
   as nothing being there." Because the line differs per row, it has to be read
   to be understood, and the more alarming case is the one that now reads more
   alarming.
2. **Under the list, once per screen, verbatim.** It is a statement about the
   mechanism, and the mechanism belongs to the list, not to any one session.
3. **Directly above `Contribute`, verbatim, in gold with a glyph.** This is the
   only irreversible click in the product. A caveat someone scrolled past ten
   times is worth less than one sitting under their cursor at the moment it
   matters, so repeating it there buys something.

---

## Alignment with the community site

Read from `community/public/styles.css` and `index.html`.

Carried across:

| Site | App |
| --- | --- |
| `--bg: #f6f7f4` warm off-white ground | `TC.ground`, on the content area |
| `--surface` / `--surface-2` / `--line` | `TC.surface` / `TC.surfaceInset` / `TC.line` |
| `--green: #178f70` primary | app accent (`.tint`), and the `clear` tone |
| `--blue` / `--gold` / `--coral` | `held` / `attention` / `refused` tones — the site's own meter-fill role mapping |
| `.eyebrow`, `th`, `.kpi .label`: 12px, weight 800, uppercase | `TCFieldLabel` |
| `.pill`: 999px radius, hairline border, heavy small type | `TCTag` |
| radii 6 / 8 | `TC.Radius.inset` / `.card` |
| hairline `--line` rules banding sections | `TCSectionHeader`, card borders |
| `.hero-band` two-column grid | the welcome screen's hero |
| `.kpi` band: uppercase label over a large figure | "This week" band, History tallies |
| `.brand-mark` | `BrandMark`, transcribed as geometry |

**The brand mark** is reproduced exactly, as paths rather than an asset. The
site builds it from two CSS gradients; the stops translate to `x + y <= 0.76`
for the green wedge and `y <= x + 0.1` for the blue field, on a bordered square
of `--surface`. It appears at 148pt on the welcome screen and at 15pt in the
menu bar.

**The menu-bar icon is now the mark, not `tray.full`.** A menu bar holds twenty
icons drawn from the same SF Symbol set and a generic tray is not findable among
them; a distinctive geometric mark is, and it is the same mark people will meet
on the web. State precedence is unchanged (decisions owed → unhealthy → paused →
idle) and the numeric badge still counts decisions owed. The monochrome build is
a reduction, not a desaturation: filled flat in one tint the two wedges merge
into a heavy blob, so it keeps the green wedge solid, drops the blue field to a
wash, opens a hairline seam, and draws the border. Everything is `.primary`, so
the system tints it for a light or dark menu bar and inverts it when the menu is
open.

### Deliberate departures from the site

- **Inter is not bundled.** A font file in a notarized bundle is a real cost for
  a brand cue. The site's 680/760/800 weights are reproduced with SF's
  `.semibold` / `.bold` / `.heavy`, which is what those weights are for.
- **No drop shadows.** `0 18px 48px rgba(23,31,33,0.08)` is a web idiom; inside
  a macOS window it reads as a floating dialog. Hairlines separate natively.
- **The brand stops at the chrome.** Toolbar, sidebar, sheet chrome, focus
  rings and the menu-bar popover stay system materials and vibrancy. The palette
  is applied to the content area only. The menu-bar menu gets leading glyphs and
  nothing else — an AppKit menu draws its own vibrancy and highlight, and
  anything painted over that reads as a bug.
- **Overriding the user's accent colour is itself a departure**, made on
  purpose: the app and the site are one product, the accent is the strongest cue
  that they are, and green carries a meaning here (good standing) that the
  system blue does not. Control shape, focus behaviour and keyboard handling
  stay stock.
- **Accent colours are darkened for type.** The site tunes its accents for
  fills, meter bars and borders, where 3:1 is the bar. As small text on a light
  surface several fail 4.5:1 — `--gold` on `--surface-2` measures about 2.9:1,
  which is not a contrast a warning sentence may be set in. Each accent has a
  text-only twin (`TC.goldText`, `greenText`, `coralText`, `blueText`) with the
  hue preserved and only the lightness moved. Fills, glyphs and borders keep the
  site's exact values.

### Dark Mode: derived, not inverted

The site declares `color-scheme: light` and has no `prefers-color-scheme` block,
so there was nothing to copy. The dark palette preserves the site's *relations*:

- The site's ground is not neutral grey — it is warm with a faint green cast.
  The dark ground keeps that cast at the other end of the scale (a warm
  near-black in the `#15170F` family) rather than the blue-black a naive
  inversion produces.
- Ground / surface / inset keep the same order and roughly the same perceptual
  spacing as `--bg` / `--surface` / `--surface-2`.
- Every accent keeps its hue and its role and is lifted in lightness until it
  clears contrast against the dark ground. `#178f70` is a good colour on white
  and an illegible one on near black; the dark counterpart is the same green,
  raised.

---

## Density

The first pass left too much air: a 720pt column in a wide window, a manifest
band and an action band stacked into two rows, and a lot of empty page below a
short list. The site fills its 1180px measure by *banding* content across the
full width rather than by setting long lines, and the app now does the same:

- The list measure went 720 → 980. Prose measure stays narrow (660) — running
  text is still read, not scanned.
- The manifest figures and the two buttons share one band. Labelled figures at
  the leading edge, the decision at the trailing edge, one line.
- A "This week" KPI band closes the queue: three labelled figures across the
  measure, in the same words the menu bar and History use. It sits at the foot,
  not the head — the screen's job is decisions, and a summary above the list
  would push the decisions down to make room for something nobody opened the
  window to read.

---

## The landing screen

The first frame is the only screen with a hero and the only one with motion,
because it has a job the others do not: a developer who just installed something
that reads their transcripts decides in about four seconds whether this is
serious software, and four same-sized paragraphs do not answer that.

It is banded like `.hero-band`: eyebrow, a display-weight headline, a lede, a
rule, two columns of supporting copy, then large-control actions. The headline
runs at 42pt heavy via `@ScaledMetric(relativeTo: .largeTitle)`, so accessibility
text sizes still move it.

**One sentence moved, none rewritten.** "You decide what gets contributed.
Nothing is sent unless you say so." was set bold inside the second paragraph,
where it was the most important claim on the screen and the least likely to be
read. It is now the headline — which is what bolding it inside a paragraph was
trying and failing to do. The paragraph it came from still reads as a complete
sentence without it. The scrubbing concession ("That scrubbing is good and it is
not perfect — which is why you get to look first") stays verbatim, stays on this
screen, and is not demoted into small print.

**Motion.** There is no animation on the community site to port — not one
`transition`, `@keyframes`, or `requestAnimationFrame` in `styles.css` or
`app.js`. So one was designed, built from the only graphic the brand owns: the
mark assembles itself once, each wedge drawing in along its own diagonal, over
0.85s. No loop, no bounce, no ambient drift, and no second animated thing
anywhere else in the product — an app asking to be trusted with someone's source
code should not fidget. `accessibilityReduceMotion` renders the finished mark
immediately.

---

## Accessibility

- Colour is never the only signal. `TC.Tone` pairs a colour with a mandatory
  glyph, and call sites pair both with words. Held-for-privacy-review reads as
  held from the clock and the phrase before any colour is involved; the consent
  checkboxes change shape (`square` → `checkmark.square.fill`) as well as
  colour; "0 matches" and "1 match" carry different glyphs.
- Icon-only and figure-only controls carry VoiceOver labels: the menu-bar item
  states the count or the condition in a sentence, the toolbar status tag, every
  KPI tile, and every manifest cell.
- Text accents are contrast-corrected as described above.
- Every face derives from a system text style; the one absolute size on the
  welcome headline is `@ScaledMetric`. `TCSectionHeader` drops its rule to a
  second line rather than let a heading wrap under it, which is what keeps
  headers intact at accessibility text sizes.
- Dismissive actions ("Not this one", "Not now", "What gets removed?") are
  untinted. A bordered button inherits the accent, and a green "Not this one"
  beside a green "Look inside" reads as a second way to approve.

---

## Verification

- `cd macos && swift build` — clean.
- Screenshots regenerated and inspected in both appearances, repeatedly, through
  five iterations. Two defects were caught only by looking: the manifest strip
  drawn in `underPageBackgroundColor`, which is a heavy mid-grey in Light and
  swallowed the gold caution text; and the accent and ground not reaching the
  rasterized views at all, so an early "after" shot was still system blue on
  white.
- The self-test passes and still reports `opening prompt: chars=97
  nonempty=true` with no prompt text. Grepping both runs for fixture content
  (`Northwind settlement`, `zsh prompt`, `AKIA`, `ghp_`) returns nothing.
- `run-demo.sh` still always rebuilds. The conditional rebuild was not
  reintroduced; the only change to that script is passing
  `TRACE_COMMONS_APPEARANCE` through.

### Why the appearance hook needed a view modifier

`NSApp.appearance` pins real windows but not an offscreen `ImageRenderer`, which
resolves colours from the SwiftUI environment rather than from the application
object. A capture run asked for Dark and silently produced Light — visually
identical to the Light run, which is exactly the class of false-confidence
failure the always-rebuild fix exists to prevent. `tcScreen()` therefore pins
`\.colorScheme` as well when `TRACE_COMMONS_APPEARANCE` is set. Unset (the
normal case) it is `nil` and every screen follows the system.

`tcScreen()` also applies the tint and ground per screen even though the `Window`
scene sets the tint too. The duplication is deliberate: the screenshot hook
rasterizes these views detached from any scene, and a verification image showing
a different accent from the shipping app is worse than no image.

---

## Not done, and why

- **`DebugScreenshot.swift` was not touched**, per the brief. It still renders
  the menu-bar preview without a screen wrapper, so that one capture is always
  Light. That is acceptable — the menu-bar popover is system chrome and was left
  system-drawn by design — but it means the menu bar is unverified in Dark by
  capture. It was reasoned about, not photographed.
- **`TextField` and `Menu` placeholders remain in captures.** Documented above.
  Substituting SwiftUI-drawn replacements to satisfy a screenshot tool would be
  the tail wagging the dog; a hand-rolled text field is worse than an NSTextField
  in every way that matters to someone typing a client's name into it.
- **The `Toggle` in Settings** has the same rasterization caveat, but Settings is
  not in the capture set at all.
- **No model changes.** Everything here is view-layer plus one development hook
  in `TraceCommonsAppMain.launch()`, beside the existing `TRACE_COMMONS_*` hooks.
- **The site's `--violet`** has no role in the app and was not imported. There is
  no fifth state to give it, and a colour without a meaning is decoration.
- **The bottom of the queue is still empty with a two-session fixture.** That is
  a property of a list with two items, not of the layout; filling it with
  decoration would be worse than the space.
