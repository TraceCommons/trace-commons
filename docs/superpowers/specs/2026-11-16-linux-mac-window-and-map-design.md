# Linux main window and trace map, matching native macOS

Status: draft for owner review. Nothing here is implemented. Facts about the
current code are cited at `origin/main` 3d65da0e2; anything not read in source
is marked as unverified.

## Decision this spec carries out

On 2026-11-16 the owner chose to make the GTK app look as close to the native
macOS app as possible, rather than "the same app in a Linux look". This
supersedes the target of
[2026-11-11-linux-aesthetic-parity-design.md](2026-11-11-linux-aesthetic-parity-design.md),
which aimed only for a shared visual family. The owner also decided:

- **Font:** bundle Inter (OFL) in place of SF Pro. SF Pro is licensed for
  Apple platforms only. This reverses the note at
  `crates/trace-commons-contributor-gtk/src/ui/style.rs:30`, which says Inter is
  deliberately not bundled.
- **Icons:** bundle an open-licensed icon set in place of SF Symbols, which are
  also Apple-only, and map each symbol the Mac window uses to a lookalike.
- **Glass:** real translucency and blur where the desktop's compositor
  allows it, and a painted fallback everywhere else.
- **Window controls:** Mac layout, but the desktop's own close and minimise
  buttons.

The owner's first-noticed gaps were the map panel and the window structure.
This spec covers those two; other screens' content is out of scope (see
"Not in this spec").

## What the Mac window is

Read from `macos/` source:

- **Scene.** `Window("Monitor")` with a hidden title bar. Its default size is
  `GlassThreePane.defaultWidth` by the `windowHeight` token
  (`TraceCommonsAppMain.swift:222-255`).
- **No sidebar.** The root is `GlassThreePane` (`MonitorWindowView.swift:259`),
  three columns left to right with 10pt gaps
  (`macos/Sources/TCDesign/Components/ThreePane.swift:347-371`):
  - **Main pane:** 440pt and never narrower. It does not change width when
    another pane opens or closes, and it takes all spare width when the map is
    hidden.
  - **Map:** 440pt by default, minimum 360. It absorbs window resizes first. A
    divider in the gap moves width between the main pane and the map.
  - **Inspector:** fixed at 293pt. That is round(440 × 2/3); the token
    `size.inspectorWidth` = 300 is not the value used.
- **Showing and hiding panes.** A hidden pane reserves no space. Opening or
  closing one grows or shrinks the window at its right edge, and the window is
  kept on screen (`ThreePane.swift:156-199`). On first launch the map is shown
  only on screens at least 1100pt wide, and the inspector only on screens at
  least 900pt. After that, the person's choices persist.
- **Main pane, top to bottom** (`MonitorWindowView.swift:823-1004`):
  1. **Toolbar row**, beside the traffic lights. It holds an optional staging
     tag, then one capsule of three toggles (Graph, Map, Inspector), then a
     round Settings gear. Graph works only on Traces. Inspector is disabled
     where no inspector exists.
  2. **Tab strip:** Home, Inference, Traces, Insights. Home is fixed first; the
     other three can be reordered by dragging, and the order persists. Before
     onboarding only Inference is shown. Traces carries a badge for decisions
     owed; Inference carries a status dot.
  3. **Breadcrumb**, on the History and Missions sub-pages.
  4. **Tab content**, with popover notices laid over it.
  5. **Graph footer**, on Traces only: 206pt high, sliding in from the bottom.
- **Inspector.** A pane that can open on Traces, on Home's History page, and on
  Insights' Breakdown detail; never on Inference.
  - It opens when the person selects a session or opens a History row.
  - It also opens whenever something new demands it (`InspectorDemand`,
    `Monitor/MonitorShell.swift:46-66`): an undo window, a contribution, a kept
    session, a folder undo, or a folder's Submit all. A demand going away
    never closes it.
- **Settings** is its own window, 780×640, holding a section list and one
  scrolling body. It is not a sheet over the main window.
- **Not in the main window:** the edge rail (a separate panel, off by
  default) and the menu-bar panel.

### The map

- **Pane.** Always beside whichever tab is showing, while the Map toggle is on.
  A segmented control in its corner switches between a Traces view and a
  Private AI view. Choosing the Inference tab switches the map to Private AI,
  and choosing Traces switches it to Traces; Home and Insights leave it as it
  was.
- **Scene** (`Monitor/FlowMapScene.swift`). Pure data, laid out in a 580×760
  design space and reshaped to the pane.
  - Node kinds: hub, library, tool, folder, harness, destination, device.
  - Arc styles: quiet, dashed, flowing, shared, unknown.
- **Traces layout.**
  - A folder column at 30% of the width shows 10 folders, then a "+N" pill
    that reveals 6 more per click.
  - Tool "coin piles" sit at 50%.
  - The hub sits at 72%, with the library above it.
- **When an arc flows.** Only for an auto-upload folder, while the commons gate
  is open, and only if the core says that tool's sessions go to the commons.
- **Rendering** (`Monitor/FlowMapView.swift`). Arcs are drawn on a timed
  canvas. Nodes sit on a still layer that redraws only on change. A tool's pile
  opens on hover, and a node card shows on hover or keyboard focus. A legend
  and zoom −/+ (0.6–2, steps of 0.25) sit along the bottom.
- **Interaction.** Clicking a folder selects it in the tree, switches to Traces
  and opens the inspector. The graph footer's binoculars focus the map on the
  selected tool, and show the map if it is hidden.
- **Transfers.** History changes, plus the last keep or dismiss, become
  animated transfers (`MapTransferFeed.swift`, `MapTransfers.swift`). Each one
  travels inbound, orbits, goes outbound, ascends and arrives. While a tab
  other than Traces is showing, a transfer card appears at the foot of the map.
- **Reduced motion.** No transfer runs, no dash animation, and the timeline is
  paused.

### Where the logic lives

All of the following is Swift, and none of it is in Rust:

- tree building (`TracesTree.build`)
- map layout (`FlowMapScene`)
- graph bucketing (`TracesGraphModel`)
- `InspectorDemand`
- the transfer feed
- the pane layout

What does come from Rust is the data, over `tc_call` (the reads are
`list_pending`, `list_projects`, `get_settings`, `status`, `tool_destinations`,
`list_history`, `history_rollup` and others), and every word, from
`MonitorScreensCopy` and `MonitorTracesCopy` in
`crates/trace-commons-contributor/src/preview_copy.rs`.

## What the GTK window is now

- **Structure.** An `adw::ApplicationWindow`, 980×720
  (`ui/mod.rs:355-360`). A hand-built row of toggle buttons sits in an
  `adw::HeaderBar` over one `adw::ViewStack`.
- **Pages:** Home, Insights, Queue, History, Private AI and Settings, plus
  Mission drafts when the daemon's `missions_enabled` switch is on
  (`ui/mod.rs:84-97, 140`).
- **Responsiveness.** There are no breakpoints, because the crate builds
  libadwaita with feature `v1_2`. Responsive changes are width checks in tick
  callbacks.
- **Map.** `ui/traces_map.rs`. It lives inside the Queue page and is hidden
  when nothing is pending (`traces_map.rs:~253`). It is three columns of
  widgets (folders, tools, "This computer") with cairo lines behind them, and
  its scene is computed in GTK from `list_pending` alone.
- **Inspector.** `ui/traces_inspector.rs`. It is the end child of a
  `gtk::Paned` inside the Queue page, shown when a folder or session is
  selected.
- **Data.** The contributor crate is linked in-process. `Backend` either talks
  to a running daemon over its socket or hosts one embedded, and every read is
  a JSON-RPC method.
- **Wording.** GTK already reads `monitor_screens_copy()` for Home's tab label
  and the map words. The toolbar toggles, tab names, graph and map words all
  exist in `MonitorShellCopy`, but the Insights, Queue, History and Settings
  tab labels are hard-coded English.
- **Plain launch.** A launch with no flags opens the account-free local
  Insights window (`main.rs:169-175`), not the contributor window. That is the
  window the owner first saw.

## Target

The GTK contributor window reproduces the Mac Monitor window's structure,
sizes, behaviour and look, using the same data and the same words.

### Window

- **Three panes.** Main 440 | map 440 (minimum 360) | inspector 293, with 10px
  gaps, built as one custom GTK container whose allocation rules copy
  `GlassPaneLayout`. A divider between main and map moves width the same way.
- **Pane persistence.** Pane visibility and widths persist per user, in the
  shell's existing settings store. First launch seeds them from the monitor's
  width using the Mac thresholds.
- **Window controls.** There is no `adw::HeaderBar`. The main pane's toolbar
  row is the title bar: a `gtk::WindowHandle` holding the staging tag, the
  Graph/Map/Inspector capsule and the Settings gear, with `gtk::WindowControls`
  at the end and side the desktop's `gtk-decoration-layout` names. On a
  desktop that puts the buttons on the left, they take the place of the
  traffic-light clearance.
- **Tab strip.** Home, Inference, Traces, Insights, labelled from
  `MonitorShellCopy` (`tab_home`, `tab_inference`, `tab_traces`, and the
  Insights heading the Mac app uses). Home is fixed first. Before onboarding,
  only Inference is shown. Badge and dot as on Mac.
- **Tab reordering by drag is deferred.** It is a separate small slice once the
  strip exists.
- **Settings** opens in its own window, holding the existing GTK Settings view.
  Its content is unchanged by this spec.
- **Graph footer** on Traces, with the same zoom, range and focus behaviour.
  Previous, next and zoom change only the graph's range.
- **Inspector** at window level:
  - It hosts the existing session, folder and summary inspectors, and History
    row detail.
  - It opens on selection, and on the same demand keys as the Mac
    `InspectorDemand`.
  - Its availability per tab matches the Mac app.
  - The demand behaviour is listed under "Owner approvals needed", because the
    repo's rule on notices may apply.
- **Opening and closing a pane resizes the window at its right edge, where the
  platform allows.** Wayland compositors do not let an app place or resize a
  mapped window reliably, so this needs a spike (S2):
  - If it fails, the window keeps its size and the panes reallocate within it,
    the main pane first.
  - The minimum window width is the sum of the shown panes, as on Mac.

### Where the existing GTK pages go

| GTK today | Target |
|---|---|
| Home | Home tab |
| Queue (with its map and inspector) | Traces tab. The map and inspector move out to window level |
| History | Home's History sub-page and the Traces History sub-page, behind the breadcrumb, as on Mac |
| Private AI | Inference tab |
| Insights | Insights tab |
| Mission drafts | Home's Missions sub-page while `missions_enabled` is on |
| Settings | Separate Settings window |

### Map

- **Scope.** A full port of the Mac Traces map view: scene layout, node kinds,
  arc styles and flow rule, the folder "+N" reveal, tool piles, the hover and
  focus node card, the legend, zoom, transfers, transfer cards on other tabs,
  and binocular focus.
- **The Private AI map view is a follow-up slice** within this spec. It is
  listed separately because its data (devices, harnesses, destinations) is a
  different read set.
- **Rendering.** Arcs and motion are drawn in one custom widget through
  `gtk::Snapshot` (or cairo), driven by a frame-clock tick. Nodes are real
  child widgets, so they get focus, accessible names and descriptions.
  Reduced motion follows `gtk-enable-animations` and pauses the clock, as the
  Mac app pauses its timeline.
- **No hiding.** The map is not hidden when nothing is pending; it shows the
  same scene the Mac app shows for that state.

## Shared logic: the port

GTK cannot reuse the Swift logic, so the tree, the scene layout, graph
bucketing, inspector demand and the transfer feed must be written again. To
avoid writing them twice more (Windows, Tauri), and to stop the Mac app and
Linux drifting apart, they go into permissive Rust code. The Mac app keeps its
Swift implementations for now.

- **Conformance.** A Swift test dumps each model's output for every recorded
  sample set as JSON into a checked-in fixtures directory. The Rust tests read
  those files and must produce identical output. A change on either side
  without the other fails a test.
- **Later.** Switching the Mac app to consume the Rust models through the FFI
  is a separate later decision, and it removes the duplication.
- **Where the code lives is an owner decision** (see below).

## Shared test data and side-by-side captures

- **Reuse the recorded samples.** The Mac app already has samples recorded
  from the real daemon by `crates/trace-commons-contributor/tests/k2_sample_recorder.rs`.
  They are stored at `macos/Sources/TCShellCore/DataContract/RecordedSamples/`,
  in sets `empty`, `normalDay`, `busyQueue`, `heldSessions`, `armedFolder` and
  `unknownCounts`. Each set holds `list_pending`, `list_history`, `list_kept`,
  `tool_destinations`, `status`, `get_settings`, `history_rollup` and others.
  A drift test keeps them matched to the daemon.
- **Gaps in those samples:**
  - No set records account devices, which the map reads.
  - The inference reads beyond `inference_calls` are hand-written.
  - Which sets contain withdrawn traces is unverified.
  - The recorder needs extending for any read the map uses and the sets lack.
- **GTK reads the same samples.** Add a debug-only `Backend` variant that
  answers from a sample set, chosen by the same `TRACE_COMMONS_SAMPLE`
  variable the Mac app reads (`MonitorWindowView.swift:182-204`). It must be
  impossible in release builds, as on Mac.
- **Mac captures.** The current capture hook
  (`TraceCommonsApp/DebugScreenshot.swift`) renders separate components (the
  tree, the inspector) from live daemon data, not the three-pane window, and
  not from a sample set.
  - Extend it to render the whole Monitor window, per tab and pane
    combination, from `TRACE_COMMONS_SAMPLE`.
  - It renders with `ImageRenderer`, which shows no real blur. Mac captures
    therefore compare against Linux's painted glass, which is the fair
    comparison.
- **Linux captures.** Render under headless Weston with
  `weston-screenshooter`, the same path `scripts/weston-portal-verify-inner.sh`
  uses. Don't use the Xvfb scripts that click at fixed coordinates.
- **Gallery.** A script pairs the two captures for every sample set, tab, pane
  combination and appearance (light and dark), and writes one HTML page.
- **Not uploaded from CI.** The gallery is produced locally, and in CI as an
  artifact only if the owner asks. The current workflow does not upload
  screenshots, and this spec does not change that.

## Look

- **Tokens.** Every value comes from `design-tokens/glass.tokens.json` through
  `scripts/design-tokens/generate.py`: colours, radii, spacing, shadows, type,
  motion, and the `GlassTier` roles (pane, card, cardQuiet, well, control,
  controlSelected, popover, menu, nodeCard). Extend the generator's GTK output
  to cover what this spec uses; nothing is hand-copied into CSS.
- **Glyphs.** The toolbar glyphs (viewMenu, graph, map, inspector, gear,
  chevronDown, folder, kebab) are the repo's own SVG paths
  (`TCDesign/Components/Glyphs.swift:9`). Export them to SVG once, from the
  same source, for GTK. This needs no licence approval.
- **Icons.** About 30 SF Symbols appear across the window, tree and map. The
  chosen icon set needs a lookalike for each, recorded in one checked-in
  mapping table.
- **Font.** Inter is loaded at application start from the app's own files.
  The Flatpak manifest installs it.
- **Glass:**
  - **Painted (every desktop):** opaque surfaces using the tokens'
    `paneOpaque` and tier fills. This is what the Mac app itself uses under
    Reduce Transparency.
  - **Translucent:** a transparent window background with the veil and tier
    fills at token opacity. Enabled only where real blur is also available,
    because translucency without blur fails the tokens' contrast floors over
    an arbitrary desktop.
  - **Blur:** needs a spike (S1). Wayland KDE exposes a blur protocol and X11
    KDE a window property. Both names are from background knowledge, not
    verified in this repo, and both need crates that are not in the GTK
    lockfile today (no `wayland-client`, `gdk4-wayland`, `gdk4-x11` or
    `x11rb`). GNOME offers apps no blur, so GNOME gets the painted look.

## Spikes before implementation

- **S1. Blur and translucency.** On a real KDE Plasma session (Wayland and
  X11) and a GNOME session: does a transparent GTK 4 window composite
  correctly, and can the blur request be made from gtk4-rs 0.9? Produce the
  smallest crate set it needs, with the dependency write-up.
- **S2. Resizing on Wayland.** Can a mapped GTK 4 window grow at its right
  edge when a pane opens, on GNOME and KDE? If not, confirm that the
  in-window fallback looks right.
- **S3. Recorded samples.** Confirm which reads the map, tree, graph and
  inspector need that the recorded sets lack, and that every tab's states are
  covered: empty, populated, unknown, held, armed, withdrawn and kept.

## What this breaks, deliberately

- **`screens()` order and length** (`ui/mod.rs:2185+`). It feeds the stack,
  the tray menu rows (`tray.rs:148-191`) and the position-derived `<Control>N`
  accelerators (`shortcuts.rs:61-145`). Their tests change with the new tab
  list.
- **Accelerators.** Accelerators should follow the new tab order.
  `<Control>,` for Settings would match Mac; it is not in the GTK app today.
- **`ui/css_contract.rs` `SOURCES`** must list each new view file.
- **`tests/shell_wording.rs`.** Moving tab labels to `MonitorShellCopy` should
  shrink the recorded baseline, not grow it. No new words are authored in GTK.
- **Weston OCR gate** (`weston-portal-verify-inner.sh`). It reads the Insights
  page's own tabs, which should survive. Rerun it, and update the strings only
  if the Insights content moves.
- **Xvfb click scripts.** `settings-shot.sh`, `roots-answer.sh` and
  `removed-dialog-shot.sh` click at fixed coordinates and will miss. Replace
  them with the Weston capture path, or retire them.
- **Map and inspector tests** move with their widgets. The selection-identity
  and reduced-motion assertions stay.
- **`linux-shell desktop integration (weston + portal)`** in
  `.github/workflows/clients.yml` builds with `--locked`. Any new crate updates
  `Cargo.lock` and `flatpak/cargo-sources.json`.

## Owner approvals needed

1. **Notices.** The repo forbids new notices without explicit approval of the
   notice and its wording. These Mac behaviours would be new on Linux:
   - popover notices over the tab content
   - transfer cards at the foot of the map on non-Traces tabs
   - the inspector opening by itself on a demand

   Approve each, or say which to leave out. Their words already exist in the
   shared copy.
2. **Assets.** Inter and the icon set, with the usual write-up (version,
   licence, size, maintenance) before they land. Candidates for the icon set
   are Lucide (ISC) and Phosphor (MIT). The write-up compares how close each
   is to the ~30 symbols.
3. **Blur crates.** Whatever S1 finds, with the same write-up.
4. **libadwaita floor.** The crate builds against libadwaita 1.2. This design
   needs no newer widget, because the three-pane container is custom, so the
   recommendation is to stay on 1.2. Raising it would give breakpoints, but it
   drops older distributions.
5. **Where the shared models live.** Recommendation: a new permissive leaf
   crate, `trace-commons-shell-model`, MIT OR Apache-2.0, depending only on
   the protocol crate and `serde`. Adding it touches the crate sets in
   `crates/trace-commons-server/tests/license_boundary.rs`; it adds a
   permissive crate, and no AGPL edge. The alternative is a module inside
   `trace-commons-contributor`, which rebuilds with every daemon edit.

## Slices (one PR each)

1. **Shared samples in GTK, plus the capture gallery.** Add the debug sample
   backend, extend the Mac capture to whole windows, and add the pairing
   script. Its output is the "before" gallery.
2. **Shared models in Rust,** with Swift-generated conformance fixtures.
3. **Window shell:**
   - the three-pane container
   - the toolbar row with the desktop's controls
   - the tab strip
   - Settings in its own window
   - the window-level inspector host
   - the graph footer
   - the page moves in the table above
4. **Traces map:** scene, nodes, arcs, legend, zoom and focus.
5. **Transfers and motion,** with reduced motion.
6. **Look:** tokens and tiers, glyphs, Inter and the icon set (after approval
   2), and painted glass.
7. **Translucency and blur** (after S1 and approval 3).
8. **Private AI map view.**
9. **Tab drag reordering.**

Slices 1 and 2 can run in parallel. Slice 3 depends on 1. Slices 4 and 5
depend on 2 and 3.

## Acceptance

- For every sample set, tab, pane combination and appearance, the paired
  gallery shows the same structure, the same words, the same states and the
  same pane geometry within a few pixels. The owner signs off on the gallery,
  not on test counts.
- The same comparison under enlarged text and reduced motion.
- Keyboard: every map node, toolbar toggle and tab is reachable and named.
- Selection, consent and send boundaries are unchanged: the inspector shows
  what it shows today, and no new action is added.
- No new words authored in GTK (`shell_wording.rs` passes).
- The GTK workspace suite, clippy, fmt, and the Weston integration job pass.
- A final run on a real GNOME and a real KDE desktop, recorded in
  `docs/superpowers/plans/2026-11-11-linux-desktop-qualification.md`.

## Not in this spec

- The content of the Home, Insights, Inference and Settings screens.
- Onboarding.
- The menu-bar panel and edge rail.
- Windows and Tauri.
- Switching the Mac app to the shared Rust models.
