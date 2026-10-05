# Glass for the Native macOS App — Design

Date: 2026-09-30. Revised 2026-10-02 to match Ron's decisions on #1173,
again on 2026-10-02 to make Ron's design the source of truth, and on
2026-10-05 to record the merged code and Kristi's (poldsam's) reviews.
Status: draft for review. Ron's (rdisandro's) stack #1178–#1184, #1190 and
#1195 (#1173's C2 and R1–R7, plus R8–R13 work) is merged to `main` as
`ff7ffa99f`. This revision checks the code at `main` `efd05c6f8`.

**Precedence (Zaki, 2026-10-02, narrowed 2026-10-05).** Ron's code and his
decisions on #1173 bind the visual and component choices: materials,
components, geometry, tokens and their values. Where this spec and Ron's
code disagree on one of those, the spec is wrong and follows the code.
The following bind the code instead:

- consent copy and notices (the words come from the core, and every notice
  the shipping app shows has a place);
- behaviour kept from `main` (see "Behaviour changes (decided)" for the
  named exceptions);
- accessibility duties;
- the contrast floors.

Where the code lacks one of these, the gap is a defect in the code to fix,
not a reason to change the spec. The current gaps are listed under "Open
code gaps".

- Decided: purple brand (D3), the minimum OS and glass on every supported
  macOS (D4), the Settings window (D8), the three-pane layout (D9), SF Pro
  and SF Mono, the token source and generator, Ron's custom-painted menus,
  popovers and toggles, eyebrow weight 600, the Reduce Transparency base
  `paneOpaque`, a permanent Watching/Paused readout with pause and resume on
  every tab, and the folder Submit all and Submit all as. See "Decisions".
- Appearance: the merged token source follows the system appearance ("D2
  revised"); see Decision 1, which is open on where that was decided.
- Open: the items marked open in "Decisions", "Tokens" and "Open".

Visual source: #1146, "Adopt the WYSIWYG UX and Glass design system" (open,
frozen as the design reference per #1173). Line references into #1146 are at
its head `a1a15fbe` (2026-10-05). Token values are quoted from
`design-tokens/glass.tokens.json` on `main`, not from #1146 (see Tokens).
The 2026-09-30 screen recording is indexed below.
Decided context: the native macOS app is kept and built to match the WYSIWYG
design (2026-09-28, in the #1118 review), and native SwiftUI is the main
client on macOS with no new Tauri work (#1173 D1). Native macOS is the lead
client (2026-10-02).
Scope: `macos/` (the `TraceCommonsApp` target and the `TCDesign` target),
the token source `design-tokens/glass.tokens.json`, and its Swift generator.
No Tauri or GTK output. The product behaviour changes the glass shell makes
are named and decided in "Behaviour changes (decided)"; there are no consent,
withdrawal or Private AI behaviour changes.

## Changes since 2026-09-30

### Third revision (2026-10-05): the merged code and Kristi's reviews

Kristi reviewed this spec on 2026-09-30 (twice), 2026-10-01 and 2026-10-02.
Ron's stack merged on 2026-10-02. This revision answers each of her points,
applies Zaki's decisions of 2026-10-05, and records what the merged code
already does.

- **Precedence narrowed** (her 10-02 item 1, Zaki's decision): see above.
- **Stack record** (10-02 item 2): the open PR heads are replaced by the
  merge commit. The watch-knob contrast item is closed: the knob is
  `textOnStatus` `#0C0C0E` on `watchOn`, about 10.9:1
  (`TCDesign/Components/Controls.swift:402`, `SwitchContrastTests`).
- **Appearance:** the merged JSON carries light values and follows the
  system ("D2 revised", `glass.tokens.json:2`). Under the precedence rule
  the spec records it, and the light values are held to the same contrast
  floors. Where D2 was revised is open with Ron. Kristi's light-tertiary
  finding (`#858585`) is moot: that palette is gone, and the light
  `textTertiary` is `#5C5C63`.
- **One token revision** (10-02 item 4): the JSON on `main` is the source of
  every value quoted here. The separate "checked at `a15fa6addf`" claim is
  dropped.
- **Code contradictions** (10-02 item 3, 09-30 items 4 and 14): the tree has
  two controls, not one. Tool rows have a two-state source switch, gated on
  the core's source-settings explanation, and folder rows have the
  three-way `GlassPicker`. Tabs are `GlassSegmentedTabs`, not
  `Picker(.segmented)`. The shipping menu-bar item is menu-style, and the
  glass panel is `MenuBarExtra(.window)` in debug builds only.
- **View menu, graph toggle and chart footer** (10-02 item 3, 10-01 item 3):
  not in the code. Zaki's decision: they are open items for Ron, not
  requirements and not dropped. The map glyph is `map`, as the code has it.
- **Notices** (09-30 item 1): a "Notices" subsection records where
  `ShellNotices` sits and adds an Acceptance check.
- **Watching/Paused and pause/resume** (09-30 item 2, Zaki's decision): a
  permanent readout with pause and resume on every tab is required. The
  monitor window does not have it yet; see "Open code gaps".
- **Behaviour changes named** (09-30 item 3, 10-01 item 4): "No product
  behaviour changes" is replaced by "Behaviour changes (decided)".
- **Traces badge** (10-01 item 1): the code's rule is copied into the
  Toolbar paragraph.
- **"Unknown has no dot"** (10-02 item 5) is now a general rule for every
  status readout.
- **Folder Submit** (09-30 item 9, Zaki's decision): Submit all and Submit
  all as are kept, on the folder row and in the folder inspector, with
  Submit all as presented as a modal.
- **Recording** (10-01 item 2): the filename, local path and hash are
  removed. The table is the reference.
- **Tokens and #1030** (09-30 item 5), **contrast on live glass** (09-30
  item 6), **token types** (09-30 item 11), **motion literals** (09-30
  item 12), **accessibility** (09-30 item 13), **the 300pt inspector**
  (09-30 item 8), **the menu-bar popover** (09-30 item 10), **screens**
  (09-30 item 7), **CI for the fallback** (09-30 item 15), **data binding**
  (09-30 item 16), **the undo region** (10-01 item 5), **the component
  table** (10-01 item 6) and **Compute in Settings** (10-02 item 6, already
  in the code): each is updated in its section.

### Second revision (2026-10-02): Ron's design is the source of truth

Checked then against the open PR heads #1178 `e6e05ac2` through #1184
`2c701c2a`. Those PRs, and #1190 and #1195, have since merged as
`ff7ffa99f`.

- **Precedence** stated: the spec follows Ron's code and his #1173
  decisions (narrowed on 2026-10-05; see above).
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
revision adopted the decisions and kept the requirements the reviews found
missing.

- **Dark only** (#1173 D2), since revised in the merged code; see Decision 1.
- **SF Pro and SF Mono.** Ron's font decision on #1173 replaces Schibsted
  Grotesk, Mona Sans and JetBrains Mono. Nothing is bundled and no font
  licence is carried, so no font registration step is needed. The
  "Alignment with near.ai" section is reduced to the copy and
  third-party-mark rules, which the decision did not touch.
- **Token source and generator.** The source is
  `design-tokens/glass.tokens.json` (not `design/tokens/trace-commons.tokens.json`).
  `scripts/design-tokens/generate.py` writes
  `macos/Sources/TCDesign/Generated/GlassTokens.swift`; `TokenDriftTests` and
  `generate.py --check` guard drift. There is no separate drift CI job: the
  Swift suite, which CI already runs, is the guard (#1178).
- **No Tauri or GTK generators.** #1146 is frozen (#1173 D1 and Ron's
  2026-10-02 note); Windows and Linux stay on Tauri, frozen, until macOS ships
  (D13, a default). The acceptance item that diffed generated Tauri CSS
  against #1146 is dropped. The typefaces therefore change nothing in Tauri
  or GTK.
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
  window radius 22, the ease curve, scene glows, the window rim, own edges
  only before 26, pressed as a darker fill, the Reduce Transparency base,
  Increase Contrast, Reduce Motion, arrow-key lists, no hard-coded wording,
  traffic-light clearance of at least 78pt, minimum window 760×560, the
  exercised pre-26 fallback, and the accent contrast floor (#1178's blocking
  finding). Window radius, scene glows and the window rim are visual choices,
  so under the narrowed precedence they are now open with Ron (see Tokens).

## Problem

This section records the motivation as of 2026-09-30, before Ron's stack
merged. `main` now has the `TCDesign` target, the token pipeline, the purple
palette and the debug-only glass windows.

#1146 moves the Tauri app onto a glass design system: layer tiers, a purple
brand, status colours, a three-pane "monitor" window. The native macOS app
should look the same. On `main` at 2026-09-30 it shared almost nothing with
that system:

- `macos/Sources/TraceCommonsApp/Views/DesignSystem.swift` defined a `TC`
  enum with a green-on-warm-grey palette (`ground` `#F6F7F4`/`#23251D`,
  `green` `#178F70`/`#3FBE9A`). It had smaller radii (`TC.Radius`), no
  materials and no glass. It supported both system appearances and Dynamic
  Type. D3 (purple) supersedes the green "good standing" rationale that
  `TraceCommonsAppMain.swift` gave for the accent; that comment now gives
  the purple accent's reasons.
- The window was a two-column `NavigationSplitView` with a 184pt sidebar.
  Its default size was 940×660 and its minimum 760×520. A `MenuBarExtra`
  provided the menu-bar item.
- The package targets `.macOS(.v14)` (`macos/Package.swift`).
- No token pipeline existed. The palettes were hand-maintained. This spec
  fixes that for the macOS app only; the Tauri and GTK palettes are frozen
  with their shells (D1, D13).

#1146 is a good visual source for the geometry, the layer tiers and the
component inventory. It is not a build spec on its own:

- It is dark only (`app/app.css` sets `color-scheme: dark`).
- Every size is a fixed pixel value.
- It handles reduced motion (`glass.css:1409` at `a1a15fbe`), but not
  reduced transparency, increased contrast or pressed states.
- It has no menu-bar design, and it leaves Compute with no screen.
- Some of its components carry product behaviour that the #1146 review asked
  to change.

This spec takes the look from #1146 and Ron's code, takes the behaviour from
`main`, and fills in what a native build needs.

## Screen recording reference (2026-09-30)

A 2026-09-30 screen recording of the #1146 glass UI, 2:49 long, was reviewed
for this spec. It is not committed and contains real project names, so it
is not a reference anyone else can check. The table below is self-contained:
each row states the observed state and the requirement taken from it.
Committed screenshots and fixtures use synthetic data, such as the debug
monitor's sample sets (`TRACE_COMMONS_SAMPLE`).

This is a visual and interaction reference, not evidence that the native
app implements these screens. `glass.tokens.json` is the source for exact
token values; do not sample colours or infer point sizes from the scaled
video. The native adaptations and behaviour exceptions in this spec take
precedence over the recording.

| Time | Observed reference | Requirement carried into this spec |
|---|---|---|
| 00:00–00:20 | Centred onboarding pane, horizontal step progress, grouped choices, then a completion card | Keep progress and action hierarchy consistent across steps; retain the core's consent choices, disclosures and completion state. The clip begins partway through onboarding. |
| 00:25–00:38 | Home cards beside a large flow map and a Summary inspector; the map also has a Private AI mode | Keep the left tab selection separate from the map's Traces / Private AI selector. Preserve the summary's hierarchy of counts, decisions, statistics and runtime cards. |
| 00:45 | Map hidden; the main pane expands beside the inspector while the window is resized | Define the two-pane layout explicitly, with no empty map-width reservation. |
| 00:55 | Missions inside Home, with Back and a breadcrumb above stacked cards | Keep sub-navigation inside the main pane; retain the inspector alongside it. |
| 01:08 | Inference content scrolls while its Private AI inspector remains visible | Scroll each pane independently; use the core's status and credential copy. |
| 01:15–01:44 | Hierarchical trace rows, a stationary chart footer, selected-project details and a separate approval/Undo card | Selection, scrolling and approval feedback have separate state; do not turn row selection into submission. |
| 02:00 | Map and inspector hidden; the trace tree and chart span the main pane | Define the one-pane layout and preserve a way to reopen either pane. |
| 02:05–02:15 | Chart date hover and range change (11 to 42 days); checked "Show ignored folders" menu item | Open with Ron: the chart footer and the view menu (see Open). If built, keep the view filter separate from watch and consent settings. |
| 02:26–02:35 | Settings with a persistent section list and a scrolling detail area | Carry the navigation and grouped content into the native Settings window, not the recorded modal presentation. |
| 02:46 | Return to the trace tree with the inspector and the wider chart range | Preserve presentation state when opening and closing Settings. |

The brand and typeface questions the recording left open are settled by
#1173 (purple, SF Pro/SF Mono). The recording does not settle accessibility
fallbacks. Its painted blue/green/purple scene illustrates depth and
hierarchy; it does not replace the desktop-backed material decision for
macOS 26.

## Non-goals

This spec takes the look only. Where #1146's components encode behaviour,
the macOS app keeps `main`'s behaviour (agreed by Ron on #1173). In
particular:

- **Folder Submit all and Submit all as** (Zaki, 2026-10-05). #1146
  bulk-approves with one click and drops the core's withheld line. The macOS
  app keeps `main`'s folder actions, as the #1241 port does:
  - **Submit all** (`approve` with the folder's `project_id`) on the folder
    row and in the folder inspector, with the folder's `withheld_line` shown beside it whenever
    the core supplies one;
  - **Submit all as** on the folder row and in the folder inspector,
    presented as a modal (a sheet) that offers the `ContributorVerdict`
    choices with the core's words and sends the chosen verdict for every
    entry the folder covers;
  - both drawn only when the shared table offers Contribute for the
    daemon's counts, and removed rather than disabled otherwise.
- **The tree's two controls.** #1146 gives every row the same two-state
  switch and collapses three project modes into two. The macOS app has two
  different controls:
  - **Tool rows:** a two-state watch/off source switch
    (`GlassToggleStyle(.watch)`, 38×22). It writes the source declaration
    only after the core's `SourceSettingsCopy` explanation is shown and
    confirmed; with no copy, nothing is written. An unset or unknown source
    draws no switch, never an off one.
  - **Folder rows:** `main`'s three modes (auto upload, ask, ignore) through
    a three-way `GlassPicker`, with the core's arming and ignore
    confirmations. A mode that needs the core's words is not set without
    them.
- **Private AI status.** #1146 hand-types "answer on NEAR AI" and "answer at
  their vendors". The macOS app renders the core's copy
  (`private_inference_copy`), never retyped strings. It takes the dot and the
  legend components, not the sentences.
- **Compute.** It stays reachable, with its pause, resume and withdraw
  controls, as a Settings section.
- **Consent copy.** It always comes from the core. This spec adds no new
  sentence that promises privacy or security. The #1146 shield-and-check
  icon on the credential node is not carried over.
- **Tauri and GTK.** No generated output for either shell (D1, D13). #1146's
  Tauri glass bridge (`set_glass_regions`) is not carried forward.

## Behaviour changes (decided)

The glass shell changes these behaviours. Each is decided; nothing else
changes. There are no consent, withdrawal or Private AI behaviour changes.

| Change | From `main`'s shipping window | To | Decision |
|---|---|---|---|
| Settings | a sidebar destination (⌘7) | a macOS Settings window (⌘,) | #1173 D8 |
| Compute | a sidebar destination | a Settings section, keeping pause, resume and withdraw | this spec (#1146 has no screen for it); built in `MonitorSettingsWindow` |
| Navigation | a two-column sidebar of seven destinations | three tabs (Home, Inference, Traces) in a three-pane window | #1173 D9 |
| Launch tab | Insights, with discovery and enrollment deferred while it shows | Home, the `@SceneStorage` default (built, R5). The debug monitor window calls `navigation.activateServicesForWindow()` on appear. Whether the Insights deferral is dropped when the monitor becomes the main window is not decided: it is plan D-11, an owner decision whose default is to start services when the monitor opens | Home: R5. The deferral: open (plan D-11) |
| Pane collapse | none | the map hides below 1100pt at runtime and returns when the window widens; the saved preference is kept. #1146 applies its 1100pt rule only once, at launch, so this is a native divergence | #1182 |
| Map modes | none | the map shows Traces or Private AI; the selector changes the map only | D11, R8 |
| A session's pill | a session card's Submit | Review: selects the session and opens the inspector | #1173 D10 |
| Menu-bar item | menu-style `MenuBarExtra` with nested pause-duration submenus | at R15, the glass panel in `MenuBarExtra(.window)`, with three state pills, sub-lists and a global contribution override behind the core's confirmation. Until R15 the shipping item stays menu-style and the panel is debug-only (`TRACE_COMMONS_GLASS_MENU=1`). Keyboard and VoiceOver change from menu to window semantics; see Menu-bar popover | R13 |

Open with Ron, and behaviour additions if built: the chart footer and its
ranges, the "Show ignored folders" filter, and the graph toggle (see Open).

## Copy and third-party marks

The 2026-09-30 draft aligned the app with near.ai's brand (typefaces, light
neutrals). Ron's font decision supersedes that; the near.ai faces and
neutrals are gone. Two rules from that section were not touched by #1173 and
still apply:

- **Voice.** Direct, and centred on proof rather than promises. Our copy rules
  still exclude any sentence that promises privacy, and consent copy still
  comes from the core.
- **Logo.** near.ai publishes no rules for third-party use. The app shows no
  near.ai logo or lockup, for example on the Private AI screen, without
  near.ai's written permission. The name "NEAR AI" appears in text only
  where the core's copy uses it.

## Decisions

From #1173's table, Ron's 2026-10-02 comment on it, and Zaki's decisions of
2026-10-05.

1. **Appearance: follows the system, as merged; where it was decided is
   open.** #1173 D2 and Ron's 2026-10-02 comment on this PR said dark only.
   The merged JSON instead carries a light value for every colour that
   differs and follows the system appearance ("D2 revised",
   `glass.tokens.json:2`; `LightAppearanceTests`). Appearance is a visual
   choice, so this spec follows the code. The light values meet the same
   contrast floors as the dark ones (`LightContrastGroundsTests`). Ron to
   confirm where D2 was revised (also plan D-2 in
   `docs/superpowers/plans/2026-10-02-macos-glass-rebuild-and-r15-cutover.md`).
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
8. **Map tab label: decided, "Private AI" (D11)**, as the core supplies it.
9. **Menus, popovers and toggles: decided, Ron's custom-painted
   `GlassMenu`, `GlassPopover` and `GlassToggleStyle`** (#1178), carrying the
   accessibility obligations listed in Components.
10. **Eyebrow weight: decided, 600** (#1179).
11. **Reduce Transparency base: decided, `paneOpaque` `#1C1E24`** (#1180).
12. **Precedence: decided, narrowed** (Zaki, 2026-10-05). See the top of
    this spec.
13. **Watching/Paused: decided, a permanent readout with pause and resume
    on every tab** (Zaki, 2026-10-05). `main`'s rule holds: paused is a
    state a person can forget they chose, so it is never left implicit.
    Home's Watching row and the menu-bar popover are not enough on their
    own, because Home is one tab of three. See Toolbar.
14. **Folder Submit all and Submit all as: decided, kept** (Zaki,
    2026-10-05), on the row and in the folder inspector, with Submit all as
    as a modal. See Non-goals.
15. **View menu, graph toggle and chart footer: open items for Ron** (Zaki,
    2026-10-05). They are neither required nor dropped. See Open.

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

The JSON was seeded from #1146's `tokens/*.css` at `09a26c0d`, the revision
the JSON names. The JSON on `main` is now the source, and every value below
is quoted from it at `efd05c6f8`.

**First-run styling.** The JSON is upstream of any first-run styling. #1030's
`--ftux-*` palette (`features/ftux/ftux.css`) is Tauri-only and frozen (D1),
and it is not a token source. The native first run (#1235, porting #1030)
and the passkey screens (#1120) read `GlassTokens` only. So #1030's
text-3 `#A9A9B0` is superseded by `textTertiary` `#B4B4BC`, which the JSON
raised for 4.5:1, and #1030's extra blur values are not carried.

Composite CSS tokens (multi-inset edges, gradients, `color-mix`) are
expressed as typed groups, because Swift cannot read them as strings:

| Group | Fields |
|---|---|
| `color` | `hex`, optional `alpha`, optional `note`, optional `light` (`hex`, `alpha`) |
| `gradient` | `angle`, and `stops` of (`hex`, `alpha`, `at`, optional `light`) |
| `shadow` | an array of layers: `x`, `y`, `blur`, `hex`, `alpha`, `inset` |
| `radius`, `space`, `size` | points |
| `opacity` | 0–1 |
| `type` | `textStyle`, `size`, `weight`, `lineHeight`, optional `tracking`, `uppercase`, `tabular`, `design` |
| `motion` | durations in seconds, and the ease curve's control points |

Three #1146 forms have no type of their own:

- **Radial fields** (for example `--tc-map-fill`) are two colour tokens, with
  the geometry in code. The map field is `mapFieldInner` and `mapFieldOuter`
  in a `RadialGradient` whose radii are set in `MonitorWindowView.swift`.
- **Aliases** (for example `--tc-watch-on: var(--tc-status-on)`) are
  resolved into duplicated values in the JSON: `watchOn` repeats
  `statusOn`'s dark hex. `TokenDriftTests` guards each copy.
- **Multi-layer CTA edges** in #1146's `components.css` are `shadow` groups
  (`ctaEdge`, `ctaSecondaryEdge`).

**Tokens #1146 has that the JSON does not carry.** These are visual choices,
so under the precedence rule they are Ron's. Each is open: Ron either adds
it (a token and a generated constant) or records it as dropped.

- **Window radius:** 22 in #1146 (the JSON's `radius` group has no window
  step).
- **Scene glows:** `#3F6A8A`, `#6A3F7A`, `#2F6B5A` (the JSON has only
  `sceneBase` and `sceneWarm`).
- **Window rim:** #1146's window-tier edge, as a `shadow` entry.

**Increase Contrast: decided, as built.** The affected tokens have
`*HighContrast` partners (`textSecondaryHighContrast`,
`textTertiaryHighContrast`, `hairlineHighContrast`, `edgeHighContrast`),
generated and drift-checked like every other token. See Appearance.

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

### Values

Quoted from the JSON at `efd05c6f8`. Dark values first; light values are in
the JSON beside each.

- **Brand:** purple `#6D14F3`, purple soft `#8A3DFF`, purple text `#C9B3FF`,
  blue `#3A7BD5`.
- **Text:** primary `#F2F2F4`, secondary `#C9C9D0`, tertiary `#B4B4BC`, on
  accent `#FFFFFF`, on status `#0C0C0E`.
- **Status**, for glyphs and labels only, never fills: on `#3DDC84`, ask
  `#F5C142`, off `#A9A9B0`, outside `#FF6B6B`. The one exception is Ron's
  `watchOn` (`#3DDC84`), the watch switch's on fill, which carries a
  `textOnStatus` knob.
- **Data:** shared `#8A3DFF`, kept `#3A7BD5`, inference `#A78BFA`.
- **Scene:** base `#0F1219`, warm `#1D2430`. Glows are open (above).
- **Map field:** inner `#1E2A3A`, outer `#131A24`.
- **Radii:** pane 16, card 14, small card 12, control 8, tile 6, checkbox 5,
  pill 999, menu panel 18, menu state pill 22. Window 22 is open (above).
- **Motion:** ease `cubic-bezier(0.2, 0.8, 0.2, 1)` (`easeX1`…`easeY2`);
  durations `fast` 150, `standard` 220 and `slow` 300 ms, plus `reveal` 180
  and `slide` 250 ms.
- **Spacing:**
  - scale 2, 4, 6, 8, 10, 12, 14, 16, 20, 24;
  - padding: window 10, pane 12, card 12×14;
  - gaps: panes 10, cards 10, inline 6;
  - sizes: control 26 (28 large), checkbox 15, dot 7, tab 26, glyph 16, tool
    tile 22, list row 40;
  - widths: left pane 400 (maximum; see Window and layout), inspector 300;
    window 1320×760, minimum 760×560.
- **Tiers:** the fills and edges for pane, card, well, control, popover,
  menu, node card, map and consent block. #1146's blur radii are not carried
  over; see "No blur tokens" above.

### Accent contrast floor

`#C9B3FF` (`purpleText`) is a text colour only. Every accent fill and tint,
including the window-level `.tint`, `.borderedProminent`, checkbox and
check-circle fills, uses `#6D14F3` (`primaryFill`) or the gradients built
from it. A glyph or label drawn on an accent fill reaches at least 3:1
against it; white on `#6D14F3` is about 6.9:1, white on `#C9B3FF` about
1.85:1. This is the #1178 blocking regression and applies to the shipping
`TC` palette as well as `TCDesign`, in both appearances.

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
- **Fixed geometry and Dynamic Type.** The pane widths (400/320, 300), the
  row grid and the 26pt controls do not scale with text size. Text inside
  them does, so rows and controls grow in height, not width, and the
  Acceptance large-text check covers clipping and overlap.

### Tokens Ron added: adopted or open

- **`paneBase` `#161A22` at 0.96: adopted** as the neutral dark under a
  pane's glass where there is no native material. Not adopted as the Reduce
  Transparency base; see Appearance.
- **`glassVeil` `#0C0E14` at 0.28: adopted.** A thin dark veil over native
  glass that keeps text at contrast, as #1146's native-glass CSS does. Its
  measured floor is an Acceptance item ("Contrast on live-desktop glass").
- **`windowControlsInset` 30 and `windowControlsWidth` 78: satisfied.**
  #1182 raised the width from 76 to 78, the traffic-light clearance in
  Window and layout.
- **`toggleWidth`/`toggleHeight` 40×24 and
  `watchSwitchWidth`/`watchSwitchHeight` 38×22: adopted.** They size Ron's
  `GlassToggleStyle` (see Components), with `toggleOn` `#3A7BD5`,
  `toggleOnSettings` `#8A3DFF`, `watchOn` `#3DDC84` and `toggleOff` white at
  0.18.
- **`mapWidth` 600: open, still in the JSON.** As built the map is the
  remaining width (580 at 1320 wide, matching the flow map's `viewBox`). The
  recommendation stands: drop the token and define the map as the remaining
  width.
- **Tool tints (`toolClaude` and so on) and `tileFolder`: adopted.**
  Antigravity and Gemini CLI share `toolAntigravity`; whether Gemini CLI gets
  its own tint is open.

**#1146's literal motion timings.** At `a1a15fbe`, eleven timings in its
`.css` and `.tsx` files sit outside its tokens:

| Where | Literal | macOS mapping |
|---|---|---|
| `glass.css:992` | `transform 50ms linear` | `fast` |
| `glass.css:1188` | `tc-spin 0.8s linear infinite` | the system `ProgressView` |
| `glass.css:1193` | `tc-pulse 1.6s` (on the ease curve) | `slow` per step, repeating; removed under Reduce Motion |
| `glass.css:1236` | `tc-rise-in 0.18s ease` | `reveal` (0.18s) on the single curve |
| `glass.css:1273` | `background 0.1s` | `fast` |
| `glass.css:1320` | `stroke 0.55s ease-out, stroke-width 0.55s ease-out` | `slow` on the single curve |
| `glass.css:1332` | `tc-flow 0.6s linear infinite` | a named `flowDash` token, 0.6s linear, the only linear loop; removed under Reduce Motion |
| `monitor-shell.tsx:312` | `height .25s ease` | `slide` (0.25s) on the single curve |
| `flow-map.tsx:179` | `transform .7s cubic-bezier(.4,0,.2,1)` | `slow` on the single curve |
| `flow-map.tsx:332` | `opacity .3s` | `slow` |
| `indicators.tsx:203` | `tc-slide-* .45s cubic-bezier(.4,0,.2,1)` | `slide` on the single curve |

Everything except the flow-map dash uses the one curve. `flowDash` is open
until it is added to the JSON.

## Appearance

- **System appearance**, as merged (Decision 1). Every colour token has a
  dark value and, where it differs, a light one; the light values meet the
  same floors (4.5:1 text, 3:1 glyphs and status dots) on every light ground
  they are drawn on (`LightContrastGroundsTests`). `TRACE_COMMONS_APPEARANCE`
  pins one appearance for screenshots in development only.
- **Increase Contrast** (`colorSchemeContrast == .increased`), as built:
  - `textSecondary`, `textTertiary`, `hairline` and painted edges swap to
    their `*HighContrast` tokens;
  - a painted surface's soft edge becomes a solid 1pt stroke
    (`edgeHighContrast`; `GlassStyle.swift`). Liquid Glass surfaces take the
    system's own contrasting border instead;
  - status glyphs carry their label as well as their colour.
- **Reduce Transparency** (`accessibilityReduceTransparency`). Every tier
  swaps blur and translucent fill for an opaque fill: the pane is the
  `paneOpaque` token, solid (#1180), and a card is one step lighter. This is
  required, because #1146 has no fallback. Floating surfaces under Reduce
  Transparency get their painted tier alone, with no blur (#1181).
- **Unknown is neither on nor off.** Any status readout whose state the core
  did not report draws no dot and reads as unknown, in the core's word
  (`MonitorScreensCopy.unknown`). It is never drawn as on or as off. This
  covers the Inference dot, Home's Watching row, the Watching/Paused
  readout, the Traces badge and the menu-bar pills
  (`HomeViews.swift`, `MenuBarGlassPanel.swift`).

## Materials by OS

| Tier | macOS 26 and later | macOS 14–25 |
|---|---|---|
| Scene | the desktop, through a transparent window background; #1146's painted scene only behind the first-run pane | the system material behind the window (below) |
| Pane | Liquid Glass (`NSGlassEffectView` or `.glassEffect`) in a 16pt continuous rounded rectangle, with `glassVeil` and the pane sheen | the system material in the same shape, plus the pane edge as a gradient stroke overlay |
| Map pane | opaque content, not glass: the map field's radial gradient (`GlassPane(isContent: true)`) | the same |
| Card / well / control inside a pane | a painted tint with no blur (the fill colour at its opacity) | the same |
| Control or card floating over the map | Liquid Glass, through the single call site | the painted tier over a within-window HUD blur (`GlassFloatingBlur`) |
| Primary button | `.buttonStyle(.glassProminent)` tinted `#6D14F3`, or a painted fill of the CTA gradient | a filled button, the CTA gradient |
| Popover / menu (`GlassPopover`, `GlassMenu`) | Liquid Glass through `glassSurface(_, floating: true)`, tinted with `glassVeil` | the painted popover or menu tier over a within-window HUD blur (`GlassFloatingBlur`) |

- **Material before 26: HUD vibrancy, adopted subject to the macOS 14
  check.** #1180 uses `NSVisualEffectView` with the `.hudWindow` material
  behind the window, the same choice #1146's `tc_glass_view_make` makes for
  Tauri. D4 says "the system material", which this satisfies. The
  2026-09-30 spec's `.regularMaterial` is no longer required. The pane edge
  stroke is still required before 26.
- **One call site: adopted.** #1181's `glassSurface` is the one place a
  surface becomes Liquid Glass. It holds the `#available(macOS 26, *)` check,
  honours Reduce Transparency, and a source-scan test fails if anything else
  calls `.glassEffect`. That test satisfies the Acceptance item on
  glass-only APIs. Screens never branch on the OS themselves.
- **Painted controls inside panes: adopted.** #1180 keeps cards, wells and
  controls inside a pane painted, which is the "no glass on glass" rule. The
  2026-09-30 idea of `.buttonStyle(.glass)` inside a `GlassEffectContainer`
  for every control is dropped.
- **The map is content, not glass: adopted.** The map pane is opaque, so the
  selector, zoom and node cards floating on it are its only glass. This
  settles the earlier "glass controls over a glass map" question, and
  matches #1146, where the map is a dark field rather than glass.
- **Specular edges** (the multi-inset CSS edges, `glassEdge`) are not drawn
  on a Liquid Glass surface: on 26 the system draws its own rim, and adding
  ours doubles it. Painted tiers (cards, wells and controls inside a pane)
  keep their edges on every OS, and every tier draws its edge before 26.
  Fixed in #1180 (`drawsOwnEdge`).
- **Floating blur before 26.** Popovers, menus, node cards and controls that
  float are not a bare translucent fill: they are the painted tier over a
  within-window HUD blur of what they float on (`GlassFloatingBlur`, #1181).
  Under Reduce Transparency they are the painted tier alone.
- **Contrast on live-desktop glass.** On 26, `.glassEffect(.regular)`
  adapts to the wallpaper and can render light under dark-appearance text.
  The floors are measured on device, not assumed from the token values (see
  Acceptance). If a surface fails, raise the `glassVeil` alpha or pin the
  glass appearance; never lower the floor. Transparent gaps between panes
  pass clicks to the desktop and do not drag the window; that is the
  system's behaviour for a transparent window and is accepted.
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
  window corner radius is open with Ron (see Tokens).
- **Size.** Default 1320×760; minimum 760×560, as in #1146. The declared
  minimum and the content minimum agree: at 760 wide the panes fit without
  clipping, because the map hides below 1100pt and the leading pane shrinks
  to 320 (#1182 review found a content minimum of about 1080 against a
  declared 600).
- **Panes.** `NavigationSplitView` does not fit (D9): its leading column
  cannot grow to fill the window when the middle column hides, and the map
  and inspector hide independently. Instead (`GlassThreePane`):
  - An `HStack` of three panes, with 10pt window padding and 10pt gaps.
  - Left pane: width `min(400, max(320, 0.34 × window width))`, the same
    formula as #1146's `monitor-shell.tsx`. It fills the window when the map
    is hidden, less the inspector width and gap when the inspector is open.
  - Map: the remaining width.
  - Inspector: 300pt, composed directly in the `HStack`.
  - **What hides.** The map and the inspector, each independently. The left
    pane, which holds the tabs, never hides; hiding it removes all
    navigation (#1182 review).
  - Supported compositions: main + map + inspector, main + map, main +
    inspector, and main alone. Hidden panes reserve no space. The map is
    hidden at runtime below 1100pt window width, and the person's
    preference is kept separately so widening restores it; an explicit hide
    stays hidden. This is a native divergence from #1146, which applies the
    1100pt rule only once, at launch (see "Behaviour changes (decided)").
- **Toolbar.** On the main pane's top row, trailing-aligned: a capsule with
  the Map (`map`) and Inspector (`sidebar.right`) toggles, then a separate
  round Settings button (`gearshape`). Reserve at least 78pt at the leading
  edge for the real traffic lights; this is a clearance, not a fixed origin
  for the capsule. While the window is too narrow for the map, the Map
  toggle shows it hidden and is disabled. Every icon has a tooltip and an
  accessible name; toggles expose their state. #1146's view menu and graph
  toggle are open with Ron (Decision 15).
- **Watching/Paused readout and pause/resume** (Decision 13). Required: a
  permanent readout, visible on every tab and every pane composition, with
  a pause/resume control beside it. Where it sits is Ron's choice; the
  proposal is the main pane's top row, beside the toolbar capsule.
  - The readout says Watching or Paused, in the core's words, with a glyph
    as well as a colour. A state the core did not report is unknown, with
    no dot (see Appearance).
  - While watching, the control pauses with `main`'s three durations (for
    an hour, until tomorrow morning, until turned back on). While paused,
    it resumes. The words come from the core; until the core exports the
    duration words (plan D-12), the shipping menu's words are used, never
    new Swift sentences.
  - The readout is one accessibility element with its state as the value;
    the control is a separate element.
- **Tabs.** Home, Inference and Traces, as `GlassSegmentedTabs`. The map
  selector's second mode is labelled "Private AI" (D11), as supplied by the
  core's copy, not typed in the shell; until the core supplies it, the map
  stays on Traces.
  - **Traces badge:** the core's `decisions_owed`, never queue depth. Hidden
    at zero; a dash, read as "unavailable", when the core did not say;
    capped at `99+`. An amber dot is added when the `QueueShieldState` is
    attention. Its accessible value is the core's `decisionsOwedText`,
    followed by the core's second-look line when the dot shows
    (`MonitorWindowView.tracesDescription`).
  - **Inference dot:** the daemon's reported Private AI state, never the
    switch. Only the core's "clear" tone is on; held, attention and refused
    are amber; no report is no dot. Its accessible value is the core's line
    for the state.
- **Opening state and restoration.** #1146 decides pane visibility once at
  launch (the map if the window is at least 1100 wide, the inspector if at
  least 900). The macOS app restores the person's last choice per window
  (`@SceneStorage`), and uses those widths only to seed the first launch.
  The compact-layout rule above affects rendered visibility, not the saved
  preference. The tab opens on Home by default (see "Behaviour changes
  (decided)").
- **Review opens the inspector.** A session's Review pill selects the
  session and shows the inspector, where its review is
  (`MonitorWindowView.review`). Selecting a row alone only changes the
  selection.
- **Settings.** A macOS Settings window (D8), `MonitorSettingsWindow`. Its
  sections are the ones #1146's modal lists, plus Compute, which shows
  `ComputeView` with pause, resume and withdraw (see Screens).

### Notices

`ShellNotices` is the app-wide consent notice stack. It holds all five of
`main`'s notices:

- `AttachedDaemonNotice`;
- `GrantVoidNotices`;
- `ArmingRewordingNotices`, which also carries #1162's Scrub-check notice;
- `GateHeldNoticeCard`;
- `LegacyMigrationNoticeCard`.

In the monitor window it sits in the main pane, which never hides, between
the top row and the tabs (`MonitorWindowView.swift`). So every notice is
visible on every tab and every map and inspector combination. Every word
comes from `TCConsentCopy` or the core, and a notice the ABI cannot word
draws nothing. The Traces tab's own notices (the folder notice, the
queue-safeguard notices and the core-down line) are `GlassNotice`s above the
tree.

### The approval and undo region

The approval/undo region is a single instance: the pending contribution's
toast with its Undo, the pending keep's Undo, and any refusal of either. It
renders at the top of the inspector when the inspector is open, and at the
bottom of the left pane when it is closed. Moving it never duplicates it,
restarts its deadline or dismisses it, and changing the selection does not
dismiss it. Consent offers follow the same rule. Keep the core's existing
deadline and result handling; the recorded countdown is not a new duration
or a guarantee that an upload can still be cancelled.

On `main` the region renders only inside `SessionInspectorView`, so with the
inspector closed there is no Undo. That is a code gap (see "Open code
gaps").

### Recorded content and interaction details

- **Independent scrolling.** Tabs and the top row remain above the main
  content. The trace tree scrolls within that pane, and the inspector
  scrolls independently. At the minimum window size and larger text sizes,
  all controls remain reachable without text or buttons overlapping.
- **Trace rows.** Preserve tool → project → session indentation, disclosure
  controls, tool/folder glyphs, primary name and secondary state/count line.
  Align trailing actions across rows. A selected row has a full-width
  highlight distinct from hover, keyboard focus and contribution status.
  Clicking a disclosure or an action must not also trigger the row's other
  actions. Empty projects retain their row. Tool rows carry the source
  switch and folder rows the mode picker, Submit all and Submit all as (see
  Non-goals).
- **Inspector context.** With no selection, show the tab's overview
  (Summary on Home); a session selection shows its details and review, a
  folder selection shows the folder inspector with its Submit all and
  Submit all as, and Inference shows Private AI details. Keep headings,
  paired shared/kept count wells, disclosure sections and quiet runtime
  cards in that order where applicable. Clearing selection returns to the
  tab's overview. Pane toggles and resizing retain selection and scroll
  position.
- **Review in the 300pt inspector.** The review's consent content
  (`GATE_STATEMENT`, `ScrubbingCaveat`, `WitnessReviewConsent` and
  `AdmissionPreparationView`) scrolls within the inspector and never clips,
  at any text size (see Acceptance).
- **Flow map.** Put the Traces / Private AI selector at the map's upper
  trailing edge. It changes the map view, not the main tab, consent or
  routing configuration. Retain its choice while the map is hidden. Use the
  same node/arc hierarchy in both modes, with labels and an accessible
  alternative to pointer-only node inspection. The recorded credential
  shield and hand-written routing copy remain excluded by Non-goals.
  #1146's binoculars focus and node-card hover and pin behaviour are open
  with Ron.
- **Settings navigation.** Inside the native Settings window, retain the
  section list and grouped, independently scrolling content: Connection,
  Startup, Notifications, Updates, Watching, How traces may be used, Public
  profile, Watched folders, Tools, Private AI, Redaction witness, Projects,
  and Changes on this machine, plus Compute (`SettingsSection`). A section
  whose copy has not loaded is a disabled placeholder, never a missing row.
  The selected section must match the displayed content. Opening or closing
  Settings preserves the main window's tab, selection and pane visibility.
  This adapts the reference's organisation without adopting its scrim or
  close button inside the main window.

### Data

The monitor window is debug-only and reads sample data
(`DaemonDataWiring.sample`, chosen by `TRACE_COMMONS_SAMPLE`) until K1 moves
its stores to the live `DaemonDataClient` and R15 replaces `MainWindowView`
(plan D-13, Phase 4). The ops each tab reads, as `DaemonDataClient` names
them:

- **Traces:** `status`, `list_pending`, `list_projects`, `harness_list`,
  `tool_destinations`, `preview`, and the writes `approve` (with
  `project_id` for a folder), `cancel`, `keep`, `undo_keep`, `dismiss`,
  `set_project_mode` and `set_settings` with `<tool>_source`.
- **Inference and the map's Private AI view:** `tool_destinations`,
  `harness_list`, `inference_calls`, and the provisional Z1 methods (the
  per-model summary, `inference_call_proof`, spend and the Private AI
  switch).
- **Home:** `status`, `list_history`, `history_rollup`,
  `commons_credit_summary` and the missions catalogue.

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
- `GlassPopover` (the popover tier at radius 14);
- `GlassToggleStyle(.standard | .settings | .watch)`: a 40×24 switch
  (`toggleWidth`/`toggleHeight`), or 38×22 for the source switch on tool
  rows (`watchSwitchWidth`/`watchSwitchHeight`), with an 18pt knob; on
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
- **Tree rows in VoiceOver.** A row is one element whose label is its name
  and state line. Its controls are separately focusable children, each
  named from the core's copy: the source switch or mode picker, Submit all
  or Review, Submit all as, and the disclosure.
- **Reduce Motion.** The knob slide and any menu or popover transition are
  removed when Reduce Motion is on (`GlassMotion.systemReducesMotion`).
- **Contrast floor.** A knob, check or label on an on-state fill reaches at
  least 3:1 against it, and the off state is distinguishable from the on
  state by more than colour. White on `toggleOn` is about 4.2:1 and on
  `toggleOnSettings` about 5.0:1. On `watchOn` the knob is `textOnStatus`
  (`#0C0C0E`), about 10.9:1 (`SwitchContrastTests`).
- **No wording in components.** Components author no words, including
  accessibility values and default labels ("on"/"off", "checked",
  "expanded", "More", "Choose…", tile text such as "dir"). Callers pass every
  string in, from the core's copy, or the component uses a native control or
  trait that supplies it (#1178 review).

| #1146 component | macOS build |
|---|---|
| Window, Pane | the Window and layout section (`GlassThreePane`, `GlassPane`) |
| Popover, Menu | Ron's custom-painted `GlassPopover` and `GlassMenu` (with `GlassMenuItem`, `GlassMenuSeparator`), meeting the accessibility requirements above |
| Modal, Sheet | `.sheet`; Settings is its own window; Submit all as is a sheet |
| Scrim | the system sheet dimming |
| Card (quiet, flush, interactive), Well | `GlassCard` using the tier tokens |
| Notice | `GlassNotice`, with a tone; words from the core or `TCConsentCopy` only |
| ConsentBlock | custom; text comes from the core only |
| ListRow, tree | `GlassListRow` in one list (grid columns 16/24/flex/auto/38/22) with arrow-key selection, not a tab stop per row and not `.onTapGesture` selection, so Full Keyboard Access can select a row |
| TableRow/Head, KeyValueList | `Grid` / `Table` / `GlassKeyValueList` |
| SegmentedTabs | `GlassSegmentedTabs`, with a badge, a dot and an accessibility value per segment |
| Breadcrumb, StepProgress | custom |
| Buttons (primary, secondary, glass, round, pill icon, icon, submit, folder, kebab) | `ButtonStyle`s over the tier tokens; see Materials by OS |
| Picker, Select, RadioGroup | `GlassPicker`: a native SwiftUI `Menu` dressed as a glass pill; the placeholder comes from the caller. It carries the folder's three modes |
| Toggle (plain, settings, watch) | a SwiftUI `Toggle` with Ron's `GlassToggleStyle` (`.standard` 40×24, `.settings` 40×24, `.watch` 38×22), meeting the accessibility requirements above. `.watch` is the tool row's two-state source switch only |
| Checkbox (with mixed) | a SwiftUI `Toggle` with Ron's `GlassCheckboxStyle` (15pt); the mixed state's spoken value comes from the caller; its accessible name is its full sentence, never "Option" |
| Expander | `DisclosureGroup` |
| TextField | `TextField` with a custom style |
| TextArea | `TextEditor` with a custom style |
| Skeleton | no shimmer: a loading state per `ScreenState`, drawn as a dash, never as zero |
| Dot, Status, Chip, Tag, Badge | custom; the colour is always paired with a label; "clear" or "on" glyphs use `statusOn`, not the accent |
| ToolTile, logos | vector assets converted from `logo-paths.ts` (Claude, Codex, Antigravity, OpenCode, Theia) at 15pt in the 22pt tile and 20pt in the 30pt tile; initials until then, and for Gemini CLI and Cline, which have no artwork |
| BarGraph | `GlassDayColumn` (the menu-bar panel); the Traces chart footer is open (Decision 15) |
| MapNode, MapArc, NodeCard (flow map) | `Canvas` plus `TimelineView` for the moving dashes; each node is an accessibility element with a label and actions, not a picture |

The preview gallery (`GlassGallery`, `swift run TCDesignGallery`) is a
development tool and does not compile into the release app (`#if DEBUG` or a
separate target).

## Motion

- One curve, `Animation.timingCurve(0.2, 0.8, 0.2, 1, duration:)`, at the
  duration tokens (0.15, 0.18, 0.22, 0.25 and 0.30 seconds), read from the
  easing and duration tokens. Not `.easeInOut` or `.easeOut` (#1182
  review).
- #1146's eleven literal timings are mapped as the Tokens table says. The only
  exception to the one curve is the flow-map dash (`flowDash`, 0.6s linear).
- Reduce Motion (`accessibilityReduceMotion`) removes transitions, including
  pane show and hide, and the flow-map dash animation, as #1146's
  reduced-motion rule does (`glass.css:1409` at `a1a15fbe`).

## Menu-bar popover

#1146 has no design for the menu bar: it has only a doc comment, "Popover
tier: menu-bar panel, floating menus." (`components/surfaces.tsx`). Ron's
R13 panel (`MenuBarGlassPanel`) is the design. Until R15 it is debug-only
(`TRACE_COMMONS_GLASS_MENU=1`) and the shipping item stays menu-style.

- **Presentation.** `MenuBarExtra(.window)`. Inside it the panel draws no
  glass of its own: the system window carries its material, and a second
  layer would be glass on glass. Only the preview window draws the panel's
  own surface.
- **Content, as built.** Three state pills (contribution mode, watching,
  Private AI), each with a sub-list; the shared/kept legend and day graph;
  recent activity; and shortcuts into the app. Nothing is sent from the
  popover except the shipping menu's own writes (pause, resume, turning
  Private AI off) and the contribution override.
- **Pause and resume** sit in the watching pill's sub-list, with the three
  durations, or Resume while paused.
- **The contribution override** (#1208) is set only after the core's
  confirmation for it; Auto contribute's carries the arming disclosure, so
  arming never happens from a single press.
- **The status icon** shows the decisions-owed badge, grey while paused, and
  attention when the core reports a health problem or did not report the
  count.
- **Rows kept from `main`.** The shipping menu has separate core-worded rows
  for health, budget health, witness capacity, the gate-held notice, armed
  projects and the week's totals (`MenuBarView.swift`). The glass panel
  must keep each of them; it does not yet (see "Open code gaps").
- **Keyboard and VoiceOver.** A window-style item loses the menu's nested
  submenus and menu semantics, so the panel meets the custom-component
  duties in Components: every control reachable by Tab, Escape closes, and
  each pill and row is named from the core's copy.

## Screens

Every screen in the current app maps to the new shell. Nothing becomes
unreachable.

| Screen | Where it goes |
|---|---|
| Home | the Home tab: the Watching row, the waiting and contributed counts, the Missions card, and recent history with a way into History. The inspector shows the Summary (`HomeSummaryInspector`) |
| Traces (waiting, review) | the Traces tab: tree in the left pane, review and the folder inspector in the inspector |
| Inference (Private AI) | the Inference tab, with core copy |
| History | Home, then History (breadcrumb) |
| Mission drafts (`main`'s local drafts, `MissionDraftsView`) | a Home page (`.missionDrafts`), reached from Home (R15 plan, Phase 4 Task 8), with its delete confirmation unchanged. Until then it stays in the shipping window |
| Missions catalogue | Home, then Missions (`MissionsPage`) |
| Compute | a Settings section (`ComputeView`), including pause, resume and withdraw |
| Insights, including comparison tasks (`ComparisonTasksView`) | a Home sub-view, or its existing view, as long as it stays reachable (including its delete actions) |
| Skills (`SkillLearningView`) | per history record, in the History page's inspector for the selected row (plan D-7) |
| Onboarding / first run | a single pane over the scene, with StepProgress (#1235 ports #1030's first run; #1120 the passkey screens) |
| Settings | the Settings window (⌘,) |
| Menu bar | the menu-bar popover |

**Keyboard shortcuts.** `main`'s ⌘1–⌘7 select `MainWindowView`'s seven
sidebar destinations, and the monitor window has no tab shortcuts. Whether
the monitor maps ⌘1, ⌘2 and ⌘3 to Home, Inference and Traces is open for
R15. Settings is ⌘, either way.

## Icons

The toolbar glyphs in #1146 are inline SVG (`monitor-toolbar.tsx`). The
macOS app uses SF Symbols:

| Glyph | SF Symbol |
|---|---|
| map | `map` |
| inspector | `sidebar.right` |
| settings | `gearshape` |
| binoculars | `binoculars` |
| view menu (open, Decision 15) | `line.3.horizontal.decrease.circle` |
| graph (open, Decision 15) | `chart.bar` |

With no other shell consuming them, the mapping lives in Swift; an `icons`
group in the JSON is optional.

The tool logos are third-party marks. They are shipped as vector assets only
after a trademark-use check.

## Implementation order

Each step is a PR of its own. #1173's numbering is in brackets. Steps 1–5
and parts of 6 and 7 are merged (`ff7ffa99f`).

1. **Token source and generator [R1].** The JSON, `generate.py`,
   `GlassTokens.swift`, `TokenDriftTests`, and the green palette's removal.
   Merged. Remaining: Ron's call on the window radius, scene glows and window
   rim, `flowDash`, and `mapWidth`.
2. **Type scale [R2].** Text-style mapping, `micro`, the fixed-point guard.
   Merged. No fonts are bundled.
3. **Materials [R3].** The native backdrop for both OS paths, Reduce
   Transparency (`paneOpaque`), no own pane edge on 26. Merged.
4. **Components [R4].** The glass rendering behind C2's API, the single
   `glassSurface` call site, the tool logos, Ron's `GlassMenu`,
   `GlassPopover` and `GlassToggleStyle`, and the floating blur before 26.
   Merged. Remaining: the accessibility requirements on the custom menus,
   popovers and toggles (see Components), pressed fill, arrow-key lists.
5. **Window shell [R5].** Three panes, the toolbar, tabs, restoration, all
   four pane compositions, independent scrolling, the notices and the
   Settings window. Merged (debug-only). Remaining: the Watching/Paused
   readout with pause and resume, and the undo region outside the
   inspector.
6. **Screens [R6–R12].** Move screens one at a time, keeping `main`'s
   behaviour and the core's copy, including the folder Submit all and
   Submit all as and the folder inspector (#1241).
7. **Menu-bar popover [R13].** Built (debug-only). Remaining: `main`'s
   health, budget, witness, gate-held, armed and weekly rows.
8. **Accessibility [R14].** Increase Contrast (built), focus, VoiceOver
   grouping. Reviewed by the brand owner.
9. **Cutover [R15].** The live client, the monitor as the main window, and
   the glass menu-bar panel as the release default
   (`docs/superpowers/plans/2026-10-02-macos-glass-rebuild-and-r15-cutover.md`).

## Acceptance

- `python3 scripts/design-tokens/generate.py --check` passes and
  `TokenDriftTests` pass.
- At 1320×760 on macOS 26, screenshots match #1146's monitor window in
  layout, radii and colours. They are not expected to match the painted
  scene, because real glass shows the desktop.
- Every accent fill and tint uses `#6D14F3`/`primaryFill`; `#C9B3FF` appears
  only as text; every glyph on an accent fill reaches at least 3:1, checked
  for the consent read-gate checkbox, check circles and `.borderedProminent`
  buttons in the shipping app, in both appearances.
- **Contrast on live-desktop glass.** On macOS 26, measured on device over a
  pure-white and a pure-black wallpaper with `glassVeil` applied, in each
  appearance: body text reaches 4.5:1, and glyphs and status dots 3:1, on
  every pane and floating surface. If a surface fails, raise the
  `glassVeil` alpha or pin the glass appearance; do not lower the floor.
  The PR that ships the shell records the measured values.
- Replay the sequence listed in the recording table with synthetic data:
  partial onboarding → Home and both map modes → main + inspector → Missions
  → Inference → Traces and project/session selection → main alone →
  Settings → restored Traces. Exercise main + map as well. Verify all four
  pane preferences at the default size, then resize to the minimum
  (760×560) and back: nothing clips, the map collapses below 1100pt and
  returns only when the saved preference calls for it, and the left pane is
  never hidden.
- During that replay, selection updates inspector context without
  submitting anything, and Settings round trips preserve presentation
  state. Long synthetic names and larger text sizes do not cover trailing
  row actions or clip labels in fixed-size controls.
- **Notices.** Each of the five `ShellNotices` is visible with the map and
  the inspector both hidden, on every tab (`ShellNoticesPlacementTests`
  covers the placement).
- **Watching/Paused.** The readout and the pause/resume control are visible
  on every tab and every pane composition. Pausing for each duration and
  resuming change the daemon's state, and an unreported state draws no dot.
- Repeat the approval/Undo flow with the inspector closed and while changing
  tabs and pane visibility. Core disclosures and the active undo opportunity
  remain reachable, the region is never duplicated, and the deadline is not
  restarted by a layout change.
- **Review in the inspector.** At the largest text size, the review's
  `GATE_STATEMENT`, `ScrubbingCaveat`, `WitnessReviewConsent` and
  `AdmissionPreparationView` scroll within the 300pt inspector without
  clipping, and Review with the inspector hidden opens it.
- **Folder actions.** Submit all shows the folder's `withheld_line` when the
  core supplies one; Submit all as opens a modal and sends the chosen
  verdict; both are absent, not disabled, when the table does not offer
  Contribute; both are reachable from the row and the folder inspector.
- **Tool source switch.** Flipping it shows the core's explanation first,
  and writes nothing when that copy is missing or the person cancels.
- On macOS 14, the same screens render with the material fallback, with no
  missing panes or controls, and with our own pane edges drawn; on macOS 26
  our edges are not drawn on Liquid Glass surfaces.
- **The pre-26 fallback is exercised, not only compiled.** The
  `macos-15-fallback-tests` job in `clients.yml` (advisory) runs
  `swift test --filter TCDesignTests` on `macos-15` with an Xcode 26
  toolchain, which runs the vibrancy branch. `macos-14` cannot build the
  package (its Swift is older than the package's tools version). Before the
  shell ships, the PR also records a manual check on a macOS 14 machine:
  screenshots of each tier and of the Reduce Transparency state. #1173
  already asks Ron to review on both 26 and 14.
- The package still declares `.macOS(.v14)`, and the source-scan test fails
  the build if a glass-only API is used outside the single call site.
- VoiceOver reaches and names every control, including flow-map nodes and
  tree rows (grouped as Components says). Full Keyboard Access can select
  and operate the tree with arrow keys. No component supplies its own
  English wording.
- Reduce Transparency, Increase Contrast and Reduce Motion each change what
  this spec says they change.
- No screen from the current app is unreachable (see Screens).
- The existing macOS test suite and the Swift CI job stay green. The only
  behaviour changes are those in "Behaviour changes (decided)"; no consent,
  withdrawal or Private AI behaviour changes.

## Open code gaps

Under the precedence rule these are defects in the code, not open design
questions. Checked at `efd05c6f8`.

- **No Watching/Paused readout or pause/resume in the monitor window.** The
  top row has only Map, Inspector and Settings. Home's Watching row and the
  debug menu-bar panel's watching pill are not on every tab (Decision 13).
- **Undo only inside the inspector.** `SessionInspectorView.pendingUndo` is
  the only place the region renders, so with the inspector closed there is
  no Undo (see "The approval and undo region").
- **The glass menu-bar panel drops `main`'s rows** for health, budget health,
  witness capacity, gate held, armed projects and the week's totals.
- **Folder Submit all and Submit all as.** Not on `main`'s monitor. #1241's
  port (at `84a45178`) draws Submit all with the withheld line on the folder
  row and Submit all as as an inline `GlassMenu` on the row. It has no folder
  inspector, and Submit all as is not yet a modal (Decision 14).

## Open

- Where D2 (appearance) was revised, and whether "follows the system" is
  the decision (Ron; plan D-2).
- The pre-26 material in the light appearance. `GlassBackdrop` uses
  `.hudWindow` whatever the appearance, and HUD is a dark material; the
  first revision adopted it for a dark-only app.
- Whether the community site adopts the purple brand (Decision 2).
- Ron's call on the window radius 22, the scene glows and the window rim:
  add them or record them as dropped.
- The view menu (with "Show ignored folders"), the graph toggle and the
  Traces chart footer (Decision 15, Ron). If built, they are behaviour
  additions, and the chart needs:
  - its series sources named. In #1146, shared comes from history
    `submitted_at`, and kept from every queue entry's `discovered_at`, so
    #1146's "kept" includes sessions still awaiting a decision. The macOS
    definition of kept must exclude those unless decided otherwise;
  - #1146's three ranges, 24 hours, 11 days and 42 days
    (`traces-model.ts:332`);
  - keyboard-operable day inspection, series told apart by more than
    colour, and zero distinct from missing.
- #1146's binoculars focus and node-card hover and pin behaviour on the
  flow map.
- ⌘1–⌘3 for the monitor's tabs at R15.
- The Liquid Glass tint values per tier on macOS 26. They are tuned on
  device and recorded in the JSON.
- Adding `flowDash` (0.6s linear) to the JSON.
- Removing the unused `mapWidth` token.
- A separate tint for Gemini CLI, and artwork for Gemini CLI and Cline.
- Restyling the Settings window to the glass theme (Ron, D8: "theming may
  follow").
- Where Insights lives long term. This spec only requires that it stays
  reachable.
