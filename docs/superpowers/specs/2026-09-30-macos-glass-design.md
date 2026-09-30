# Glass for the Native macOS App — Design

Date: 2026-09-30
Status: draft for review. Nothing here is implemented, and the decisions under
"Decisions needed" are open
Visual source: #1146, "Adopt the WYSIWYG UX and Glass design system" (open,
changes requested), at `a15fa6addf`
Decided context: the native macOS app is kept and built to match the WYSIWYG
design (2026-09-28, in the #1118 review)
Scope: `macos/` (the `TraceCommonsApp` target), one new token source, and the
generators that feed `tauri-desktop/frontend` and
`crates/trace-commons-contributor-gtk`. No product behaviour changes.

## Problem

#1146 moves the Tauri app onto a glass design system: layer tiers, a purple
brand, status colours, a three-pane "monitor" window. The native macOS app
should look the same. Today it shares almost nothing with that system:

- `macos/Sources/TraceCommonsApp/Views/DesignSystem.swift` defines a `TC`
  enum with a green-on-warm-grey palette (`ground` `#F6F7F4`/`#23251D` at
  line 463, `green` `#178F70`/`#3FBE9A` at line 522). It has smaller radii
  (`TC.Radius`, line 165), no materials and no glass. It supports light and
  dark (`colorScheme`, line 752) and Dynamic Type (`dynamicTypeSize`, line
  798).
- The window is a two-column `NavigationSplitView` with a 184pt sidebar
  (`MainWindowView.swift:194-196`). Its default size is 940×660 and its
  minimum 760×520 (`TraceCommonsAppMain.swift:52,63`). A `MenuBarExtra`
  provides the menu-bar item (`TraceCommonsAppMain.swift:38`).
- The package targets `.macOS(.v14)` (`macos/Package.swift:23`).
- No token pipeline exists. The Tauri CSS tokens, the Swift `TC` enum and the
  GTK stylesheet (`crates/trace-commons-contributor-gtk/src/ui/style.css`)
  are three hand-maintained palettes.

#1146 is a good visual source for dark-mode geometry, the layer tiers and
the component inventory. It is not a build spec on its own:

- It is dark only: `app/app.css:12` sets `color-scheme: dark`, and the theme
  provider and selector are deleted.
- Every size is a fixed pixel value.
- It handles reduced motion (`glass.css:1232`), but not reduced
  transparency, increased contrast or pressed states.
- It has no menu-bar design, and it leaves Compute with no screen.
- Some of its components carry product behaviour that the #1146 review asked
  to change.

This spec takes the look from #1146, takes the behaviour from `main`, and
fills in what a native build needs.

## Non-goals

This spec takes the look only. Where #1146's components encode behaviour,
the macOS app keeps `main`'s behaviour. In particular:

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

## Decisions needed

1. **Appearance.** #1146 is dark only. The options are:
   - (a) dark only on macOS too, which removes light mode the app ships
     today;
   - (b) derive a light palette, as proposed in "Appearance" below;
   - (c) follow the system appearance, with a light palette designed
     separately.

   Recommendation: (b), reviewed by the brand owner.
2. **Brand.** #1146's purple (`#6D14F3`) replaces the current green
   (`#178F70`), which came from the community site. Confirm the macOS app
   moves to purple, and that the community site either follows or is
   deliberately left behind.
3. **Minimum OS.** The app targets macOS 14. Liquid Glass (`.glassEffect`)
   needs macOS 26. The options are:
   - (a) keep 14 and build two material paths;
   - (b) raise the floor to 26.

   Recommendation: (a). The fallback is specified below and is not large.
4. **Settings.** #1146 opens Settings as a modal over the window. On macOS
   the expected place is a `Settings` scene (⌘,). Recommendation: a
   `Settings` scene.
5. **Token source format.** Recommendation: a typed JSON file, described in
   "Tokens" below.

## Tokens

### One source, three outputs

Add `design/tokens/trace-commons.tokens.json`. It is the only file anyone
edits. A generator writes:

- the Tauri CSS: `tauri-desktop/frontend/src/design-system/tokens/*.css`,
  which replaces the hand-copied files;
- the Swift file: `macos/Sources/TraceCommonsApp/Views/Tokens.generated.swift`,
  which the `TC` enum reads;
- the GTK stylesheet: the colour block in `style.css`.

A CI job regenerates all three and fails on any diff. That is what keeps the
shells from drifting.

The source cannot be the CSS. Several #1146 tokens are composites:

- An edge is three insets plus a shadow (`--tc-pane-edge`, `materials.css`).
- Fills are gradients.
- Some values use `color-mix`.

Swift cannot read those as strings. The JSON gives each token a type:

| Type | Fields |
|---|---|
| `color` | `dark`, `light`, `darkHighContrast`, `lightHighContrast` (sRGB hex plus alpha) |
| `gradient` | angle, and stops of (color, position) |
| `edge` | `highlight` (color, y-offset), `sideHighlight`, `lowlight`, `shadow` (color, y, blur) |
| `blur` | radius and saturation, with per-platform overrides |
| `textStyle` | size, line height, weight, tracking, and `macosTextStyle` (the Dynamic Type style it scales with) |
| `dimension` | points |
| `duration` / `easing` | milliseconds / cubic-bezier control points |

A token can carry a platform override, for example a Liquid Glass tint on
macOS 26 in place of a gradient fill.

**Seeding.** Copy #1146's `tokens/*.css` into the JSON verbatim as the
`dark` values, so the first generated CSS diffs to zero against #1146. Light
and high-contrast values are added in the Appearance step.

### Values taken from #1146

These are exact, from `tauri-desktop/frontend/src/design-system/tokens/` at
`a15fa6addf`.

- **Brand** (`colors.css`): purple `#6D14F3`, purple soft `#8A3DFF`, purple
  text `#C9B3FF`, blue `#3A7BD5`.
- **Text** (`colors.css`): primary `#F2F2F4`, secondary `#C9C9D0`, tertiary
  `#B4B4BC`, on accent `#FFFFFF`, on status `#0C0C0E`.
- **Status** (`colors.css`), for glyphs and labels only, never fills: on
  `#3DDC84`, ask `#F5C142`, off `#A9A9B0`, outside `#FF6B6B`.
- **Data** (`colors.css`): shared `#8A3DFF`, kept `#3A7BD5`, inference
  `#A78BFA`.
- **Scene** (`colors.css`): base `#0F1219`, warm `#1D2430`, glows `#3F6A8A`,
  `#6A3F7A`, `#2F6B5A`.
- **Radii** (`materials.css`): window 22, pane 16, card 14, small card 12,
  control 8, pill 999.
- **Motion** (`materials.css`): ease `cubic-bezier(0.2, 0.8, 0.2, 1)`;
  durations 150, 220 and 300 ms.
- **Type** (`typography.css`), as weight size/line-height:

  | Style | Value | Note |
  |---|---|---|
  | eyebrow | 600 10/14 | uppercase, tracking 0.08em |
  | caption | 400 11/16 | |
  | label | 500 12/16 | |
  | body | 400 13/18 | |
  | body strong | 600 13/18 | |
  | title | 600 15/20 | |
  | heading | 600 17/22 | |
  | display | 700 22/28 | tracking -0.01em |
  | number | 700 28/34 | tabular figures |
  | mono | 400 11/16 | SF Mono |
- **Spacing** (`spacing.css`):
  - scale 2, 4, 6, 8, 10, 12, 14, 16, 20, 24;
  - padding: window 10, pane 12, card 12×14;
  - gaps: cards 10, inline 6;
  - sizes: control 26 (28 large), toggle 40×24, checkbox 15, dot 7, tab 26,
    glyph 16, tool tile 22;
  - widths: left pane 400, map 600, inspector 300; window height 760.
- **Tiers** (`materials.css`): the fills and edges for scene, window rim,
  pane, card, well, control and popover. The window blur is 30 with 180%
  saturation, the popover blur 24 with 170%, and the scrim blur 6.

**Two #1146 inconsistencies to resolve in the JSON, not copy:**

- The map-width token (600) is unused. As built, the map is the remaining
  width, 580 at 1320 wide, which matches the flow map's `viewBox`.
  Recommendation: drop the token and define the map as the remaining width.
- Six motion timings in `glass.css` and `monitor-shell.tsx` are literals
  outside the tokens. Map each one to the three durations or add a named
  token for it.

## Appearance

- **Dark.** Use #1146's values.
- **Light** (subject to Decision 1). Derive it per tier rather than
  inverting:
  - The scene becomes a pale, warm gradient.
  - Pane and card fills become white at the same relative opacities, with a
    darker lowlight edge.
  - Text tiers invert to `#1D1D1F`, `#3C3C43`, `#6E6E73`, the system label
    ramp.
  - The brand purple stays, and purple text becomes `#5A10CC` for contrast.
  - Status colours darken until they reach 4.5:1 against the light card.

  Every light value goes in the JSON, and the brand owner reviews them
  before implementation.
- **Increase Contrast.** Edges become a solid 1pt stroke at 40% text colour,
  fills gain 50% opacity, and status glyphs get their label as well as the
  colour.
- **Reduce Transparency** (`accessibilityReduceTransparency`). Every tier
  swaps blur and translucent fill for a solid fill: pane `#1C1E24` dark /
  `#F2F2F5` light, and card one step lighter or darker. This is required,
  because #1146 has no fallback.

## Materials by OS

| Tier | macOS 26 and later | macOS 14–25 |
|---|---|---|
| Scene | the desktop, through a transparent window background; #1146's painted scene only behind the first-run pane | a solid window background from the scene base colour |
| Pane | `.glassEffect(.regular, in: RoundedRectangle(cornerRadius: 16, style: .continuous))` | `.regularMaterial` in the same shape, plus the pane edge as a gradient stroke overlay |
| Card / well | a tint layer with no blur (the fill colour at its opacity) | the same |
| Control | `.buttonStyle(.glass)` inside a `GlassEffectContainer` | a flat fill from `control` plus a 1pt top highlight |
| Primary button | `.buttonStyle(.glassProminent)` tinted purple | a filled button, purple to purple soft |
| Popover / menu | the system `Menu`, `Popover` and `NSMenu`, which get glass on 26 | the system controls |

- **"No glass on glass"** carries over. A card on a pane is a tint, never a
  second `.glassEffect`.
- **Specular edges** (the multi-inset CSS edges) are drawn only before 26.
  On 26 the system draws its own rim, and adding ours doubles it.
- **Saturation** (`saturate(180%)`) has no public SwiftUI control. Omit it.

## Window and layout

- **Shape.** One window of three glass panes over the scene, with a hidden
  title bar and the real traffic lights over the left pane. The #1146
  traffic-light stand-ins are not built.
- **Size.** Default 1320×760; minimum 760×560, as in #1146.
- **Panes.** `NavigationSplitView` does not fit: its leading column cannot
  grow to fill the window when the middle column hides, and #1146 hides the
  map and the inspector independently. Instead:
  - An `HStack` of three panes, with 10pt window padding and 10pt gaps.
  - Left pane: width `min(400, max(320, 0.34 × window width))`, the same
    formula as `monitor-shell.tsx:97`. It fills the window when the map is
    hidden.
  - Map: the remaining width.
  - Inspector: 300pt, through `.inspector(isPresented:)` (macOS 14 and
    later).
- **Toolbar.** A capsule of four toggles (view menu, graph, map, inspector)
  and a round Settings button, starting 78pt from the leading edge to clear
  the traffic lights. Tabs: Home, Inference, Traces.
- **Opening state and restoration.** #1146 decides pane visibility once at
  launch (the map if the window is at least 1100 wide, the inspector if at
  least 900). The macOS app restores the user's last choice per window
  (`@SceneStorage`), and uses those widths only on first launch.
- **Behaviour that must not depend on the inspector.** In #1146, the undo
  window, the arming offer and the review exist only while the inspector is
  open. The macOS app shows the undo bar and consent offers regardless of
  which panes are visible, anchored to the bottom of the left pane when the
  inspector is closed.
- **Settings.** A `Settings` scene, subject to Decision 4. Its sections are
  the ones #1146's modal lists, plus Compute (see Screens).

## Components

These are the #1146 components (`design-system/index.ts`), with the macOS
build of each. Every one needs these states: default, hover, pressed, focus,
disabled, and selected or on/off or mixed where it applies. #1146 has no
pressed state (`:active`) and no focus style on rows and map nodes, so both
are defined here.

- **Pressed** is 8% darker fill on dark and 6% darker on light, applied with
  the fast duration.
- **Focus** is the system focus ring (`.focusable()` plus the default ring),
  not #1146's custom 2px purple ring. That keeps VoiceOver and Full Keyboard
  Access consistent.

| #1146 component | macOS build |
|---|---|
| Window, Pane | the Window and layout section |
| Popover, Menu | `Menu`, `Popover` |
| Modal, Sheet | `.sheet`; Settings is a scene |
| Scrim | the system sheet dimming |
| Card (quiet, flush, interactive), Well | custom `TCGlassCard` using the tier tokens |
| ConsentBlock | custom; text comes from the core only |
| ListRow, tree | `List` / `OutlineGroup` with a custom row (grid columns 16/24/flex/auto/38/22); one list with arrow-key navigation, not a tab stop per row |
| TableRow/Head, KeyValueList | `Grid` / `Table` |
| SegmentedTabs | `Picker(.segmented)`; badge and dot in a custom segment label |
| Breadcrumb, StepProgress | custom |
| Buttons (primary, secondary, glass, round, pill icon, icon, submit, folder, kebab) | `ButtonStyle`s over the tier tokens; see Materials by OS |
| Picker | `Picker(.menu)` |
| Toggle (plain, settings, watch) | `Toggle(.switch)` at system size, with `.tint`; #1146's 38×22 watch size is not copied |
| Checkbox (with mixed) | a wrapped `NSButton` checkbox, since SwiftUI has no tri-state; its accessible name is its full sentence, never "Option" |
| Expander | `DisclosureGroup` |
| TextField | `TextField` with a custom style |
| Dot, Status, Chip, Tag, Badge | custom; the colour is always paired with a label |
| ToolTile, logos | vector assets converted from `logo-paths.ts` |
| BarGraph | Swift Charts |
| MapNode, MapArc, NodeCard (flow map) | `Canvas` plus `TimelineView` for the moving dashes; each node is an accessibility element with a label and actions, not a picture |

## Motion

- One curve, `Animation.timingCurve(0.2, 0.8, 0.2, 1, duration:)`, at 0.15,
  0.22 and 0.30 seconds.
- #1146's six literal timings are mapped to these, per the Tokens section.
- Reduce Motion (`accessibilityReduceMotion`) removes transitions and the
  flow-map dash animation, as `glass.css:1232` does.

## Menu-bar popover

#1146 has no design for the menu bar: it has only a doc comment, "Popover
tier: menu-bar panel, floating menus." (`components/surfaces.tsx:44`). `main`
has a `MenuBarExtra`. This spec defines its look,
not its content:

- a `MenuBarExtra(.window)` panel at the popover tier: blur 24, popover fill
  and edge, radius 14;
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
| Settings | the Settings scene |
| Menu bar | the menu-bar popover |

## Icons

The toolbar glyphs in #1146 are inline SVG (`monitor-toolbar.tsx`). Map
each to an SF Symbol, recording the table in the JSON's `icons` group so
Tauri and GTK can map the same names:

| Glyph | SF Symbol |
|---|---|
| view menu | `line.3.horizontal.decrease.circle` |
| graph | `chart.bar` |
| map | `point.3.connected.trianglepath.dotted` |
| inspector | `sidebar.right` |
| settings | `gearshape` |
| binoculars | `binoculars` |

The tool logos are third-party marks. They are shipped as vector assets only
after a trademark-use check.

## Implementation order

Each step is a PR of its own.

1. **Token source and generators.** The JSON, three generators and the CI
   drift job. The Tauri output must diff to zero against #1146's
   `tokens/*.css`.
2. **Swift token layer.** `Tokens.generated.swift`. `TC` reads its colours,
   radii and type from it; the old green palette is removed behind a single
   switch.
3. **Materials and components.** The tier modifiers for both OS paths, and
   the component catalogue, with a SwiftUI preview gallery equivalent to
   #1146's Storybook page.
4. **Window shell.** Three panes, the toolbar, tabs, restoration, and the
   Settings scene.
5. **Screens.** Move screens one at a time, keeping `main`'s behaviour and
   the core's copy.
6. **Menu-bar popover.**
7. **Light and high-contrast values.** Reviewed by the brand owner.

## Acceptance

- The generated Tauri CSS equals #1146's token CSS, byte for byte, on the
  first run.
- At 1320×760 in dark mode on macOS 26, screenshots match #1146's monitor
  window in layout, radii and colours. They are not expected to match the
  painted scene, because real glass shows the desktop.
- On macOS 14, the same screens render with the material fallback, with no
  missing panes or controls.
- VoiceOver reaches and names every control, including flow-map nodes and
  tree rows. Full Keyboard Access can operate the tree with arrow keys.
- Reduce Transparency, Increase Contrast and Reduce Motion each change what
  this spec says they change.
- No screen from the current app is unreachable (see Screens).
- The existing macOS test suite and the Swift CI job stay green. No consent,
  withdrawal or Private AI behaviour changes.

## Open

- Whether the community site adopts the new brand (see Decision 2).
- Where Insights lives long term. This spec only requires that it stays
  reachable.
- The Liquid Glass tint values per tier on macOS 26. They are tuned on
  device and recorded in the JSON as platform overrides.
