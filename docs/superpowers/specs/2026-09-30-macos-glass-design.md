# Glass for the Native macOS App — Design

Date: 2026-09-30. Revised 2026-10-02 to match Ron's decisions on #1173, and
again on 2026-10-02 to make Ron's design the source of truth.
Status: draft for review. Implemented in Ron's (rdisandro's) draft stack
#1178→#1184 (#1173's C2 and R1–R7).
**Precedence (Zaki, 2026-10-02).** Ron's implementation (#1178→#1184) and
his decisions on #1173 are the source of truth for the macOS glass work.
This spec documents them and records the shared requirements (consent copy,
behaviour kept from `main`, accessibility). Where this spec and Ron's code or
#1173 decisions disagree, the spec is wrong and follows Ron.
- Decided: dark only (D2), purple brand (D3), the minimum OS and glass on
  every supported macOS (D4), the Settings window (D8), the three-pane layout
  (D9), SF Pro and SF Mono, the token source and generator, Ron's
  custom-painted menus, popovers and toggles, eyebrow weight 600, and the
  Reduce Transparency base `paneOpaque`. See "Decisions".
- Open: the items marked open in "Decisions", "Tokens" and "Open".
Visual source: #1146, "Adopt the WYSIWYG UX and Glass design system" (open,
frozen as the design reference per #1173), at `a15fa6addf`, plus the
2026-09-30 screen recording indexed below.
Decided context: the native macOS app is kept and built to match the WYSIWYG
design (2026-09-28, in the #1118 review), and native SwiftUI is the main
client on macOS with no new Tauri work (#1173 D1).
Scope: `macos/` (the `TraceCommonsApp` target and the new `TCDesign`
target), the token source `design-tokens/glass.tokens.json`, and its Swift
generator. No product behaviour changes. No Tauri or GTK output.

## Changes since 2026-09-30

### Second revision (2026-10-02): Ron's design is the source of truth

Checked against the PR heads on 2026-10-02: #1178 `e6e05ac2`, #1179
`cab266ff`, #1180 `9bc8168d`, #1181 `44865ff2`, #1182 `aa64a1cc`, #1183
`a549b81b`, #1184 `2c701c2a`.

- **Precedence** stated above: the spec follows Ron's code and his #1173
  decisions.
- **Menus, popovers and toggles are Ron's custom-painted components**
  (`GlassMenu`, `GlassMenuItem`, `GlassPopover`, `GlassToggleStyle`), not
  the system `Menu`/`Popover`/`NSMenu` and `Toggle(.switch)` the first
  revision required. The accessibility those system controls gave for free
  is now a requirement on the custom components (see Components).
- **Eyebrow weight: decided, 600** (`semibold` in the JSON since #1179).
- **`windowControlsWidth`: satisfied, 78** (#1182).
- **Reduce Transparency base: decided, `paneOpaque` `#1C1E24`** (#1180).
- **Implementation status.** The doubled rim on 26 is fixed (#1180, a pane on
  Liquid Glass draws no edge of its own); floating surfaces blur before 26
  (#1181, `GlassFloatingBlur`); the pane layout matches D9, with the
  leading-width formula, the 1100pt rule, 10pt padding and gaps, the 760×560
  minimum and the single ease curve with Reduce Motion (#1182). The easing
  control points are now in the JSON (#1182).
- **Blur radii dropped.** The window, popover and scrim blur radii are no
  longer required tokens. Ron blurs with the HUD material (panes) and
  `GlassFloatingBlur` (floating surfaces) before 26, and with nothing under
  Reduce Transparency. See "No blur tokens" under Tokens.

### First revision (2026-10-02)

Ron recorded his decisions on #1173 on 2026-10-02, and zmanian's reviews of
#1178–#1182 (2026-10-02) listed where the code and this spec disagreed. This
revision adopts the decisions and keeps the requirements the reviews found
missing.

- **Dark only.** #1173 D2, confirmed by Ron on 2026-10-02. The light palette,
  light token fields, near.ai light neutrals and light pressed value are
  removed. Light appearance is out of scope, not deferred inside this spec.
- **SF Pro and SF Mono.** Ron's font decision on #1173 replaces Schibsted
  Grotesk, Mona Sans and JetBrains Mono. Nothing is bundled and no font
  licence is carried. The "Alignment with near.ai" section is reduced to the
  copy and third-party-mark rules, which the decision did not touch.
- **Token source and generator.** The source is
  `design-tokens/glass.tokens.json` (not `design/tokens/trace-commons.tokens.json`).
  `scripts/design-tokens/generate.py` writes
  `macos/Sources/TCDesign/Generated/GlassTokens.swift`; `TokenDriftTests` and
  `generate.py --check` guard drift. There is no separate drift CI job: the
  Swift suite, which CI already runs, is the guard (#1178).
- **No Tauri or GTK generators.** #1146 is frozen (#1173 D1 and Ron's
  2026-10-02 note); Windows and Linux stay on Tauri, frozen, until macOS ships
  (D13, a default). The acceptance item that diffed generated Tauri CSS
  against #1146 is dropped.
- **Panes.** D9 as decided by Ron: three floating glass panes, with the map
  and the inspector each hidden independently. The leading-width formula,
  the map-hidden-below-1100pt rule and 10pt window padding are unchanged
  (#1182 review).
- **Settings.** D8 confirmed: a macOS Settings window (⌘,), which may later be
  restyled to the glass theme.
- **Type scale.** Every step is a macOS text style (#1179). `micro` is added
  and `number` is 26 (largeTitle); both adopted. Eyebrow weight was open here;
  it is now decided as 600 (see the second revision).
- **Choices in Ron's code recorded as decisions**, each marked adopted or
  open: `paneBase`, `glassVeil`, the single `glassSurface` call site, the HUD
  material before 26, painted controls inside panes. See "Tokens" and
  "Materials by OS".
- **Kept as requirements** because the reviews found the code missing them:
  the blur values (since dropped for Ron's system materials; see the second
  revision), window radius 22, the ease curve, scene glows, the window
  rim, own edges only before 26, pressed as a darker fill, the Reduce
  Transparency base, Increase Contrast,
  Reduce Motion, arrow-key lists, no hard-coded wording, traffic-light
  clearance of at least 78pt, minimum window 760×560, the exercised pre-26
  fallback, and the accent contrast floor (#1178's blocking finding).

## Problem

#1146 moves the Tauri app onto a glass design system: layer tiers, a purple
brand, status colours, a three-pane "monitor" window. The native macOS app
should look the same. On `main` it shares almost nothing with that system:

- `macos/Sources/TraceCommonsApp/Views/DesignSystem.swift` defines a `TC`
  enum with a green-on-warm-grey palette (`ground` `#F6F7F4`/`#23251D` at
  line 463, `green` `#178F70`/`#3FBE9A` at line 522). It has smaller radii
  (`TC.Radius`, line 165), no materials and no glass. It supports both system
  appearances (`colorScheme`, line 752) and Dynamic Type (`dynamicTypeSize`,
  line 798).
- The window is a two-column `NavigationSplitView` with a 184pt sidebar
  (`MainWindowView.swift:194-196`). Its default size is 940×660 and its
  minimum 760×520 (`TraceCommonsAppMain.swift:52,63`). A `MenuBarExtra`
  provides the menu-bar item (`TraceCommonsAppMain.swift:38`).
- The package targets `.macOS(.v14)` (`macos/Package.swift:23`).
- No token pipeline exists on `main`. The palettes are hand-maintained. This
  spec fixes that for the macOS app only; the Tauri and GTK palettes are
  frozen with their shells (D1, D13).

#1146 is a good visual source for the geometry, the layer tiers and the
component inventory. It is not a build spec on its own:

- It is dark only (`app/app.css:12` sets `color-scheme: dark`), which this
  spec now matches (D2).
- Every size is a fixed pixel value.
- It handles reduced motion (`glass.css:1232`), but not reduced
  transparency, increased contrast or pressed states.
- It has no menu-bar design, and it leaves Compute with no screen.
- Some of its components carry product behaviour that the #1146 review asked
  to change.

This spec takes the look from #1146, takes the behaviour from `main`, and
fills in what a native build needs.

## Screen recording reference (2026-09-30)

The user supplied `Screen Recording 2026-09-30 at 4.14.23 PM.mov` from
`~/Documents` (the filename uses a narrow no-break space before `PM`). It is
2:49 long, 2864×1744, with SHA-256
`1d86ef5329bff2166a2fa25ca63c0ed97b4c5835fd0d80e7026bb0b7278a979b`.
The recording remains local; it contains real project names and paths.
Use synthetic data for committed screenshots and implementation fixtures.

This is a visual and interaction reference, not evidence that the native
app implements these screens. Its build revision is not established by
the recording. #1146 and `glass.tokens.json` remain the source for exact
token values; do not sample colours or infer point sizes from the scaled
video. The observed states below supplement that source. The native
adaptations and behaviour exceptions in this spec take precedence over the
recording.

| Time | Observed reference | Requirement carried into this spec |
|---|---|---|
| 00:00–00:20 | Centred onboarding pane, horizontal step progress, grouped choices, then a completion card | Keep progress and action hierarchy consistent across steps; retain the core's consent choices, disclosures and completion state. The clip begins partway through onboarding. |
| 00:25–00:38 | Home cards beside a large flow map and a Summary inspector; the map also has a Private AI mode | Keep the left tab selection separate from the map's Traces / Private AI selector. Preserve the summary's hierarchy of counts, decisions, statistics and runtime cards. |
| 00:45 | Map hidden; the main pane expands beside the inspector while the window is resized | Define the two-pane layout explicitly, with no empty map-width reservation. |
| 00:55 | Missions inside Home, with Back and a breadcrumb above stacked cards | Keep sub-navigation inside the main pane; retain the inspector alongside it. |
| 01:08 | Inference content scrolls while its Private AI inspector remains visible | Scroll each pane independently; use the core's status and credential copy. |
| 01:15–01:44 | Hierarchical trace rows, a stationary chart footer, selected-project details and a separate approval/Undo card | Selection, scrolling and approval feedback have separate state; do not turn row selection into submission. |
| 02:00 | Map and inspector hidden; the trace tree and chart span the main pane | Define the one-pane layout and preserve a way to reopen either pane. |
| 02:05–02:15 | Chart date hover and range change (11 to 42 days); checked “Show ignored folders” menu item | Specify chart inspection and range controls; keep the view filter separate from watch/consent settings. |
| 02:26–02:35 | Settings with a persistent section list and a scrolling detail area | Carry the navigation and grouped content into the native Settings window, not the recorded modal presentation. |
| 02:46 | Return to the trace tree with the inspector and the wider chart range | Preserve presentation state when opening and closing Settings. |

The brand, typeface and appearance questions the recording left open are now
settled by #1173 (purple, SF Pro/SF Mono, dark only). The recording does not
settle accessibility fallbacks. Its painted blue/green/purple scene
illustrates depth and hierarchy; it does not replace the desktop-backed
material decision for macOS 26.

## Non-goals

This spec takes the look only. Where #1146's components encode behaviour,
the macOS app keeps `main`'s behaviour (agreed by Ron on #1173). In
particular:

- **Folder "Submit" pill.** #1146 bulk-approves with one click and drops the
  core's withheld line. The macOS app keeps the pill's look but routes it
  through the existing approval sentence and shows `withheld_line`.
- **Watch switch.** #1146 collapses three project modes into two. The macOS
  app keeps the switch's look and `main`'s three-way choice (auto upload,
  notify only, ignore) with the core's disclosures.
- **Private AI status.** #1146 hand-types "answer on NEAR AI" and "answer at
  their vendors". The macOS app renders the core's copy
  (`private_inference_copy`), never retyped strings. It takes the dot and the
  legend components, not the sentences.
- **Compute.** It stays reachable, with its pause, resume and withdraw
  controls.
- **Consent copy.** It always comes from the core. This spec adds no new
  sentence that promises privacy or security. The #1146 shield-and-check
  icon on the credential node is not carried over.
- **Light appearance.** Out of scope (D2). The app forces the dark
  appearance on its windows. A future light mode needs its own decision and
  its own token values; nothing in this spec reserves them.
- **Tauri and GTK.** No generated output for either shell (D1, D13). #1146's
  Tauri glass bridge (`set_glass_regions`) is not carried forward.

## Copy and third-party marks

The 2026-09-30 draft aligned the app with near.ai's brand (typefaces, light
neutrals). Ron's font decision and D2 supersede that; the near.ai faces and
light palette are gone. Two rules from that section were not touched by
#1173 and still apply:

- **Voice.** Direct, and centred on proof rather than promises. Our copy rules
  still exclude any sentence that promises privacy, and consent copy still
  comes from the core.
- **Logo.** near.ai publishes no rules for third-party use. The app shows no
  near.ai logo or lockup, for example on the Private AI screen, without
  near.ai's written permission. The name "NEAR AI" appears in text only
  where the core's copy uses it.

## Decisions

From #1173's table and Ron's 2026-10-02 comment on it.

1. **Appearance: decided, dark only (D2).** Confirmed by Ron on 2026-10-02.
   The light palette proposed on 2026-09-30 is withdrawn.
2. **Brand: decided, purple `#6D14F3` (D3).** It replaces the community green
   (`#178F70`) in the macOS app, including in the shipping `TC` palette
   (#1178). Whether the community site follows is still open.
3. **Minimum OS: decided, macOS 14 with glass on every supported macOS
   (D4).** The app is built with the macOS 26 SDK (Xcode 26) and keeps
   `.macOS(.v14)` as its deployment target.
   - Liquid Glass on macOS 26; the system material on 14–25.
   - Glass-only APIs sit behind `#available(macOS 26, *)` in one place (see
     Materials by OS).
   - Every custom surface therefore has two renderings, both exercised (see
     Acceptance).
4. **Settings: decided, a macOS Settings window (⌘,) (D8).** Ron may later
   restyle it to the glass theme rather than the stock window look.
5. **Token source: decided, typed JSON at `design-tokens/glass.tokens.json`,
   generating Swift only** (#1173 R1, built in #1178).
6. **Typefaces: decided, SF Pro and SF Mono** (Ron on #1173, built in
   #1179). The type scale maps to macOS text styles.
7. **Window layout: decided, a custom three-pane layout, not
   `NavigationSplitView` (D9).** Three floating glass panes with gaps; the map
   and the inspector are each hidden independently from the toolbar.
8. **Map tab label: decided, "Private AI" (D11).**
9. **Menus, popovers and toggles: decided, Ron's custom-painted
   `GlassMenu`, `GlassPopover` and `GlassToggleStyle`** (#1178), carrying the
   accessibility obligations listed in Components.
10. **Eyebrow weight: decided, 600** (#1179).
11. **Reduce Transparency base: decided, `paneOpaque` `#1C1E24`** (#1180).

## Tokens

### One source, one output

`design-tokens/glass.tokens.json` is the only file anyone edits.
`scripts/design-tokens/generate.py` writes
`macos/Sources/TCDesign/Generated/GlassTokens.swift`, which the `TCDesign`
components and the app's `TC` palette read.

Drift is guarded twice, both inside existing CI:

- `python3 scripts/design-tokens/generate.py --check` exits non-zero if the
  Swift file is out of date;
- `TokenDriftTests` (in `macos/Tests/TCDesignTests`) decodes the JSON and
  compares every token, value by value, with the generated constants.

There is no separate drift job. The generator also validates the source: it
refuses a type step whose size is not its text style's default size.

The JSON was seeded from #1146's `tokens/*.css` at `09a26c0d` (the revision
the JSON names). The values listed below were checked against #1146 at
`a15fa6addf`. Composite CSS tokens (multi-inset edges, gradients,
`color-mix`) are expressed as typed groups, because Swift cannot read them as
strings:

| Group | Fields |
|---|---|
| `color` | `hex`, optional `alpha`, optional `note` |
| `gradient` | `angle`, and `stops` of (`hex`, `alpha`, `at`) |
| `shadow` | an array of layers: `x`, `y`, `blur`, `hex`, `alpha`, `inset` |
| `radius`, `space`, `size` | points |
| `opacity` | 0–1 |
| `type` | `textStyle`, `size`, `weight`, `lineHeight`, optional `tracking`, `uppercase`, `tabular`, `design` |
| `motion` | durations in seconds |

Colours carry one (dark) value; there are no light fields (D2).

**Required tokens the JSON does not yet carry.** These are spec values, not
optional; the #1181 and #1182 reviews found them missing. Each needs a token
and a generated constant:

- **Window radius:** 22 (the JSON's `radius` group stops at pane 16).
- **Scene glows:** `#3F6A8A`, `#6A3F7A`, `#2F6B5A` (the JSON has only
  `sceneBase` and `sceneWarm`).
- **Window rim:** #1146's window-tier edge, as a `shadow` entry.
- **Increase Contrast values** (see Appearance). How they are represented is
  open: a parallel `highContrast` value on each affected token, or a rule
  applied in code. Either is acceptable if the generator and the drift test
  cover it.

**No blur tokens.** #1146's CSS blur radii (window 30 at 180% saturation,
popover 24 at 170%, scrim 6) are not carried into the JSON. Ron's code
(#1180, #1181) blurs with system materials instead, which have no radius to
set:

- **Panes before 26:** the HUD material (`NSVisualEffectView`,
  `.hudWindow`, blending behind the window), under `glassVeil` and the pane
  sheen (`GlassBackdrop`, `GlassPaneFill`). On 26 the pane is
  `NSGlassEffectView`.
- **Floating surfaces before 26** (popovers, menus, node cards, controls
  over the map): the painted tier over the HUD material blending within the
  window, so it blurs what the surface floats on (`GlassFloatingBlur`). On
  26 they are Liquid Glass through `glassSurface`.
- **Reduce Transparency:** no material at all. A pane is the solid
  `paneOpaque`; a floating surface is its painted tier alone.
- **Scrim:** the system sheet dimming (see Components), so no scrim blur.

### Values taken from #1146

These are exact, from `tauri-desktop/frontend/src/design-system/tokens/` at
`a15fa6addf`, and match the JSON unless noted.

- **Brand:** purple `#6D14F3`, purple soft `#8A3DFF`, purple text `#C9B3FF`,
  blue `#3A7BD5`.
- **Text:** primary `#F2F2F4`, secondary `#C9C9D0`, tertiary `#B4B4BC`, on
  accent `#FFFFFF`, on status `#0C0C0E`.
- **Status**, for glyphs and labels only, never fills: on `#3DDC84`, ask
  `#F5C142`, off `#A9A9B0`, outside `#FF6B6B`. The one exception is Ron's
  `watchOn` (`#3DDC84`), the watch switch's on fill.
- **Data:** shared `#8A3DFF`, kept `#3A7BD5`, inference `#A78BFA`.
- **Scene:** base `#0F1219`, warm `#1D2430`, glows `#3F6A8A`, `#6A3F7A`,
  `#2F6B5A` (glows not yet in the JSON).
- **Radii:** window 22 (not yet in the JSON), pane 16, card 14, small card
  12, control 8, pill 999. The JSON adds tile 6 and checkbox 5.
- **Motion:** ease `cubic-bezier(0.2, 0.8, 0.2, 1)` (`easeX1`…`easeY2`
  since #1182);
  durations 150, 220 and 300 ms.
- **Spacing:**
  - scale 2, 4, 6, 8, 10, 12, 14, 16, 20, 24;
  - padding: window 10, pane 12, card 12×14;
  - gaps: panes 10, cards 10, inline 6;
  - sizes: control 26 (28 large), checkbox 15, dot 7, tab 26, glyph 16, tool
    tile 22;
  - widths: left pane 400 (maximum; see Window and layout), inspector 300;
    window height 760.
- **Tiers:** the fills and edges for scene, window rim, pane, card, well,
  control and popover. #1146's blur radii are not carried over; see "No
  blur tokens" above.

### Accent contrast floor

`#C9B3FF` (`purpleText`) is a text colour only. Every accent fill and tint,
including the window-level `.tint`, `.borderedProminent`, checkbox and
check-circle fills, uses `#6D14F3` (`primaryFill`) or the gradients built
from it. A glyph or label drawn on an accent fill reaches at least 3:1
against it; white on `#6D14F3` is about 6.9:1, white on `#C9B3FF` about
1.85:1. This is the #1178 blocking regression and applies to the shipping
`TC` palette as well as `TCDesign`.

### Type

Sizes and line heights come from #1146's `typography.css`, except where a
decision below changes them. Faces are SF Pro, with SF Mono for `mono`
(decided). Each step is `Font.system(<textStyle>, design:)`, so it follows
the macOS text-size setting; leading and tracking scale with it (#1179).

| Style | Size/line height | Weight | `textStyle` | Note |
|---|---|---|---|---|
| micro | 10/13 | bold | caption2 | new; tags and badges. **Adopted.** |
| eyebrow | 10/14 | 600 (semibold) | caption2 | uppercase, tracking 0.08em. **Decided** (#1179). |
| caption | 11/16 | regular | subheadline | |
| label | 12/16 | medium | callout | |
| body | 13/18 | regular | body | |
| body strong | 13/18 | semibold | body | |
| title | 15/20 | semibold | title3 | |
| heading | 17/22 | semibold | title2 | |
| display | 22/28 | bold | title | tracking -0.01em |
| number | 26/34 | bold | largeTitle | tabular figures. **Adopted** (#1146 had 28). |
| mono | 11/16 | regular | subheadline | SF Mono |

- **`number` at 26: adopted.** Ron chose it because every step maps to a text
  style at its default size, and largeTitle is 26. The #1179 review's
  alternative, 28 via `@ScaledMetric(relativeTo: .largeTitle)`, is not
  required.
- **`micro`: adopted.** It covers tags and badges, the only words without a
  step.
- **Eyebrow weight: decided, 600.** #1178 had bold (700); from #1179 on the
  JSON has `semibold`, matching #1146.
- **Labels in fixed-height controls** must not clip at large text sizes: use
  a minimum height, not a fixed one, where the label scales (#1179 review).
- **Fixed point sizes** in components are forbidden except for glyphs inside
  fixed-size controls (`glassGlyph`), as `FixedPointTypeTests` enforces.

### Tokens Ron added: adopted or open

- **`paneBase` `#161A22` at 0.96: adopted** as the neutral dark under a
  pane's glass where there is no native material. Not adopted as the Reduce
  Transparency base; see Appearance.
- **`glassVeil` `#0C0E14` at 0.28: adopted.** A thin dark veil over native
  glass that keeps text at contrast, as #1146's native-glass CSS does. Text
  contrast is checked with the veil, on a light and a dark desktop.
- **`windowControlsInset` 30 and `windowControlsWidth` 78: satisfied.**
  #1182 raised the width from 76 to 78, the traffic-light clearance in
  Window and layout.
- **`toggleWidth`/`toggleHeight` 40×24 and
  `watchSwitchWidth`/`watchSwitchHeight` 38×22: adopted.** They size Ron's
  `GlassToggleStyle` (see Components), with `toggleOn` `#3A7BD5`,
  `toggleOnSettings` `#8A3DFF`, `watchOn` `#3DDC84` and `toggleOff` white at
  0.18.
- **`mapWidth` 600: open, still in the JSON.** As built in #1146 the map is
  the remaining width (580 at 1320 wide, matching the flow map's `viewBox`).
  The recommendation stands: drop the token and define the map as the
  remaining width.
- **Tool tints (`toolClaude` and so on) and `tileFolder`: adopted.**
  Antigravity and Gemini CLI share `toolAntigravity`; whether Gemini CLI gets
  its own tint is open.

**#1146's literal motion timings.** Six timings in `glass.css` and
`monitor-shell.tsx` are literals outside the tokens. Map each one to the
three durations or add a named token for it.

## Appearance

- **Dark only** (D2). The app's windows use the dark appearance whatever the
  system setting. There are no light values.
- **Increase Contrast.** Edges become a solid 1pt stroke at 40% text colour,
  fills gain 50% opacity, and status glyphs get their label as well as the
  colour. No PR in the stack handles it yet; #1173 schedules it as R14.
- **Reduce Transparency** (`accessibilityReduceTransparency`). Every tier
  swaps blur and translucent fill for an opaque fill: pane `#1C1E24`, card
  one step lighter. This is required, because #1146 has no fallback.
  **Decided:** the pane base is the `paneOpaque` token, `#1C1E24`, solid
  (#1180). Floating surfaces under Reduce Transparency get their painted
  tier alone, with no blur (#1181).

## Materials by OS

| Tier | macOS 26 and later | macOS 14–25 |
|---|---|---|
| Scene | the desktop, through a transparent window background; #1146's painted scene only behind the first-run pane | the system material behind the window (below) |
| Pane | Liquid Glass (`NSGlassEffectView` or `.glassEffect`) in a 16pt continuous rounded rectangle, with `glassVeil` and the pane sheen | the system material in the same shape, plus the pane edge as a gradient stroke overlay |
| Card / well / control inside a pane | a painted tint with no blur (the fill colour at its opacity) | the same |
| Control or card floating over the map | Liquid Glass, through the single call site | the painted tier over a within-window HUD blur (`GlassFloatingBlur`) |
| Primary button | `.buttonStyle(.glassProminent)` tinted `#6D14F3`, or a painted fill of the CTA gradient | a filled button, the CTA gradient |
| Popover / menu (`GlassPopover`, `GlassMenu`) | Liquid Glass through `glassSurface(_, floating: true)`, tinted with `glassVeil` | the painted popover or menu tier over a within-window HUD blur (`GlassFloatingBlur`) |

- **Material before 26: HUD vibrancy, adopted subject to the macOS 14
  check.** #1180 uses `NSVisualEffectView` with the `.hudWindow` material
  behind the window, the same choice #1146's `tc_glass_view_make` makes for
  Tauri. D4 says "the system material", which this satisfies, and with a
  dark-only app the HUD material is the closer match. The 2026-09-30 spec's
  `.regularMaterial` is no longer required. The pane edge stroke is still
  required before 26.
- **One call site: adopted.** #1181's `glassSurface` is the one place a
  surface becomes Liquid Glass. It holds the `#available(macOS 26, *)` check,
  honours Reduce Transparency, and a source-scan test fails if anything else
  calls `.glassEffect`. That test satisfies the Acceptance item on
  glass-only APIs. Screens never branch on the OS themselves.
- **Painted controls inside panes: adopted.** #1180 keeps cards, wells and
  controls inside a pane painted, which is the "no glass on glass" rule. The
  2026-09-30 idea of `.buttonStyle(.glass)` inside a `GlassEffectContainer`
  for every control is dropped.
- **Glass controls over the map: open.** #1181 floats Liquid Glass controls
  and node cards over the map, which itself sits in a Liquid Glass pane.
  Whether that reads as glass on glass is decided on a real device.
- **Specular edges** (the multi-inset CSS edges, `glassEdge`) are not drawn
  on a Liquid Glass surface: on 26 the system draws its own rim, and adding
  ours doubles it. Painted tiers (cards, wells and controls inside a pane)
  keep their edges on every OS, and every tier draws its edge before 26.
  Fixed in #1180 (`drawsOwnEdge`).
- **Floating blur before 26.** Popovers, menus, node cards and controls that
  float are not a bare translucent fill: they are the painted tier over a
  within-window HUD blur of what they float on (`GlassFloatingBlur`, #1181).
  Under Reduce Transparency they are the painted tier alone.
- **Saturation** (`saturate(180%)`) has no public SwiftUI control. Omit it.
- **Glass-only APIs** (`NSGlassEffectView`, `.glassEffect`,
  `GlassEffectContainer`, `.buttonStyle(.glass)`, `.glassProminent`) are
  used only inside the tier code and button styles, behind the availability
  check.
- **The fallback is not pixel-identical.** Materials look flatter than
  Liquid Glass. The fallback is judged on layout, radii, colour and
  legibility, not on refraction.

## Window and layout

- **Shape.** One window of three floating glass panes over the scene (D9),
  with a transparent full-size title bar and the real traffic lights over
  the left pane. The #1146 traffic-light stand-ins are not built. The
  window corner radius is 22.
- **Size.** Default 1320×760; minimum 760×560, as in #1146. The declared
  minimum and the content minimum agree: at 760 wide the panes fit without
  clipping, because the map hides below 1100pt and the leading pane shrinks
  to 320 (#1182 review found a content minimum of about 1080 against a
  declared 600).
- **Panes.** `NavigationSplitView` does not fit (D9): its leading column
  cannot grow to fill the window when the middle column hides, and the map
  and inspector hide independently. Instead:
  - An `HStack` of three panes, with 10pt window padding and 10pt gaps.
  - Left pane: width `min(400, max(320, 0.34 × window width))`, the same
    formula as `monitor-shell.tsx:97`. It fills the window when the map is
    hidden, less the inspector width and gap when the inspector is open.
  - Map: the remaining width.
  - Inspector: 300pt. Its presentation must participate in the same pane
    layout; use `.inspector(isPresented:)` only if it preserves the specified
    width, gap and independent visibility, otherwise compose the third pane
    directly in the `HStack`.
  - **What hides.** The map and the inspector, each independently. The left
    pane, which holds the tabs, never hides; hiding it removes all
    navigation (#1182 review).
  - Supported compositions: main + map + inspector, main + map, main +
    inspector (00:45), and main alone (02:00). Hidden panes reserve no space.
    As a native compact-layout rule, temporarily hide the map below 1100pt
    window width. Keep the user's preferred visibility separately so widening
    restores it; an explicit hide stays hidden. This breakpoint is a design
    requirement, not a size measured from the recording.
- **Toolbar.** A trailing-aligned capsule above the main pane's tabs, as in
  the recording, with a view menu and graph, map and inspector toggles,
  followed by a separate round Settings button. Reserve at least 78pt at the
  leading edge for the real traffic lights; this is a clearance, not a fixed
  origin for the capsule. Tabs: Home, Inference, Traces; the map selector's
  second mode is labelled "Private AI" (D11), as supplied by the core's
  copy, not typed in the shell. Preserve the Traces count badge
  and the Inference status dot, with accessible text equivalents; an unknown
  status has no dot, never an "off" dot. Every icon has a tooltip and
  accessible name; toggles expose their state.
- **Opening state and restoration.** #1146 decides pane visibility once at
  launch (the map if the window is at least 1100 wide, the inspector if at
  least 900). The macOS app restores the user's last choice per window
  (`@SceneStorage`), and uses those widths to seed first-launch preferences.
  The compact-layout rule above affects rendered visibility, not the saved
  preference.
- **Behaviour that must not depend on the inspector.** In #1146, the undo
  window, the arming offer and the review exist only while the inspector is
  open. The macOS app shows the undo bar and consent offers regardless of
  which panes are visible, anchored to the bottom of the left pane when the
  inspector is closed.
- **Settings.** A macOS Settings window (D8). Its sections are the ones
  #1146's modal lists, plus Compute (see Screens).

### Recorded content and interaction details

- **Independent scrolling.** Tabs and toolbar remain above the main content.
  The trace tree scrolls within that pane; its visible chart footer stays
  below the tree (01:15). The inspector scrolls independently. At the minimum
  window size and larger text sizes, all controls remain reachable without
  text or buttons overlapping; the graph toggle can reclaim vertical space.
- **Trace rows.** Preserve tool → project → session indentation, disclosure
  controls, tool/folder glyphs, primary name and secondary state/count line.
  Align trailing actions across rows. A selected row has a full-width
  highlight distinct from hover, keyboard focus and contribution status.
  Clicking a disclosure or an action must not also trigger the row's other
  actions. Empty projects retain their row and an unavailable Submit action.
  The recorded binary switches do not replace the three contribution modes
  specified in Non-goals.
- **Inspector context.** With no selection, show Summary; a project/session
  selection shows its details, and Inference shows Private AI details. Keep
  headings, paired shared/kept count wells, disclosure sections and quiet
  runtime cards in that order where applicable. Clearing selection returns
  to the tab's overview. Pane toggles and resizing retain selection and scroll
  position. Approval feedback is a separate region above the current details,
  so changing selection cannot dismiss an active undo opportunity. Keep the
  existing core deadline and result handling; the recorded countdown is not
  a new duration or a guarantee that an upload can still be cancelled.
- **Flow map.** Put the Traces / Private AI segmented selector at the map's
  upper trailing edge. It changes the map view, not the main tab, consent or
  routing configuration. Retain its choice while the map is hidden. Use the
  same node/arc hierarchy in both modes, with labels and an accessible
  alternative to pointer-only node inspection. The recorded credential
  shield and hand-written routing copy remain excluded by Non-goals.
- **Chart footer.** Paired shared/kept count wells sit above day columns;
  period navigation, range controls and a central date/range label sit below.
  Match the purple/blue series distinction and the recorded blue kept bars
  below the baseline, with values supplied by the core. Hovering a day shows its
  date and counts (02:05); keyboard focus provides the same information.
  Leaving the day restores the range summary. Changing the range recomputes
  column spacing and date-label density, including the recorded 11- and
  42-day states, without changing the underlying contribution state. Keep
  the chart range through pane changes and Settings. Colours alone never
  distinguish the series, and zero values are not missing data.
- **View menu.** “Show ignored folders” is a checked visibility option
  (02:15). Showing a folder never resumes watching or changes its rule.
- **Settings navigation.** Inside the native Settings window, retain the
  section list and grouped, independently scrolling content from 02:26:
  Connection, Startup & notifications, Watching, How traces may be used,
  Public profile, Watched folders, Tools, Private AI, Redaction witness,
  Projects, and Changes on this machine, plus Compute. The selected section
  must match the displayed content. Opening or closing Settings preserves
  the main window's tab, selection, pane visibility and chart range. This
  adapts the reference's organisation without adopting its scrim or close
  button inside the main window.

## Components

These are the #1146 components (`design-system/index.ts`), with the macOS
build of each; #1178's `TCDesign` contract is their public API. Every one
needs these states: default, hover, pressed, focus, disabled, and selected
or on/off or mixed where it applies. #1146 has no pressed state (`:active`)
and no focus style on rows and map nodes, so both are defined here.

- **Pressed** is the fill 8% darker, applied with the fast duration. Not a
  change of opacity (#1178 review found opacity 0.8).
- **Focus** is the system focus ring (`.focusable()` plus the default ring),
  not #1146's custom 2px purple ring. That keeps VoiceOver and Full Keyboard
  Access consistent.

**Custom-painted menus, popovers and toggles.** Ron paints these himself
(#1178, `macos/Sources/TCDesign`), rather than using the system `Menu`,
`Popover`, `NSMenu` and `Toggle(.switch)`:

- `GlassMenu` (minimum width 220, 5pt inset, the menu tier at radius 12)
  holding `GlassMenuItem` rows (an optional leading check) and
  `GlassMenuSeparator`;
- `GlassPopover` (the popover tier at radius 14), also the menu-bar panel;
- `GlassToggleStyle(.standard | .settings | .watch)`: a 40×24 switch
  (`toggleWidth`/`toggleHeight`), or 38×22 for the watch switch on tree rows
  (`watchSwitchWidth`/`watchSwitchHeight`), with an 18pt white knob; on
  colours `toggleOn`, `toggleOnSettings` and `watchOn`, off `toggleOff`.

The system controls brought accessibility for free; these do not, so each
must supply it:

- **Keyboard.** Every menu item, toggle and popover control is reachable by
  Tab and Full Keyboard Access and shows the system focus ring. Space or
  Return activates the focused item; a menu or popover closes with Escape
  and returns focus to the control that opened it.
- **VoiceOver.** Each control exposes the role of the system control it
  replaces: a toggle reads as a switch with its on/off state (Ron's
  `accessibilityRepresentation` of a native `Toggle` does this), a checked
  menu item reads as selected, and a menu or popover is one container. Labels
  and values are supplied by the caller, from the core's copy; the
  components hard-code no wording, including state words.
- **Reduce Motion.** The knob slide and any menu or popover transition are
  removed when Reduce Motion is on (`GlassMotion.systemReducesMotion`).
- **Contrast floor.** A knob, check or label on an on-state fill reaches at
  least 3:1 against it, and the off state is distinguishable from the on
  state by more than colour. White on `toggleOn` is about 4.2:1 and on
  `toggleOnSettings` about 5.0:1; white on `watchOn` (`#3DDC84`) is about
  1.8:1 and is open (see Open).
- **No wording in components.** Components author no words, including
  accessibility values and default labels ("on"/"off", "checked",
  "expanded", "More", "Choose…", tile text such as "dir"). Callers pass every
  string in, from the core's copy, or the component uses a native control or
  trait that supplies it (#1178 review).

| #1146 component | macOS build |
|---|---|
| Window, Pane | the Window and layout section |
| Popover, Menu | Ron's custom-painted `GlassPopover` and `GlassMenu` (with `GlassMenuItem`, `GlassMenuSeparator`), meeting the accessibility requirements above |
| Modal, Sheet | `.sheet`; Settings is its own window |
| Scrim | the system sheet dimming |
| Card (quiet, flush, interactive), Well | custom `TCGlassCard` using the tier tokens |
| ConsentBlock | custom; text comes from the core only |
| ListRow, tree | `List` / `OutlineGroup` with a custom row (grid columns 16/24/flex/auto/38/22); one list with arrow-key selection, not a tab stop per row and not `.onTapGesture` selection, so Full Keyboard Access can select a row |
| TableRow/Head, KeyValueList | `Grid` / `Table` |
| SegmentedTabs | `Picker(.segmented)`; badge and dot in a custom segment label |
| Breadcrumb, StepProgress | custom |
| Buttons (primary, secondary, glass, round, pill icon, icon, submit, folder, kebab) | `ButtonStyle`s over the tier tokens; see Materials by OS |
| Picker | `GlassPicker`: a native SwiftUI `Menu` dressed as a glass pill; the placeholder comes from the caller |
| Toggle (plain, settings, watch) | a SwiftUI `Toggle` with Ron's `GlassToggleStyle` (`.standard` 40×24, `.settings` 40×24, `.watch` 38×22), meeting the accessibility requirements above |
| Checkbox (with mixed) | a SwiftUI `Toggle` with Ron's `GlassCheckboxStyle` (15pt); the mixed state's spoken value comes from the caller; its accessible name is its full sentence, never "Option" |
| Expander | `DisclosureGroup` |
| TextField | `TextField` with a custom style |
| Dot, Status, Chip, Tag, Badge | custom; the colour is always paired with a label; "clear" or "on" glyphs use `statusOn`, not the accent |
| ToolTile, logos | vector assets converted from `logo-paths.ts` (Claude, Codex, Antigravity, OpenCode, Theia) at 15pt in the 22pt tile and 20pt in the 30pt tile; initials until then, and for Gemini CLI and Cline, which have no artwork |
| BarGraph | Swift Charts |
| MapNode, MapArc, NodeCard (flow map) | `Canvas` plus `TimelineView` for the moving dashes; each node is an accessibility element with a label and actions, not a picture |

The preview gallery (`GlassGallery`, `swift run TCDesignGallery`) is a
development tool and does not compile into the release app (`#if DEBUG` or a
separate target).

## Motion

- One curve, `Animation.timingCurve(0.2, 0.8, 0.2, 1, duration:)`, at 0.15,
  0.22 and 0.30 seconds, read from the easing and duration tokens. Not
  `.easeInOut` or `.easeOut` (#1182 review).
- #1146's six literal timings are mapped to these, per the Tokens section.
- Reduce Motion (`accessibilityReduceMotion`) removes transitions, including
  pane show and hide, and the flow-map dash animation, as `glass.css:1232`
  does.

## Menu-bar popover

#1146 has no design for the menu bar: it has only a doc comment, "Popover
tier: menu-bar panel, floating menus." (`components/surfaces.tsx:44`). `main`
has a `MenuBarExtra`. This spec defines its look, not its content:

- a `MenuBarExtra(.window)` panel drawn as a `GlassPopover`: the popover
  tier, radius 14;
- a status row (dot plus label from the core);
- the decisions-owed badge;
- the actions the current menu already has.

Content stays whatever `main`'s menu offers today.

## Screens

Every screen in the current app maps to the new shell. Nothing becomes
unreachable.

| Screen | Where it goes |
|---|---|
| Home | the Home tab |
| Traces (waiting, review) | the Traces tab: tree in the left pane, review in the inspector |
| Inference (Private AI) | the Inference tab, with core copy |
| History | Home, then History (breadcrumb) |
| Missions | Home, then Missions |
| Compute | a Settings section, including pause, resume and withdraw |
| Insights | a Home sub-view, or its existing view, as long as it stays reachable (including its delete actions) |
| Onboarding / first run | a single pane over the scene, with StepProgress |
| Settings | the Settings window |
| Menu bar | the menu-bar popover |

## Icons

The toolbar glyphs in #1146 are inline SVG (`monitor-toolbar.tsx`). Map
each to an SF Symbol:

| Glyph | SF Symbol |
|---|---|
| view menu | `line.3.horizontal.decrease.circle` |
| graph | `chart.bar` |
| map | `point.3.connected.trianglepath.dotted` |
| inspector | `sidebar.right` |
| settings | `gearshape` |
| binoculars | `binoculars` |

With no other shell consuming them, the mapping may live in Swift; an
`icons` group in the JSON is optional.

The tool logos are third-party marks. They are shipped as vector assets only
after a trademark-use check.

## Implementation order

Each step is a PR of its own. #1173's numbering is in brackets.

1. **Token source and generator [R1].** The JSON, `generate.py`,
   `GlassTokens.swift`, `TokenDriftTests`, and the green palette's removal.
   In #1178 (with C2). Remaining: the missing tokens listed under Tokens,
   and the accent contrast fix.
2. **Type scale [R2].** Text-style mapping, `micro`, the fixed-point guard.
   In #1179. No fonts are bundled.
3. **Materials [R3].** The native backdrop for both OS paths, Reduce
   Transparency (`paneOpaque`). In #1180, including no own pane edge on 26.
4. **Components [R4].** The glass rendering behind C2's API, the single
   `glassSurface` call site, the tool logos, Ron's `GlassMenu`,
   `GlassPopover` and `GlassToggleStyle`, and the floating blur before 26.
   In #1178 and #1181. Remaining: the accessibility requirements on the
   custom menus, popovers and toggles (see Components), pressed fill, no
   wording in components, arrow-key lists.
5. **Window shell [R5].** Three panes, the toolbar, tabs, restoration, all
   four pane compositions, independent scrolling, and the Settings window.
   In #1182 (debug-only), including the hiding contract (map and inspector,
   never the left pane), the leading-width formula, the 1100pt rule, 10pt
   padding, the 760×560 minimum, 78pt clearance, the ease curve and Reduce
   Motion.
6. **Screens [R6–R12].** Move screens one at a time, keeping `main`'s
   behaviour and the core's copy. Include the recorded inspector contexts,
   map modes, trace-tree selection, chart footer and Settings section
   navigation. R6 is #1183.
7. **Menu-bar popover [R13].**
8. **Accessibility [R14].** Increase Contrast values and handling, focus,
   VoiceOver. Reviewed by the brand owner.

## Acceptance

- `python3 scripts/design-tokens/generate.py --check` passes and
  `TokenDriftTests` pass; every required token in "Tokens" exists in the JSON
  and the generated Swift.
- At 1320×760 on macOS 26, screenshots match #1146's monitor window in
  layout, radii and colours. They are not expected to match the painted
  scene, because real glass shows the desktop.
- Every accent fill and tint uses `#6D14F3`/`primaryFill`; `#C9B3FF` appears
  only as text; every glyph on an accent fill reaches at least 3:1, checked
  for the consent read-gate checkbox, check circles and `.borderedProminent`
  buttons in the shipping app.
- Replay the recording's presentation sequence with synthetic data: partial
  onboarding → Home and both map modes → main + inspector → Missions →
  Inference → Traces and project/session selection → main alone → chart
  inspection and range change → view filter → Settings → restored Traces.
  Exercise main + map as well, though that composition is not established
  by the sampled recording states. Verify all four pane preferences at the
  default size, then resize to the minimum (760×560) and back: nothing
  clips, the map collapses below 1100pt and returns only when the saved
  preference calls for it, and the left pane is never hidden.
- During that replay, tree scrolling leaves the visible chart footer in
  place; selection updates inspector context without submitting anything;
  chart and ignored-folder controls do not mutate contribution settings;
  Settings round trips preserve presentation state. Long synthetic names
  and larger text sizes do not cover trailing row actions or clip labels in
  fixed-size controls.
- Repeat the approval/Undo flow with the inspector closed and while changing
  tabs and pane visibility. Core disclosures and the active undo opportunity
  remain reachable, and the deadline is not restarted by a layout change.
- On macOS 14, the same screens render with the material fallback, with no
  missing panes or controls, and with our own pane edges drawn; on macOS 26
  our edges are not drawn on Liquid Glass surfaces.
- **The pre-26 fallback is exercised, not only compiled.** CI's only macOS
  runner is `macos-26`, so compiling against `.macOS(.v14)` proves the
  availability guards but never runs the vibrancy branch. Before the shell
  ships, either CI runs the Swift suite on a macOS runner below 26, or the PR
  records a documented manual check on a macOS 14 machine (screenshots of
  each tier and the Reduce Transparency state). #1173 already asks Ron to
  review on both 26 and 14.
- The package still declares `.macOS(.v14)`, and the source-scan test fails
  the build if a glass-only API is used outside the single call site. The
  compiler's availability errors cover most of this; the test covers the
  rest.
- VoiceOver reaches and names every control, including flow-map nodes and
  tree rows. Full Keyboard Access can select and operate the tree with arrow
  keys. No component supplies its own English wording.
- Reduce Transparency, Increase Contrast and Reduce Motion each change what
  this spec says they change.
- No screen from the current app is unreachable (see Screens).
- The existing macOS test suite and the Swift CI job stay green. No consent,
  withdrawal or Private AI behaviour changes.

## Open

- Whether the community site adopts the purple brand (Decision 2).
- Where Insights lives long term. This spec only requires that it stays
  reachable.
- The Liquid Glass tint values per tier on macOS 26. They are tuned on
  device and recorded in the JSON.
- The watch switch's knob contrast: white on `watchOn` (`#3DDC84`) is about
  1.8:1, below the 3:1 floor in Components. Ron to choose the fix (a darker
  knob, a darker on colour, or a state cue besides colour).
- Glass controls and node cards floating over a map that is itself in a
  Liquid Glass pane (#1181): decide on a real device.
- How Increase Contrast values are represented in the JSON.
- Removing the unused `mapWidth` token.
- A separate tint for Gemini CLI, and artwork for Gemini CLI and Cline.
- Restyling the Settings window to the glass theme (Ron, D8: "theming may
  follow").
