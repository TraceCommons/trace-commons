# Windows app, matching the native Mac app

Status: draft. The owner took the W0 decisions on 2026-11-18 and the rest on
2026-11-19 (see "Decisions taken 2026-11-18" and "Decisions taken
2026-11-19"). Only two items are still open, each gated on a spike (W14-D2,
W15-D1). Nothing here is implemented.
Facts about the code are cited at `origin/main` 19b42d0d2. Anything not
read in source is marked unverified.

This spec does for the Windows app (`windows/`) what
[2026-11-16-linux-mac-window-and-map-design.md](2026-11-16-linux-mac-window-and-map-design.md)
did for Linux. It follows the slice plan in the draft guide #2190 (W0 to
W15), and corrects that guide where the code has moved (see "Corrections to
the guide"). It is step W0: the spec and the decoration inventory.

Path conventions in the tables: Mac paths are under `macos/Sources/`.
Windows paths are under `windows/src/TraceCommons.App/`, except
`Interop/...`, which is `windows/src/TraceCommons.Interop/`. Rust paths are
under `crates/trace-commons-contributor/src/` unless they start with
`crates/`.

## Decisions already made

- Decisions for Windows are asked per slice. Linux's answers are shown as precedent and never assumed.
- Title bar: the Mac's toolbar row extends into the title bar, with the system caption buttons.
- Drawing: Win2D (`Microsoft.Graphics.Win2D`, MIT) is an approved new dependency.
- Font: Segoe UI Variable. It is already the default (`Themes/DesignSystem.xaml:220`, `Segoe UI Variable Text, Segoe UI`). Whether Segoe UI Variable is installed on Windows Server 2025 and on Windows 10 22H2 is unverified; if it is missing, text falls back to Segoe UI with no error. Spike S7 records which face resolves on each test machine.
- Test box: an Azure Windows Server 2025 machine. There is no Windows 11 licence yet. Whether Mica shows there is unverified: the edition, or the remote session turning transparency off, could each prevent it. Spike S7 measures it. See "Test machines".
- Decoration and motion are parity, not polish. Each item lands in the slice it belongs to.
- "Trace Commons" is never user-visible. The name comes from `brand.rs`.

## Decisions taken 2026-11-18

The owner said "Accept all my recommendations" to the W0 decisions. Each
slice below marks its decision as taken and keeps the options that were
offered, for the record.

| Decision | Taken | What it means here |
|---|---|---|
| G-1 | (a) | The WinUI app in `windows/` is unfrozen for parity work now and becomes the Windows client. Tauri on Windows is retired once WinUI matches the Mac. CLAUDE.md is updated in a separate change. |
| G-2 | (a) | The Mac look, as close as Windows allows. |
| G-3 | (a) | The NEAR AI N everywhere: title bar, tray, first-run stage and the MSIX tiles (tiles redrawn in W10). |
| G-4 | (a) | Slices W16 and later cover the surfaces no slice covered. Their decisions were taken 2026-11-19 (see below). |
| G-5 | (a) | `az`-based equivalents of `provision-dev-vm.sh` and `win-exec.sh`; `win-capture.ps1` stays on the box. |
| W1-D1 | (a) | Captures are made on the test machines only, never uploaded from CI. |
| W1-D2 | (a) | A new `net8.0-windows` render-test project, run in the `windows contributor app` CI job. |
| W2-D1 | (a) | History and Mission drafts become Home sub-pages behind a breadcrumb. |
| W2-D2 | (a) | Settings gets its own window, 780 x 640. |
| W2-D3 | (a) | The Mac's edge growth: the window grows at its right edge and slides left at the screen edge (spike S2). |
| W2-D4 | (a) | The Mac's notice stack, X rules and inspector demand. Cards new to Windows were approved as a class on 2026-11-19 (see below). |
| W2-D5 | (a) | Ctrl+1 to Ctrl+4 by tab position, Home first; Ctrl+comma for Settings; Ctrl+Shift+M kept for Private AI. Ctrl+5 goes. |
| W3-D1 | (a) | Tool logos from the Mac's own path data, generated, with the source recorded. |
| W3-D2 | (a) | The Private AI map view lands in W3 with the Traces view. |
| W4-D1 | (a) | The Mac's peeking stack for transfer cards. |
| W5-D1 | (b) | Mica where the system supports it, painted glass as the fallback. Acrylic was not part of the answer. |
| W5-D2 | (a) | The Mac's own glyph paths exported; each SF Symbol mapped to a Segoe Fluent Icons glyph in one checked-in table. |
| W5-D3 | (a) | Stock controls re-templated to the Mac's shapes and motion, keeping their automation peers. |
| W6-D1 | (a) | A map style switch in Settings, as on the Mac. |
| W7-D1 | (a) | No wallet path in first run. |
| W7-D2 | (a) | Reuse the "this computer" variants #2177 adds, after it lands. W7 waits on #2177 for those words. |
| W9-D1 | (a) | The undo-window setting (`ApprovalHoldSeconds`) is removed. |
| W9-D2 | (a) | Match the Mac's grant flow before W9 ships. Spike S5 still gates W9 shipping. |
| W9-D3 | (a) | A contribution override is cleared only where the Mac clears it, so it arrives with the tray flyout (W13). |
| W10-D1 | (a) | All internal ids are kept. |
| W11-D1 | (a) | Hand-drawn pixel art from own geometry where the Mac samples an SF Symbol. |
| W11-D2 | (a) | The walkthrough card is left out. |
| W11-D3 | (a) | The compiled campaign only; no remote feed or image. |
| W12-D1 | (a) | Mark and badge only in the tray icon; no 24 h bars. |
| W12-D2 | (a) | The Mac's badge: 99+ cap, figure cut from the mark ink, never red. The 16 px legibility trade-off is recorded under W12. |
| W12-D3 | (a) | The news ring and idle halo, with the Mac's conditions. |
| W12-D4 | (a) | Credit arrival: tray animation (gleam, mark hop, mini coins) plus the taskbar attention flash and a taskbar coin burst, behind one Settings switch (W17). |
| W13-D1 | (a) | Both the tray flyout and the edge rail (off by default). |
| W14-D1 | (c), superseded 2026-11-19 by (b) | Windows Terminal, plus other terminals each admitted after an argv check. Superseded: see "Decisions taken 2026-11-19". |
| W14-D2 | (a), refined 2026-11-19 | Spike "Show" on Windows. Whether it ships depends on the spike; see "Decisions taken 2026-11-19". |
| W14-D3 | (a) | Per-tool launch buttons, with the per-tool start flow. |
| W15-D1 | (a), gated on spike S4 (2026-11-19) | No background-service flow on Windows unless spike S4 shows two daemons collide. |

Also decided earlier: the title bar is the Mac's toolbar row with the system
caption buttons; Win2D is approved; the font is Segoe UI Variable.

## Decisions taken 2026-11-19

The owner said "go with your recommendations" to every decision still open.
Each slice below marks its decision "Taken 2026-11-19" and keeps the options
that were offered.

| Decision | Taken | What it means here |
|---|---|---|
| W10-D2 | (a) | The invite protocol's display name is the app name alone, from `brand.rs` `APP_NAME`, with no "invite" suffix. No new words. |
| W14-D1 | (b) | Windows Terminal, plus the Windows console host (PowerShell) as the fallback when Windows Terminal is not installed. Both must pass the argv check. No other terminals. This replaces the 2026-11-18 answer (c). |
| W14-D2 | (a), gated | "Show" ships only if its spike finds a reliable way to raise the session window that never steals focus unexpectedly. Otherwise it stays Mac-only. |
| W15-D1 | (a), gated | No background-service flow on Windows unless spike S4 shows two daemons collide. |
| Windows floor | raise | Support Windows 10 22H2 and Windows 11. `TargetPlatformMinVersion` goes from 10.0.17763.0 to 10.0.19041.0, matching `TargetFramework` `net8.0-windows10.0.19041.0`. 22H2 is the tested minimum. |
| W16-D1 to W25-D2 | the option that matches the Mac | Every open decision in W16 to W25 takes the option that matches the Mac, with Linux as precedent where one exists. Each is marked "Taken 2026-11-19: (option)" in its slice. |
| Cards and notices | approved as a class | Cards and notices the Mac shows and Windows does not are ported with the Mac's conditions and core words. Each slice lists the cards it ports in its PR. This closes the per-card "own yes" of W2-D4 and W19-D2. |
| W2 deferrals | approved | The map half of pane open and close, the graph period slide, the graph footer, the zoom fade and the tree search field slide move from W2 to W3. The arming offer motion and the notice mark move from W2 to W9. The Animation coverage table and the slices' decoration tables match. |

### Still open

Only items explicitly gated on a spike result:

- **W14-D2.** Whether "Show" ships, after its spike.
- **W15-D1.** Waits for spike S4. Taken (a), no background-service flow,
  unless two daemons collide.

Where Linux made the same choice, an option is marked "Linux precedent".
That marks history, not a default.

## Goals

- The Windows window reproduces the Mac Monitor window: its structure, sizes,
  behaviour, look, decoration and motion, from the same data and the same
  words.
- First run, consent, Home, the tray and terminals follow the Mac, slice by
  slice.
- Every word comes from the shared Rust core over the C ABI. The shell
  authors no English, and its wording baseline goes down, never up.
- Consent and send boundaries never become more permissive than the Mac's.

## Non-goals

- Shared window, map or layout logic across shells. Windows ports the Swift
  to C#, as Linux ported it to Rust (owner, 2026-11-16).
- New notices. Re-housing an existing notice is not new. A card the Mac
  shows that Windows lacks today is new on Windows, and is asked card by card.
- Tauri on Windows. No parity work goes into it; it is retired once the
  WinUI app matches (G-1).

## What the Mac app is, at 19b42d0d2

- **Window.** `Window("Monitor")`
  (`TraceCommonsApp/TraceCommonsAppMain.swift:222`). The root is
  `GlassThreePane(` at `TraceCommonsApp/Views/MonitorWindowView.swift:302`,
  defined at `TCDesign/Components/ThreePane.swift:262`.
- **Panes.**
  - Main: `paneLeftWidth` 400 + 40 = 440 (`ThreePane.swift:44`,
    `TCDesign/Generated/GlassTokens.swift:582`). It never narrows.
  - Map: 440, minimum 360 (`GlassTokens.swift:584-585`).
  - Inspector: round(440 x 2/3) = 293 (`ThreePane.swift:48`). The 300 token
    at `GlassTokens.swift:586` is not used.
  - Gap: 10 (`GlassTokens.swift:512`).
  - Growth arithmetic: `ThreePane.swift:157-177`. First-launch seeding:
    `ThreePane.swift:204-207`, from breakpoints 1100 (map) and 900
    (inspector) (`GlassTokens.swift:591-592`). Window height token 614
    (`GlassTokens.swift:588`).
- **Toolbar row.** The staging tag, then one capsule of Graph, Map and
  Inspector toggles, then the Settings gear
  (`MonitorWindowView.swift:923-940`).
- **Tabs.** Home, Inference, Traces, Insights
  (`MonitorWindowView.swift:24-32`). History and Missions are Home sub-pages
  (`TraceCommonsApp/MonitorNavigation.swift:33`).
- **Inspector demand.** `Views/Monitor/MonitorShell.swift:46-66`.
- **Notices.** One peeking stack over the tab
  (`Views/Monitor/MonitorNotices.swift:62-71`, rules at `:25-34`;
  `TCDesign/Components/PopoverNotice.swift:182`).
- **Settings.** Its own window, 780 wide (`GlassTokens.swift:596`) by 640
  (`TraceCommonsAppMain.swift:156-168`), with 19 sections in 6 groups
  (`Views/SettingsSections.swift:12-79`).
- **Keyboard.** Cmd-1 to Cmd-4 by tab position, Home always first
  (`TraceCommonsAppMain.swift:291-303`); Cmd-Shift-M for Private AI
  (`:295-296`); Cmd-, for Settings (`:435`); Cmd-F in the preview
  (`Views/PreviewSheet.swift:228`).
- **Theme.** Flat is the shipping theme
  (`TCDesign/Tokens/GlassTheme.swift:29-35`). Port flat values, not classic.
- **Changes since the Linux inventory.** Hero grounds are solid navy #081538
  at both stops (`GlassTokens.swift:404-405`). Home stat cards carry no icon
  (`Views/Monitor/HomeViews.swift:1855-1866`). The hero radial light and the
  stat-card watermark are gone.

All of the window's logic is Swift: the tree, the map scene, graph
bucketing, inspector demand, the transfer feed and the pane layout. The data
comes from the daemon over `tc_call`. The words come from the Rust copy
tables.

## What the Windows app is, at 19b42d0d2

- **Window.** `MainWindow.xaml:29` `Title="Near.ai"`. A custom title bar,
  `AppTitleBar` (`MainWindow.xaml:86`), 34 px high with room for the caption
  buttons, holds the Turn mark and the literal "Near.ai" (`:97`).
  `ExtendsContentIntoTitleBar` and `SetTitleBar` are at
  `MainWindow.xaml.cs:129-132`.
- **Body.** A 184 px rail and one content column (`MainWindow.xaml:107`).
  The rail has up to six entries: Waiting (the default), History, Private
  inference, Insights, Mission drafts and Settings. Mission drafts shows
  only while `MissionsShown` is set (`:328-329`). "Insights" and "Mission
  drafts" are words written in the XAML (`:327`, `:328`). Panes are
  ContentPresenters (`:898-930`).
- **Missing.** No map, no inspector, no Home tab, no graph. Settings is a
  pane. No app-authored animation: no Storyboard, Composition animation,
  gradient, shadow, `SystemBackdrop` or `UISettings` read anywhere in
  `windows/src`. Stock ContentDialog, Expander, ProgressRing and
  ToggleSwitch still animate on their own.
- **Timers.** Three: the 10 s poll in `Controls/ManagedSessionsView.cs:20`,
  a 1 s undo tick that runs only while an undo is live
  (`ViewModels/MainViewModel.cs:48`, `:150-157`), and a visibility debounce
  timer (`MainWindow.xaml.cs:98`, `:175`).
- **Keyboard.** Ctrl+1 Waiting, Ctrl+2 History, Ctrl+3 Private inference,
  Ctrl+4 Settings, Ctrl+5 Insights (`MainWindow.xaml:44-48`), which is not
  rail order; Mission drafts has no key. Ctrl+Shift+M toggles Private AI
  (`:49`), matching the Mac's Cmd-Shift-M.
- **Notices.** Inline cards above the panes, no X, no overlay. The update
  InfoBar is at `MainWindow.xaml:397-411`; the arming card at `:753`.
- **First run.** The legacy `OnboardingWindow`: Welcome, Connect, Consent,
  Watch, Done (`ViewModels/OnboardingViewModel.cs:675-682`).
- **Tray.** `TrayIcon.cs`: `Shell_NotifyIcon` and a Win32 popup menu. The
  Turn raster with a dot for Attention and Unhealthy
  (`Interop/MarkRaster.cs:80-82`).
- **Look.** `scripts/design-tokens/generate.py:52` emits only `SHELL_ROLES`
  colours to `Themes/BrandTokens.xaml`. `Themes/DesignSystem.xaml` is
  hand-written. The accent is the classic purple #8A3DFF
  (`Themes/BrandTokens.xaml:22`); the Mac's flat accent is blue #0091FD
  (`GlassTokens.swift:178`).
- **Packages.** Only `Microsoft.WindowsAppSDK` and
  `Microsoft.Windows.SDK.BuildTools` (`TraceCommons.App.csproj:114-115`).
- **Tests.** One plain `net8.0` project,
  `windows/tests/TraceCommons.Interop.Tests`, which runs on macOS. No WinUI
  test project and no UI automation. The WinUI project compiles only in the
  `windows contributor app` CI job.

## How Windows gets words and data

GTK links the contributor crate and reads Rust directly. Windows reaches the
core only through the C ABI (`crates/trace-commons-contributor-ffi`).

- The header has 185 `tc_*` exports, identical to Rust's.
- The Mac's Swift calls 168 of them.
- C# binds 108 (`windows/src/TraceCommons.Interop/NativeMethods.cs`, 110
  `DllImport` lines, two of them Win32). Every binding is one the Swift also
  calls.
- Nothing compares the C# binding symbols with the header. W1 adds that
  test. Field-level tests already exist for some tables (for example
  `ConsentCopyTests.cs:98`, `AccountAttentionTests.cs:140`); W1's test adds
  to them.

Each slice below sorts its ABI work into four buckets:

| Bucket | Meaning | Work |
|---|---|---|
| (a) | Not in Rust or the header at all | Rust field or export, both header copies, Swift decode contract, C# |
| (b) | In the header, not bound in C# | `DllImport`, decoder and tests |
| (c) | Bound, but unused or partly decoded | C# decoder only |
| (d) | A daemon method Windows never sends | C# call and view model; no ABI work |

The daemon dispatches about 200 methods: 200 unique names in the string
match arms of `crates/trace-commons-contributor/src/daemon/ipc.rs`, counted
with a grep over lines of the form `"name" =>` and `"a" | "b" =>`. The Mac
sends 96
that Windows never sends by name. Three of those already have a Windows
path: `preview` (`tc_preview_open`), `search_original`
(`tc_search_original`) and local Insights (`tc_insights_call`).

Bucket (a) items found:

- No export or copy field carries the app name. It lives in
  `brand.rs:28-35` only.
- First-run and project words exist only as "this Mac"
  (`first_run_copy.rs:618`, `:634`, `:672`, `:731`; `project_copy.rs:173`).
  The map table does have "computer" variants (`preview_copy.rs:811`,
  `:1195-1239`).
- `appearance_copy` and the #2171 background-service words are not on main
  or not exported.

## How parity is checked

- **Recorded samples.** The Mac's samples
  (`macos/Sources/TCShellCore/DataContract/RecordedSamples/`) feed a
  debug-only sample client in `TraceCommons.Interop`. It answers `tc_call`
  from the files, refuses every unrecorded method, never starts a daemon, and
  is compiled out of Release.
- **Probes for the Mac's numbers.** Throwaway SwiftPM packages compile the
  Mac's own Swift and print its numbers. Tests then pin those numbers, to
  1e-6 where they are numbers, citing the Swift file and line. The probe is
  never committed.
- **Pure ports on `net8.0`.** The pane layout, map scene, tree, transfer
  player, stage layout and tray badge geometry are pure C# in
  `TraceCommons.Interop` (or a sibling plain project). Their tests run on
  macOS beside the probe.
- **Offscreen renders.** Win2D draws the map, station, orb, stage, hero art
  and tray mark into an offscreen target. Those renders are diffed against
  the Mac's `ImageRenderer` renders of the same sample, with text masked.
  They run on Windows only, not in the macOS run: in a new `net8.0-windows`
  render-test project in the `windows contributor app` CI job (W1-D2 (a),
  taken).
- **The gallery.** `scripts/parity/gallery.sh` (on main) pairs captures named
  `<set>-<tab>-<panes>-<appearance>.png`. Windows captures come from the test
  machines only, never from CI (W1-D1 (a), taken), through
  `windows/scripts/win-capture.ps1`, which launches the app in the
  interactive session.
- **Accessibility names.** Each ported surface sets
  `AutomationProperties.Name` from core words. Names are checked on the box
  with the UI Automation client in PowerShell (unverified; a driver library
  would be a new dependency).
- **Tests that fail on a wrong port.** Each new test is run against the old
  or a mutated behaviour first and must fail: a wrong spring, a wrong ring
  order, a halo inset of 3 instead of 2.5.
- **The test machines.** See "Test machines" below. The repo's scripts
  drive a GCE box only; `az`-based equivalents are added for the Azure box
  (G-5 (a), taken). Never sign in, enroll or share on a test machine. Clean
  up after each run, and stop the Azure box when done.
- **Owner sign-off** is on the gallery, not on test counts.

### Animation coverage

Every Mac animation, by slice. A slice is not done until each row under its
number is built or listed under "Not ported" with a reason: skipping a row is a
defect, as it is for decoration. Source: the animation audit of the Mac app at
origin/main 7e020ffe0 (89 rows, found by grepping `macos/Sources` for every
animation API and every `GlassAnimationTimeline` user). A new Mac animation
adds a row here in the same change.

**The rule.** Every animation uses the Mac's curve, duration or spring AND the
Mac's frame rate, and its direction is pinned by a test.

- Curve, duration, spring: from the motion tokens (W5: fast 0.15, standard
  0.22, slow 0.3, slide 0.25, spin 0.8, pulse 1.6, curve (0.2, 0.8, 0.2, 1),
  spatial spring 0.9 s bounce 0.04, expressive 0.78 s bounce 0.32) or the
  row's own number. Never a stock WinUI timing, easing or spring.
- Frame rate: the row's rate (30 fps, 60 fps, display rate, or the 10 to 24 fps
  of the pixel art and the dock), not one clock for the whole stage.
- Direction: every dash, sweep, turn and slide has a sign. Dashes use the
  Mac's `dashPhase: -flow` convention; a test reads the sign (pulse travels
  from the tile into the N; map and Private AI dashes run 18 pt per second and
  -22 per 1.1 s). A test that checks only magnitudes passes a reversed port.
- Motion stops when the window is hidden and follows reduce motion live.

Why: the Linux audit found 42 of 89 rows only partial. The causes were stock
toolkit timing instead of the Mac's tokens, 30 fps ticks where the Mac runs 60
fps or display rate, and reversed dash directions. All three are one-pass
failures on Windows.

| Mac animation | Mac source | Mac numbers (curve, duration, spring, fps) | Windows slice |
|---|---|---|---|
| Arming offer: bolt to check, ring pulse, hold, exit | Monitor/ArmingOfferMotion.swift; MonitorNotices.swift:880-884; AppModel.swift:2203-2252; PopoverNotice.swift:154 | symbol replace + bounce; tint spring(0.32,0.7); ring 1->1.9, 0.8->0, easeOut 0.7 s; hold 900 ms; card fades and list below slides up, spring(0.42,0.9) | W9 (moved from W2, 2026-11-19) |
| Graph bars period slide | Monitor/MonitorShell.swift:233-293 | bars move in from the period's side + fade; timingCurve(0.4,0,0.2,1) 0.45 s | W3 (moved from W2, 2026-11-19) |
| Graph footer open/close | MonitorWindowView.swift:418,943,1034 | height 0<->206, move from bottom + opacity; effective curve is the token curve over Motion.slide 0.25 s (.animation at MonitorWindowView.swift:1034), with withAnimation easeInOut 0.25 at :943 on the toggle | W3 (moved from W2, 2026-11-19) |
| Graph zoom swap | MonitorShell.swift zoom | bars swap in place with fade, fast 0.15 s | W3 (moved from W2, 2026-11-19) |
| Map/inspector pane open-close (window resize) | Components/ThreePane.swift:367-374,540-565 | NSWindow frame animated 0.22 s token curve; panes crossfade; instant in full screen | W2 (inspector half), W3 (map half; moved 2026-11-19) |
| Popover notice stack arrival + peek plates | Components/PopoverNotice.swift:225-260 | new card move(edge)+opacity, revealed card fades, spring(0.36,0.86); plates peek 8, x 0.96/0.92, opacity 0.9/0.7; the whole Monitor notice stack also fades in/out (MonitorNotices.swift:76 .transition(.opacity)) | W2 |
| Traces tree search field | Monitor/NudgeViews.swift:150-161 | slide in from trailing + fade, fast 0.15 s | W3 (moved from W2, 2026-11-19) |
| Map arcs moving dashes | Monitor/FlowMapView.swift:80-84,239-243,294 | dash [4,8] x scale, phase 18 pt/s, 30 fps; still when paused/unknown | W3 |
| Submit menu scroll into room | Monitor/ToolFolderInspectors.swift:88 | scrollTo fast 0.15 s | W3 |
| Traces folder drag reorder | TracesViews.swift:699-768 | row follows pointer; release spring(0.3, 0.85) | W3 |
| Traces list scroll to row | Monitor/TracesViews.swift:503 | scrollTo fast 0.15 s | W3 |
| Transfer cards stack at the map foot | FlowMapView.swift:187; PopoverNotice.swift GlassPopoverStack | new card slides from the bottom + fade, spring(0.36,0.86); hold 5 s, stack 6, 3 drawn | W4 |
| Transfer: out to pad, vanish pulse, pad charge | MapTransferDrawing.pulse | outbound 0.70 s; pulse r 6->44, 0.9 s; pad glow 5+4*min(n,4) | W4 |
| Transfer: rise from laptop to orb (standard map) | MapTransferDrawing.rise | ascent 1.1 s, 3-step trail lag 0.045 s | W4 |
| Transfer: rocket lift-off and flight | MapTransferDrawing.rocket, smoke; MapTransferRun.climb | settle 0.5 s; flight 2.6 s, still 26% with ignition shake, then pow(x,2.4); 2 smoke puffs; linger 1.3 s (kept 1.6 s) | W4 |
| Transfer: station dock ring and count | MapTransferDrawing.dock | ring r 28->54 fading over 1.1 s; count label 1.3 s | W4 |
| Transfer: trace orbs ride in along the folder line | Monitor/MapTransfers.swift; MapToolPile.swift MapTransferDrawing.orb | a trace every 1.05 s; inbound 0.70 s cubic in-out; 60 fps | W4 |
| Expander chevron + details reveal | Controls.swift GlassExpander:651; PopoverNotice.swift:110 | chevron 0->90 deg + fade, fast 0.15 s | W5 |
| First run: repo disclosure | ToolsScreen.swift:972,1007 | chevron 0->90 deg standard 0.22 s; past traces fade in | W5 |
| First run: small switch | FirstRunStage.swift FirstRunSwitch :472 | 30x18 track, 14 pt knob, 0.2 s token curve | W5 |
| Hover fill fade | Styling/GlassHover.swift:40; GlassStyle.swift:342 | hover fill opacity 0<->1, fast 0.15 s token curve (31 sites) | W5 |
| Modal fade + scrim blur | Components/Modal.swift:477-495 | layer fade standard 0.22 s; content behind blurred 8 pt under 40% dim | W5 |
| Motion tokens + visibility-gated clock (foundation) | TCDesign/Styling/GlassMotion.swift; Generated/GlassTokens.swift:692; Components/AnimationTimeline.swift | curve (0.2,0.8,0.2,1); fast 0.15, standard 0.22, slow 0.3, slide 0.25, spin 0.8, pulse 1.6; spatial spring 0.9/0.04, expressive 0.78/0.32; clock paused when occluded/minimised/hidden; Reduce Motion -> nil | W5 |
| Press darken | Styling/GlassPress.swift:116,127 | brightness -8% or 8% black wash, fast 0.15 s | W5 |
| Refresh pill icon spin | Components/Containers.swift GlassRefreshButton | one turn per 0.8 s at 30 fps; click completes a turn; continuous while refreshing | W5 |
| Skeleton pulse | Feedback.swift GlassSkeleton | opacity 1<->0.5, 0.8 s each way, token curve | W5 |
| Spinner | Components/Feedback.swift GlassSpinner | 12 pt ring, quarter arc, one turn per 0.8 s, 60 fps cap (28 sites, map loading 20 pt) | W5 |
| Switch knob slide | Components/Controls.swift:1167 | easeOut 0.22 s (settings) / 0.15 s (watch) | W5 |
| Tabs: drag reorder and drop | Navigation.swift:416 | follows pointer, swaps past half a tab; drop spring(0.3, 0.85) | W5 |
| Tabs: ember line and halo glide | Components/Navigation.swift:266,288 | 2 pt ember 30% past tab + 26 pt halo; matchedGeometry glide spring(0.35, 0.85) | W5 |
| Tabs: lift on hover/press/drag | Navigation.swift:244-247 | scale 1.06, rise 2 pt, shadow r8 y3; spring(0.25, 0.8) | W5 |
| Library orb meridians turning | Components/LibraryOrb.swift | 6 meridians, one turn per 18 s, 20 fps, rest 0.4 rad | W6 |
| Library space station turning | Components/LibraryStation.swift | one turn per 26 s, 20 fps, rest 0.6 rad, tilt 42, lean -14 | W6 |
| Map field shimmer | Indicators.swift:660-830 GlassMapShimmerHost | 1 s after shown then every 40 s; band 20 deg, half-width 0.1 diag, 1.1 s timingCurve(0.3,0,0.15,1) | W6 |
| Map pointer light | Indicators.swift:835-882 GlassMapMarkGlow, GlassMapLight | marks within 140 pt lit (quadratic), white bloom; fade 0.15 s | W6 |
| Map tool pile opens under the pointer | Monitor/MapToolPile.swift:141-200 | open tall spring(0.34,0.84); +260 ms wide spring(0.3,0.86); +200 ms names easeOut 0.18; close reversed 0.15/+100/+220; coins re-lay spring(0.36,0.82) | W6 |
| Transfer: coin to front, grows, rim light | MapToolPile.swift coin(), rimLights | coin to front spring(0.42,0.8); grows 1.3x spring(0.3,0.7); rim light once round in 0.45 s | W6 |
| First run: Customize card opens | FirstRun/FirstRunFrame.swift:186-225; UsesScreen.swift:645-655 | graphic and title fade out; card rises on spatial spring 0.76 s bounce 0.04 | W7 |
| First run: N hub and tool tiles travel between steps | FirstRun/FirstRunStage.swift firstRunActor (matchedGeometryEffect) | N 132/64/36/66/56, tiles 52->36->42->26; spatial spring 0.9 s bounce 0.04 | W8 |
| First run: N hub glow breathes | FirstRunStage.swift FirstRunHub.breathing :230-275 | rest 4 s: scale 1+0.08p, opacity 1-0.25p; charged 1.8 s: 1.02+0.12p, 0.9+0.1p; 30 fps | W8 |
| First run: backdrop glow breathes | FirstRunStage.swift FirstRunBackdrop :108-145 | 7 s each way cosine; scale 0.9+0.25p (top-leading), opacity 0.6+0.4p, blur 30; 30 fps | W8 |
| First run: marker check | FirstRunStage.swift FirstRunMarkerCheck :373 | stroke trim 0->1 in 0.34 s on, 1->0 in 0.16 s off, token curve | W8 |
| First run: segmented control pill | FirstRunStage.swift FirstRunSegmented :443 | chosen pill fill 0.15 s token curve | W8 |
| First run: step screens crossfade | Views/OnboardingCoordinatorView.swift:131,155-173 | opacity transition on spatial spring (0.9 s, bounce 0.04) | W8 |
| First run: step thread fades in | OnboardingCoordinatorView.swift:124 | opacity + offset y -10 on spatial spring | W8 |
| First run: step thread segment fills | FirstRunStage.swift:198 | width 0->38 pt over 0.52 s token curve | W8 |
| First run: step thread title lights | FirstRunStage.swift:211 | glow r7 @0.35 + semibold, 0.3 s token curve after 0.42 s delay | W8 |
| Join: chosen card border | JoinScreen.swift:688 | 1.5 pt toggleOn border, standard 0.22 s | W8 |
| Join: orbit ring turns | FirstRun/JoinScreen.swift orbit | 140 pt dashed [3,4] ring, one turn per 90 s | W8 |
| Join: sign-in light ring | JoinScreen.swift orbit pulse | scale 0.3->1.9 over 1.6 s, fade in first 18% then out | W8 |
| Private AI step: route dashes flow | FirstRun/PrivateAIScreen.swift | dash [5,6], phase -22 pt per 1.1 s; 30 fps | W8 |
| Private AI step: tool count ticks up | PrivateAIScreenLayout.counted | 0 until 0.9 s, then +1 every 0.24 s, first visit only | W8 |
| Tools: beam from watched tile into N | ToolsScreen.swift ToolsBeam, :437 | fade 0.5 s; breath 2.2 s; pulses along dash [0.1,26] at 26.1 pt per 1.1 s | W8 |
| Tools: dashed ring turns | ToolsScreen.swift ring() | dash [3,7], one turn per 90 s, fades in from 0.3 s over 0.7 s | W8 |
| Tools: found-tile flash | ToolsRingLayout.flash | 0.52 s from scanMoment-0.06; halo scale 0.88+0.28f; opacity up first 40% | W8 |
| Tools: list rows rise after the scan | ToolsScreen.swift RowRise struct :740-751 (animation :749), applied :604 and :619; card fade :609 | rows opacity 0->1, y 18->0, spring 0.6 s bounce 0.04, stagger 0.08(i+1); list 0.3 s fade | W8 |
| Tools: not-found tiles fly off | ToolsRingLayout.placement (gone) | from scanMoment+0.12, 0.8 s quadratic, radius 130->470, scale x(1-0.4e), fade | W8 |
| Tools: radar sweep | ToolsScreen.swift sweep; FirstRunStageLayout.sweepRotation | lead 1 s, one turn in 2 s from -36 deg; conic tail; 0.4 s fade in/out | W8 |
| Tools: ring tightens and tiles respace | ToolsRingLayout.placement; ToolsScreen.swift:465 | respace over 0.9 s token curve, radius 130->118; dashed ring scales on spatial spring | W8 |
| Tools: tiles bob | ToolsRingLayout.bob | y=-2+2cos(2pi(t+phase)/5), phases 0/1.1/2.3/3.4/4.2 | W8 |
| Tools: tiles burst out of the N | FirstRun/ToolsScreen.swift ToolsRingLayout.placement | delay 0.26+0.08i, 0.78 s overshoot; radius 0->130, scale 0.3->1, opacity min(1,1.6x) | W8 |
| Traces: estimate line rises | TracesScreen.swift:285 | opacity 0->1, y 8->0, spatial spring 0.64 s | W8 |
| Traces: orbs stream to the ring | FirstRun/TracesScreen.swift stream | 3 orbs per row, 1.38 s each, start 0.5 s, +0.46 s per orb, +0.15 s per row, fade 12% each end; 60 fps | W8 |
| Traces: ring fills and number counts up | TracesScreen.swift counted | 220 pt ring trim and number 0->total from 0.9 s over 1.9 s ease-out cubic | W8 |
| Uses: commons pulse on arrival | UsesFlowLayout.pulse | scale 0.7->0.75->1.45, opacity 0->0.9->0 from 0.90/0.92 | W8 |
| Uses: commons ring turns | UsesScreen.swift commons | one turn per 40 s | W8 |
| Uses: gate fills, thumbs-up pops | UsesFlowLayout.gate | Ask scale 0.86->1.34->0.94->1.08->1.02, turn -18/9/-4; Auto ->1.3->0.95->1.02, -16/8 | W8 |
| Uses: orb travels, waits at gate | FirstRun/UsesScreen.swift UsesFlowLayout.orb | Ask 4.4 s after 1 s with gate hold keyframes; Automatic 2.6 s after 0.6 s; 60 fps | W8 |
| Uses: satellites fill the ring | UsesScreen.swift satellites | +1 per arrival, clear 1.3 s after the fifth; each pops scale 0->1 on expressive spring 0.46 s bounce 0.32 | W8 |
| Welcome: N drawn by marker | FirstRun/WelcomeScreen.swift; Logos/GlassBrandMarkDrawing.swift | delay 0.2 s, 1.0 s cubic in-out, pen tip; display rate | W8 |
| Welcome: body and buttons rise | WelcomeScreen.swift | body at 1.45 s, buttons at 1.8 s, same 0.76 s rise from 18 pt | W8 |
| Welcome: hub glow fades in | WelcomeScreenLayout.glow | 0->1 cubic in-out over 1.1 s from 0.15 s; display rate | W8 |
| Welcome: light band crosses the N | WelcomeScreenLayout.shine; GlassBrandMarkDrawing.shineBand | first at 1.5 s then every 15 s; 110-wide band at 18 deg, x -240->470 over 1.35 s token curve | W8 |
| Welcome: title words rise | WelcomeScreen.swift Rise | words at 1.05/1.14/1.23 s; 0.76 s damped spring 1-e^-6t cos 7.5t; y +18 -> 0 + fade | W8 |
| Balance count-up | BalanceArrivalFlourish.swift; TCShellCore/BalanceArrival.swift | 24 linear steps x 50 ms (1.2 s); none under reduce motion | W11 |
| Balance tile coin burst | Components/CoinBurst.swift; Monitor/BalanceArrivalFlourish.swift | 1.7 s; flash 0.28 s r46; 44 coins 220-420 pt/s, drag 3.4, gravity 50, squash 0.72, fade from 55%, spill 120; reduce motion: still at 0.45 s for 1.6 s | W11 |
| Get-started pixel field (corner ripple, waves) | Components/PixelField.swift; Views/GetStartedCard.swift | 10 fps Bayer grid; corner ripple sin(14d-1.6t); waves 0.6+0.13sin(7u-0.9t)+0.06sin(15u+0.55t) | W11 |
| Hero card hover wash | Components/HeroCard.swift:115,146 | 6% heroInk wash, fast 0.15 s | W11 |
| Hero pixel glyph | Components/PixelGlyph.swift; HeroCard.swift GlassHeroSymbol | 190 pt glyph, 6 pt grid, -12 deg, bleed 46/34, ripple + 3-blue drift, 10 fps; still at t=0 | W11 |
| Taskbar attention flash on credit (Mac: Dock bounce) | DockCreditAnimator.swift:99-101 | requestUserAttention(.informationalRequest), one bounce while inactive | W12 |
| Taskbar icon coin burst on credit (Mac: Dock) | TraceCommonsApp/DockCreditAnimator.swift; TCDesign/Components/DockCoinBurst.swift (#2197, on main since 1c2fdcdd3) | 128 pt tile, 24 coins, drag 3.8, gravity 32, flash 0.32 s r56, icon pulse +4.5%; 1.7 s at 24 fps; still frame under reduce motion | W12 |
| Tray credit gleam, mark hop and mini coins (Mac: menu bar) | TCDesign/Components/MenuBarPanel.swift creditGleam/markHop/stripCoins; Monitor/MenuBarGlassPanel.swift:661-712 | gleam up 0.15 s, hold to 0.7 s, fade by 1.7 s; mark hops 2 pt over 0.35 s; 10 mini coins (drag 3.8, gravity 10); 24 fps; Settings toggle, default on | W12 |
| Edge rail slides open/closed | Views/EdgeRail/EdgeRailView.swift:54-82; EdgeRailController.swift:140-170 | move from trailing + opacity, slide 0.25 s token curve; closes 300 ms after leaving; fade only under reduce motion | W13 |
| Tool launch dialog: folder slides into terminal | Views/ManagedFolderExplainer.swift:173-222 | folder x -112 -> -34, scale 1->0.9, token curve 1.2 s, once; rest frame under reduce motion | W14 |
| Credit-animation switch in General (gates the three credit rows) | TraceCommonsApp/Views/Settings/GeneralSection.swift:39 | one switch; off shows the still frame | W17 |
| Insights Skills home motion | Views/InsightsSkillsHomeView.swift:60-75,196,343,372,436-500 | card hover lift -1.5 pt easeOut 0.15; scrollTransition opacity 0.6 / scale 0.985; bars grow from 2% easeOut 0.5 once; numericText roll; story expand snappy 0.25 | W20 |
| Mac chevrons deliberately not animated | Controls.swift:1019 split-button chevron; ListRow.swift:150 tree chevron; Settings/ProjectsSection.swift:428 | rotate 180/90 deg with NO animation (snap) | n/a |
| Native popovers (info buttons, Settings menus) | TCDesign/Components/InfoButton.swift:49 and 3 others | system NSPopover open/close | n/a |
| Other Mac surfaces deliberately static | GlassBarGraph dots, pad line, resting rocket, undo bar, status item states, rail peek switch, map zoom/focus, tab content swap | static or instant on Mac | n/a |
| Promotion card treatments (gallery only) | Styling/GlassPromo.swift; GlassStatePill | aurora 30 fps; shimmer 1.4 s every 6 s; glowBorder 8 s turn; spotlight | n/a |

## Test machines

| Machine | Role | Status | Known limits |
|---|---|---|---|
| Azure Windows Server 2025 VM | Build, logic and UI checks; gallery captures during slice work; spikes S1, S2, S3, S7 | being provisioned | No Windows 11 licence. Mica and Acrylic unverified there (S7 logs `MicaController.IsSupported()` and `DesktopAcrylicController.IsSupported()`; those API names are from memory and are checked in S7). GPU and Win2D drawing unverified (S1). Segoe UI Variable and Segoe Fluent Icons presence unverified (S7). How it is reached is settled by S3; if over RDP, the session may turn transparency off. |
| Physical Windows 11 PC | The release gate: the final gallery, Mica, motion, DPI and a real launch are signed off here | not yet bought | None known. Until it exists, no slice is called released on Windows. |
| Windows 11 ARM VM | Optional: an arm64 launch check | optional, not set up | Whether the FFI library and Win2D ship arm64 builds is unverified. |
| Windows 10 22H2 | The minimum supported OS | not set up | No Mica: the painted fallback (W5-D1 (b)) is what shows. Segoe UI Variable and Segoe Fluent Icons are unverified there, so each needs a fallback (Segoe UI; Segoe MDL2 Assets or exported paths). |

The code's floor is lower than the matrix's: `TargetPlatformMinVersion` is
10.0.17763.0 (`TraceCommons.App.csproj:6`; Windows 10 1809, from memory), and
the target framework is `net8.0-windows10.0.19041.0` (`:5`). Taken
2026-11-19: support Windows 10 22H2 and Windows 11, and raise
`TargetPlatformMinVersion` to 10.0.19041.0 to match the target framework.
22H2 is the tested minimum. The change itself lands with the first slice that
touches the project file; no code changes in this spec.
The project already builds x86, x64 and ARM64 (`:14-15`); the ARM VM would
be the first run of the arm64 build.

Each fallback is a gallery row of its own: a capture with Mica and one
painted, one with Segoe UI Variable and one with Segoe UI, so a missing font
or material is seen, not silently accepted.

## Spikes

- **S1. Win2D on the pinned SDK.** Does Win2D work with Windows App SDK
  1.6.240923002 and minimum platform 10.0.17763? Does it draw on the Server
  2025 box, which may have no GPU (software rendering is unverified)? Can it
  render offscreen to a PNG for the diffs? Pick the package version from
  this spike, not from memory.
- **S2. Window growth.** `AppWindow.Resize` and `Move` with an extended
  title bar, across DPI scales. W2-D3 (a) is taken; this spike says how to
  build it.
- **S3. Exec and capture on the Azure box.** How commands reach the box, and
  whether `win-capture.ps1` works there unchanged. Shapes the `az` scripts
  of G-5 (a).
- **S4. Two daemons.** What the CLI's `daemon run` does on Windows while the
  app's in-process daemon runs, and whether both watch the same sessions.
  W15-D1 is asked after it.
- **S5. The arming gate.** Windows arms through `set_project_mode`
  `auto_upload` (`MainWindow.xaml.cs:896-908`). The daemon records the grant
  terms in force, which may be none (`ipc.rs:6040-6044`, `.flatten()`).
  Whether the automatic pass then sends without an account-level grant is
  not traced. W9-D2 (a) is taken (match the Mac's grant flow); this spike
  still gates W9 shipping, and says what the existing per-project arming
  sends until then.
- **S6. Sample coverage.** Which reads the map, tree, graph, inspector and
  Home need that the recorded sets lack (account devices and the Private AI
  network reads are known gaps).
- **S7. Motion, material and fonts on each test machine.**
  `UISettings.AnimationsEnabled`, and the window visibility and minimise
  events that stop each clock. Log whether Mica and Acrylic report support
  (`MicaController.IsSupported()`, `DesktopAcrylicController.IsSupported()`;
  names from memory, checked here), and which face resolves for Segoe UI
  Variable and Segoe Fluent Icons. Run it on the Azure box now and on each
  machine in "Test machines" as it arrives, so the W5-D1 note rests on a
  measurement.

### General decisions

These apply across slices.

- **G-1. Unfreeze and which Windows client.** CLAUDE.md still says Windows stays on Tauri, frozen, until macOS ships (#1173 D13). The owner asked to start Windows parity in the WinUI app (`windows/`).
  - **Taken 2026-11-18: (a).**
  - (a) The WinUI app in `windows/` is unfrozen for parity work now and becomes the Windows client; Tauri on Windows is retired once it matches, as on macOS; CLAUDE.md is updated in a separate change (Linux precedent: GTK unfrozen for development on 2026-11-11, releases off until it catches up)
  - (b) Spec work only for now; slices wait
- **G-2. Look target.** Match the Mac's look as closely as Windows allows, or keep a Fluent look with the Mac's structure.
  - **Taken 2026-11-18: (a).**
  - (a) The Mac look, as close as possible (Linux precedent, 2026-11-16)
  - (b) A Fluent look with the Mac's structure
  - Note: The owner's remark that decoration and motion are parity pointed to (a); the owner confirmed (a) on 2026-11-18.
- **G-3. App mark.** Which mark the app shows in the title bar, the tiles, the tray and the first-run stage. Asked once here; W2, W8 and W12 apply it, and W10 redraws the MSIX tiles.
  - **Taken 2026-11-18: (a), tiles too.**
  - (a) The NEAR AI N everywhere, tiles too (Linux precedent: the N for the app, tray and Flatpak icon, #2186, still open)
  - (b) The N in the app, tray and stage; the Turn stays on the MSIX tiles only
  - (c) Keep the Turn everywhere
- **G-4. Surfaces no slice covers.** Mac surfaces outside W1-W15 (now section "Slices for the surfaces W1-W15 did not cover"), including three Insights decoration items.
  - **Taken 2026-11-18: (a): slices W16 and later, below.**
  - (a) Add slices W16 and later for them
  - (b) Fold each into the nearest existing slice
  - (c) Mark some not applicable (only the owner can choose this, per surface)
  - Note: No Linux precedent: the Linux spec left screen content out of scope.
- **G-5. Tooling for the Azure test box.** The repo's dev-VM scripts drive a GCE Windows Server 2022 box through gcloud and IAP. Nothing drives the Azure Windows Server 2025 box.
  - **Taken 2026-11-18: (a).**
  - (a) Add `az`-based equivalents of `provision-dev-vm.sh` and `win-exec.sh`, keeping `win-capture.ps1` on the box
  - (b) Drive the box by hand over RDP and run `win-capture.ps1` there
  - Note: No Linux precedent (Linux used GCP desktops).

## Slices

Each slice is one PR. W1 to W4 are a stack. W5 can start beside W2. W7 and
W9 depend on W2. W11 depends on W2 and W5. W16, W17 and W18 depend on the
Settings window from W2. W19 depends on W2 and W3. W20 depends on W2 and W5.
W21 depends on W2 and W3. W22 can start after W9. W23 depends on W2 and
W11. W24 depends on W10. W25 comes last. While a slice is written it runs
only a compile check. At its end it runs the interop suite, the WinUI build
(CI or the box), the contributor crate tests for any core change, and fmt
and clippy.

Decoration rows come from the W0 inventory: 80 items, of which 67 are
missing, 7 partial, 1 ported and 5 not applicable. (The first draft said 68
and 6; the clock-driven text refresh is partial, because Windows already
ticks a 1 s undo count.) Decoration lands in the slice it belongs to. It is
not deferred. Four items moved from the inventory's slice to the first slice
that draws them; each says so. The three rows that had no slice are in W20.

### W1. Samples, gallery and the binding test

**Mac sources:**

- `TCShellCore/DataContract/RecordedSamples/` (sets `empty`, `normalDay`, `busyQueue`, `heldSessions`, `armedFolder`, `unknownCounts`, `shared`)
- `TraceCommonsApp/DebugScreenshot.swift`
- `MonitorWindowView.swift:221-235` (`sampleClient()`: `TRACE_COMMONS_SAMPLE`, debug builds only)

**Windows files:**

- A debug-only sample client in `Interop/`, chosen by `TRACE_COMMONS_SAMPLE`
- A capture driver run through `windows/scripts/win-capture.ps1`
- Gallery naming for `scripts/parity/gallery.sh`
- A new `net8.0-windows` render-test project under `windows/tests/`, and its step in the `windows contributor app` CI job (W1-D2 (a))
- `az`-based equivalents of `provision-dev-vm.sh` and `win-exec.sh` (G-5 (a)), shaped by spike S3

**C ABI:**

- None. The sample client answers `tc_call` from files.
- New test: every `tc_*` `DllImport` in `NativeMethods.cs` exists in the header with a matching signature.

**Tests that pin the Mac:**

- The sample client refuses unrecorded methods and never starts a daemon.
- It does not exist in a Release build.
- The binding test fails when a binding names a missing export.

**Decoration in this slice:**

No decoration items.

**Owner decisions:**

- **W1-D1. Where captures run.** Where the Windows half of the gallery is produced.
  - **Taken 2026-11-18: (a).**
  - (a) On the test box only, never uploaded from CI (Linux precedent)
  - (b) Also as a CI artifact
- **W1-D2. Render tests in CI.** Win2D pixel tests cannot run in the plain `net8.0` test project on macOS.
  - **Taken 2026-11-18: (a).**
  - (a) A new `net8.0-windows` render-test project run in the `windows contributor app` job
  - (b) Render tests on the test box only; CI keeps the pure-logic tests
  - Note: Linux precedent: its pixel tests run in the GTK suite and the weston job, so closer to (a).
- Also applies: G-5 (how the test box is driven).

### W2. Window shell

The toolbar row moves into `AppTitleBar` with the system caption buttons (decided). The rail is replaced by four tabs. Mac notices with no Windows card found in the survey (unverified card by card; W2-D4 (a) is taken, and on 2026-11-19 the owner approved porting these cards as a class, with the Mac's conditions and core words; the slice lists the cards it ports in its PR): a daemon that refused to start, onboarding owed, the attached-daemon limits, first run's last word, a failed or finished action, the traces moving now, the undos, arming rewordings, a consent gate hold, the backlog and idle suggestions, the idle notifications offer, the first-contribution note, Resume while uploads are paused, History's verdict news and verdict notifications offer, and the NEAR AI sign-in card (`Views/Monitor/MonitorNotices.swift:6-25`). Windows today has the update InfoBar, the health banner, the switch-on and grant-void notices, the legacy migration card, the private inference offer, the arming offer and the witness capacity banner.

**Mac sources:**

- `TCDesign/Components/ThreePane.swift`
- `TraceCommonsApp/Views/MonitorWindowView.swift`
- `Views/Monitor/MonitorShell.swift` (demand `:46-66`, graph footer `:68-290`)
- `Views/Monitor/MonitorNotices.swift`, `MonitorNoticeDismissals.swift`, `Views/ShellNotices.swift`
- `TCDesign/Components/PopoverNotice.swift`, `Navigation.swift`
- `TCShellCore/MonitorScreensCopy.swift`

**Windows files:**

- `MainWindow.xaml` and `MainWindow.xaml.cs`, rebuilt
- A pure pane-layout class in `Interop/`
- `Controls/SettingsView.xaml` hosted in its own window, 780 x 640 (W2-D2 (a))
- `TrayIcon.cs` menu rows and the accelerators, for the four tabs
- `Controls/HistoryView.xaml`, `MissionDraftsView.xaml` moved to Home sub-pages (W2-D1 (a))

**C ABI:**

- (b) `tc_monitor_traces_copy_json`, `tc_shell_words_copy_json`
- (b) staging tag: `tc_build_channel_json`, `tc_channel_window_label`, `tc_channel_mismatch_line`
- (b) `tc_feature_profile_json`
- (c) the whole `tc_monitor_screens_copy_json` (today `MonitorShellCopy.cs:17-24` reads three keys)
- (d) `feature_flags_snapshot`, `feature_flags_refresh`, `nudge_opened`, `nudge_decline`
- (b) `tc_arming_offer_copy_json` for the arming notice

**Tests that pin the Mac:**

- Pane widths 440, 440 (min 360), 293 and gap 10; growth arithmetic from `ThreePane.swift:157-177`
- First-launch seeding from 1100 and 900
- Inspector demand keys from `MonitorShell.swift:46-66`; a demand going away never closes the inspector
- Notice stack: peek 8, 3 drawn, 4% narrowing a level, opacity 0.9 and 0.7, spring 0.36/0.86; the covered card fades (it is not hidden at once, the Linux shortfall); no motion under reduce motion
- Pane open and close on `Motion.standard` 0.22 s, none with animations off
- `ShellWordingTests` baseline goes down
- Keys: Ctrl+1 to Ctrl+4 follow tab position with Home first, Ctrl+comma opens Settings, Ctrl+Shift+M still toggles Private AI, Ctrl+5 is gone

**Decoration in this slice:**

| Item | Mac source | Windows today | Status | Weight | Numbers to pin | Decision |
|---|---|---|---|---|---|---|
| NEAR AI N mark (app mark). Moved from W12: the title bar draws the mark first. W3, W6, W8 and W12 reuse the path. | TCDesign/Logos/GlassBrandMark.swift:10 (path); drawn by the map marks (Indicators.swift:759, 849), the stage (FirstRunStage.swift:261), the strip (MenuBarPanel.swift:316) | Windows draws the Turn instead: Controls/BrandMark.xaml:32-58 (two bracket paths, "no gradients"), used in the title bar (MainWindow.xaml:92) and the empty queue (MainWindow.xaml:1876); MSIX tiles in windows/packaging/Assets are the green Turn (Square150x150Logo.png inspected) | missing | high, M |  | G-3 |
| Toolbar row in the title bar | TCDesign/Components/ThreePane.swift (GlassThreePane); MonitorWindowView.swift:979 GlassRuleTabs, :1146 GlassSegmentedTabs | custom title bar exists (MainWindow.xaml.cs:129-130 ExtendsContentIntoTitleBar/SetTitleBar; AppTitleBar MainWindow.xaml:86) but holds only the Turn and the literal "Near.ai", no toolbar row | partial | high, M |  |  |
| Pane open and close (inspector half; the map half moved to W3, 2026-11-19) | TCDesign/Components/ThreePane.swift:358-376 (map and inspector fade), :538-550 (window frame change on the standard curve) | No map control. | missing | medium, S | Motion.standard 0.22 s on the token curve (ThreePane.swift:547, GlassTokens.swift:694); none under reduce motion (:540) |  |
| Notice stack: peeking plates and arrival | TCDesign/Components/PopoverNotice.swift:182 GlassPopoverStack, :201-202, :225 arrival, :238 spring, :247 covered card fades out, :258-260; MonitorNotices.swift:76 (the whole stack fades in and out) | no notice overlay; the one InfoBar is the update notice (MainWindow.xaml:397-411); animation greps 0 (Z1) | missing | high, M | peek 8, 3 drawn (PopoverNotice.swift:201-202); plates narrow 4% a level, opacity 0.9/0.7 (:258-260); spring 0.36/0.86 (:238); the card a new one covers fades (:247); stack fade on .opacity (MonitorNotices.swift:76) | W2-D4 |

**Owner decisions:**

- **W2-D1. Rail destinations.** Where History and Mission drafts go once the rail is replaced by four tabs.
  - **Taken 2026-11-18: (a).**
  - (a) Home sub-pages behind a breadcrumb, as on the Mac (Linux precedent, #2160)
  - (b) Keep a rail beside the tabs
- **W2-D2. Settings window.** Settings is a pane on Windows and its own 780x640 window on the Mac.
  - **Taken 2026-11-18: (a).**
  - (a) Its own window, as on the Mac (Linux precedent)
  - (b) Stay a pane
- **W2-D3. Window growth at the screen edge.** On the Mac a pane opening grows the window at its right edge and slides it left at the screen edge.
  - **Taken 2026-11-18: (a).**
  - (a) The Mac's behaviour, with `AppWindow.Resize` and `Move` (spike S2)
  - (b) The window keeps its size and the panes reallocate inside it (Linux precedent: forced by Wayland, not chosen)
- **W2-D4. Notices and inspector demand.** Windows stacks inline cards above the pane with no X. The Mac lays a peeking stack over the tab and opens the inspector on demand.
  - **Taken 2026-11-18: (a).** Taken 2026-11-19: cards new to Windows are approved as a class, with the Mac's conditions and core words; each slice lists the cards it ports in its PR.
  - (a) Both as on the Mac: the overlay stack with its X rules, and the inspector opening itself on the Mac's demand keys (Linux precedent: decision 1, "all three approved")
  - (b) Re-house the existing cards only; the inspector opens on selection only
  - Note: Either way, a card the Mac shows that Windows lacks today is a new notice on Windows (copy guide rule 9). The spec lists those cards under W2; they were approved as a class on 2026-11-19.
- **W2-D5. Keyboard map.** Windows binds Ctrl+1 Waiting, Ctrl+2 History, Ctrl+3 Private inference, Ctrl+4 Settings and Ctrl+5 Insights (`MainWindow.xaml:44-48`), which is not rail order, and Mission drafts has no key. Windows already has Ctrl+Shift+M for Private AI (`:49`), matching the Mac's Cmd-Shift-M (`TraceCommonsAppMain.swift:295-296`); it is kept either way. The Mac has Cmd-1 to Cmd-4 by tab position and Cmd-, for Settings.
  - **Taken 2026-11-18: (a), and Ctrl+Shift+M is kept.**
  - (a) Ctrl+1 to Ctrl+4 by tab position, Home first, and Ctrl+, for Settings (Linux precedent: `<Control>comma` at `crates/trace-commons-contributor-gtk/src/ui/shortcuts.rs:44`)
  - (b) Keep the current keys
- Also applies: G-3 (the mark in the title bar).

### W3. Traces map and tree

Drawn with Win2D (decided). Nodes stay focusable elements with names; arcs and motion are drawn. The Private AI map view lands here too (W3-D2 (a), taken): `PrivateAIFlowMap` (`MonitorWindowView.swift:1053-1085`) builds `FlowMapScene.privateAI` (`FlowMapScene.swift:645`) from the harness list, `tool_destinations` and the Private AI state. Spike S6 names the reads the recorded samples lack for it (account devices and the Private AI network reads are known gaps); W3 records them before its map tests can pin that view.

**Mac sources:**

- `Views/Monitor/FlowMapScene.swift` (including `privateAI` at `:645`), `FlowMapView.swift`, `MapToolPile.swift`
- `Views/MonitorWindowView.swift:1053-1085` (`PrivateAIFlowMap`)
- `Views/Monitor/TracesTree.swift`, `TracesViews.swift`, `TracesStore.swift`
- `TracesInspectorHost.swift`, `SummaryInspector.swift`, `ToolFolderInspectors.swift`
- `TCDesign/Logos/GlassLogoPaths.swift`, `GlassToolLogo.swift`, `SVGPathData.swift`
- `TCDesign/Components/Indicators.swift:251` (`GlassToolTile`), `:887` (`GlassNodeCard`)

**Windows files:**

- A new map control (Win2D) and a tree
- Scene and tree as pure C# in `Interop/`
- Generated logo path data
- `ViewModels/QueueGroupViewModel.cs`, `QueueEntryViewModel.cs` replaced by the tree

**C ABI:**

- (b) `tc_monitor_traces_copy_json` (shared with W2)
- (b) `tc_residual_secret_line_text`, the line for secrets found and left (Mac: `TraceCommonsApp/Views/Monitor/TracesStore.swift:568`)
- (d) `tool_destinations`, `keep`, `list_kept`, `undo_keep`, `history_detail`, `include_past_sessions`, `list_past_sessions`

**Tests that pin the Mac:**

- Port `FlowMapSceneTests` against the samples
- Folder column at 30%, 10 folders then "+N" revealing 6; piles at 50%; hub at 72%
- An arc flows only for an auto-upload folder, gate open, sessions bound for the commons
- Arc dash 5/5; flowing dash 4/8 at width x1.3, 18 pt/s
- The Private AI scene against a sample with harnesses and destinations; nothing drawn as current when the harness read failed

**Decoration in this slice:**

| Item | Mac source | Windows today | Status | Weight | Numbers to pin | Decision |
|---|---|---|---|---|---|---|
| Tool logos (path data). Moved from W6: the map's tool nodes draw logos first. | TCDesign/Logos/GlassLogoPaths.swift:21, GlassToolLogo.swift:36 GlassToolLogoShape, SVGPathData.swift:13 | grep -w logo\|logos over windows/src *.xaml and *.cs: 0 lines; icons are 13 FontIcon lines in 6 files and hand-drawn 16 px Canvas glyphs; Win2D greps 0 (Z5) | missing | high, M |  | W3-D1 |
| Tool tiles. Moved from W11: the Traces rows draw tiles first. W11 and W14 reuse them. | TCDesign/Components/Indicators.swift:251 GlassToolTile; sizes GlassTokens.swift:577-578; 15 call sites in TraceCommonsApp (first ManagedSessionsView.swift:343) | no tile control; grep -w logo\|logos over windows/src *.xaml and *.cs: 0 lines | missing | medium, M | tile 22, large 30 (GlassTokens.swift:577-578) |  |
| Map arcs with moving dashes | TraceCommonsApp/Views/Monitor/FlowMapView.swift:80-84 (timeline + Canvas), :239 restingMotion, :263-298 drawArcs (dashPhase :294) | No map control. | missing | high, M | line 1.4 x scale; dashed 5/5; flowing dash 4/8 at width x1.3 moving 18 pt/s (FlowMapView.swift:82, 268-294) |  |
| Map nodes: discs, laptop, launch pad, key glyph, rings | TraceCommonsApp/Views/Monitor/FlowMapView.swift:313 drawNode, :435 drawLaptop, :455 drawPad, :488 keyGlyph, :511 ringMetrics, :767 still Canvas | No map control. | missing | high, L |  |  |
| Map node card | TCDesign/Components/Indicators.swift:887 GlassNodeCard; FlowMapView.swift:680; nodeCardEdge GlassTokens.swift:449 | No map control. | missing | medium, S |  |  |
| Traces folder drag settle and scroll | TraceCommonsApp/Views/Monitor/TracesViews.swift:503 (scroll on GlassMotion.fast), :764 (drag release spring 0.3/0.85); ToolFolderInspectors.swift:88 (fast) | animation greps 0 (Z1) | missing | low, S |  |  |
| Pane open and close (map half; the inspector half stays in W2). Moved from W2, 2026-11-19. | TCDesign/Components/ThreePane.swift:358-376 (map and inspector fade), :538-550 (window frame change on the standard curve) | No map control. | missing | medium, S | Motion.standard 0.22 s on the token curve (ThreePane.swift:547, GlassTokens.swift:694); none under reduce motion (:540) |  |
| Traces graph footer: bars, period slide, open and close. Moved from W2, 2026-11-19 (earlier moved from W5): the footer is part of the shell, as in the Linux spec. | TCDesign/Components/Indicators.swift:358 GlassBarGraph (Canvas :433, hover :460), :414-416 and :537 GlassBarSlide; TraceCommonsApp/Views/Monitor/MonitorShell.swift:233-290 (period steps slide, zoom swaps in place), :292-293; MonitorWindowView.swift:410-418 (footer rises from the bottom), :943, :1034 | no graph footer; Insights draws static Rectangle bars (Controls/InsightsView.xaml:57-58, 119-120, 136, 189, 282-283), a different chart; animation greps 0 (Z1) | missing | medium, M | period slide 0.45 s on (0.4, 0, 0.2, 1) (MonitorShell.swift:293); zoom on GlassMotion.fast (:284); footer opens over 0.25 s (MonitorWindowView.swift:943, :1034) |  |
| Tree search field reveal | TraceCommonsApp/Views/Monitor/NudgeViews.swift:150 (opacity + move trailing), :153/:161 fast | animation greps 0 (Z1) | missing | low, S |  |  |

**Owner decisions:**

- **W3-D1. Tool logos.** The map coins, tool tiles, first-run ring and launch art draw tool logos.
  - **Taken 2026-11-18: (a).**
  - (a) The Mac's own path data from `GlassLogoPaths.swift`, generated, with its source recorded (Linux precedent: logos allowed "as needed", #2182)
  - (b) Initials only
- **W3-D2. Private AI map view.** The map's second view reads devices, harnesses and destinations.
  - **Taken 2026-11-18: (a).**
  - (a) In W3 with the Traces view
  - (b) A follow-up slice after W4 (Linux precedent: slice 7 of its spec)

### W4. Transfers and motion

**Mac sources:**

- `Views/Monitor/MapTransferFeed.swift`, `MapTransfers.swift`, `TraceTransferCard.swift`
- `MapToolPile.swift:215` (`MapTransferDrawing`)

**Windows files:**

- The map control
- A transfer player as pure C# on an explicit clock

**C ABI:**

- None beyond W3. The feed is built from history and keep events.

**Tests that pin the Mac:**

- Phase lengths from `MapTransfers.swift:112-125`
- Hold 5 s, stack of 6, 3 drawn (`MapTransfers.swift:262-264`)
- A still frame with animations off; no ticks while hidden

**Decoration in this slice:**

| Item | Mac source | Windows today | Status | Weight | Numbers to pin | Decision |
|---|---|---|---|---|---|---|
| Transfer cards at the map foot | TraceCommonsApp/Views/Monitor/FlowMapView.swift:170-187 (GlassPopoverStack, anchor bottom); TraceTransferCard.swift; MapTransfers.swift:262-264 | No map control. | missing | high, M | hold 5 s, stack limit 6 (MapTransfers.swift:262-264), 3 drawn (PopoverNotice.swift:202) | W4-D1 |
| Transfer animation: coin rim light, rocket and flame, orb rise (and the station dock ring and count: MapToolPile.swift:484 `MapTransferDrawing.dock`, ring r 28 to 54 fading over 1.1 s, count label 1.3 s, called at :277 and :387) | TraceCommonsApp/Views/Monitor/MapToolPile.swift:215 MapTransferDrawing (orb frame :220, rocket :379, flame :428-429, paths :533-592); MapTransfers.swift:93-232; FlowMapView.swift:107-118 | No map control. | missing | high, L | step 1.05, inbound 0.70, orbit 0.45, outbound 0.70, settle 0.5, flight 2.6, linger 1.3, kept linger 1.6, pulse 0.9, ascent 1.1 s (MapTransfers.swift:112-125); rocket 44 (MapToolPile.swift:379); 60 fps only while moving (FlowMapView.swift:107) |  |

**Owner decisions:**

- **W4-D1. Transfer cards at the map foot.** How transfer cards stack while another tab is showing.
  - **Taken 2026-11-18: (a).**
  - (a) The Mac's peeking stack
  - (b) A short column, newest nearest the legend (Linux precedent on main: `crates/trace-commons-contributor-gtk/src/ui/monitor/transfer_card.rs:16-17`, after #2187 merged)

### W5. Look and motion foundations

W5 is the enabler: motion tokens and the clock stop land before any animated item, and the flat fills and accent before any coloured one.

**Mac sources:**

- `design-tokens/glass.tokens.json`, `TCDesign/Generated/GlassTokens.swift`
- `TCDesign/Tokens/GlassTheme.swift`, `Styling/GlassStyle.swift`, `GlassBackdrop.swift`, `GlassMotion.swift`, `GlassHover.swift`, `GlassPress.swift`
- `TCDesign/Components/AnimationTimeline.swift`, `Glyphs.swift`, `Feedback.swift`, `Controls.swift`, `Navigation.swift`, `Indicators.swift`, `Containers.swift`

**Windows files:**

- `scripts/design-tokens/generate.py` extended to emit flat tiers, radii, spacing, type, shadows and motion to XAML
- `Themes/DesignSystem.xaml` reduced to what the generator does not cover
- `Themes/BrandTokens.xaml` (flat accent)
- Exported glyph paths and a Segoe mapping table (W5-D2)
- A shared clock that honours `UISettings.AnimationsEnabled` and window visibility

**C ABI:**

- None. Generator work only.

**Tests that pin the Mac:**

- `generate.py --check`
- Contrast per tier in light and dark (`BrandTokensTests.cs`)
- Motion values: fast 0.15, standard 0.22, slow 0.3, curve (0.2, 0.8, 0.2, 1), spin 0.8, pulse 1.6
- Pixel tests of the tab ember, status dot and spinner

**Decoration in this slice:**

| Item | Mac source | Windows today | Status | Weight | Numbers to pin | Decision |
|---|---|---|---|---|---|---|
| Glass material under panes (Liquid Glass / HUD vibrancy) | TCDesign/Styling/GlassBackdrop.swift:5-12 GlassMaterial, :65/:97 NSGlassEffectView, :106/:128 NSVisualEffectView, :189 GlassFloatingBlur; ThreePane.swift:439 | backdrop greps 0 (Z4); opaque TcWindowBackgroundBrush on the root grid (MainWindow.xaml:31) | missing | high, M |  | W5-D1 |
| Flat tier fills (pane, card, control, CTA) | TCDesign/Generated/GlassTokens.swift:410-419 (flat: equal stops); TCDesign/Styling/GlassStyle.swift:165 GlassTier, :303 GlassTierFill | solid hand-written surface brushes, not generated: Themes/DesignSystem.xaml:47-53 (dark) and :112-118 (light), "transcribed from design-import/DESIGN-SPEC.md" (:5); scripts/design-tokens/generate.py:52 emits only SHELL_ROLES | partial | high, M | flat cardFill #0D1124 @0.35 (light #FFFFFF @0.6), cardFillQuiet @0.28, controlFill #000000 @0.3, ctaFill #7EBCFF (light #0558B6), ctaSecondaryFill #6AA8EB, checkboxOnFill #7EBCFF (GlassTokens.swift:411-419) |  |
| Flat accent (blue), not classic purple | TCDesign/Generated/GlassTokens.swift:178 tabEmber flat #0091FD, :242 tintAccent flat #0091FD @0.22, :415 ctaFill flat #7EBCFF | Themes/BrandTokens.xaml:22 TcAccentBrush #8A3DFF (dark), :49 #6D14F3 (light): the classic purple, and the tray dot uses the same (TrayIcon.cs:634) | partial | high, S |  |  |
| Floating surface shadows (flat) | TCDesign/Generated/GlassTokens.swift:438 paneEdge, :447 popoverEdge, :448 menuEdge, :449 nodeCardEdge, :450 modal, :451 mapEdge; applied in GlassStyle.swift:139 | shadow greps 0 (Z3) | missing | medium, S | flat: y 18, blur 36, #02030A @0.6 (light #000000 @0.12); modal y 24 blur 60 @0.5 (light @0.225); checkbox inset ring #FFFFFF @0.18 (GlassTokens.swift:446) |  |
| Modal scrim blur and stacked modal fade | TCDesign/Components/Modal.swift:478 blur(modalScrimBlur), :491 opacity transition, :495 standard animation | stands there today: stock ContentDialog (84 ContentDialog lines in windows/src); animation greps 0 (Z1) | missing | medium, S |  | W5-D3 |
| Hover washes | TCDesign/Styling/GlassHover.swift:24-42; GlassStyle.swift:342-347; GlassTokens.swift:150 controlOnSelectionHover; onHover in 25+ places (HomeViews.swift:1161, TracesViews.swift:1415, Surfaces.swift:173, Navigation.swift:134) | grep PointerEntered\|PointerExited: 0 lines; only stock control visual states | missing | medium, S |  |  |
| Press wash and scale | TCDesign/Styling/GlassPress.swift:33 GlassPressStyle, :108 pressed wash, :116/:127 fast animation; 15 uses in the app (first GetStartedCard.swift:164) | animation greps 0 (Z1); stock pressed states only | missing | low, S |  |  |
| Motion tokens and curves | TCDesign/Styling/GlassMotion.swift:4-47 (curve, fast, standard, spatial, expressive); GlassTokens.swift:692-703 | generate.py:52 emits only SHELL_ROLES colours; no motion keys in Themes/DesignSystem.xaml; animation greps 0 (Z1) | missing | high, M | fast 0.15, standard 0.22, slow 0.3, reveal 0.18, slide 0.25, curve (0.2, 0.8, 0.2, 1), spin 0.8, pulse 1.6 (GlassTokens.swift:693-703); spatial spring 0.9 s bounce 0.04 (GlassMotion.swift:40), expressive 0.78 s bounce 0.32 (:47) |  |
| Reduce motion and the hidden-window clock stop | TCDesign/Components/AnimationTimeline.swift:4-32 GlassAnimationTimeline (pauses unless the window is visible); GlassMotion.swift:17 systemReducesMotion | grep UISettings over windows/src: 0 lines; three timers (10 s poll Controls/ManagedSessionsView.cs:20, 1 s undo tick ViewModels/MainViewModel.cs:48 and :150-157, visibility debounce MainWindow.xaml.cs:98 and :175), none stopped by an animation setting | missing | high, S |  |  |
| Status dot with halo and ring options | TCDesign/Components/Indicators.swift:4-31 GlassStatusDot (halo ink 0.08 line 2; ring status @0.25 line 3); 5 app uses (HomeViews.swift:815) | a plain 7 px Ellipse for Private inference working (MainWindow.xaml:307-310), no halo or ring | partial | low, S |  |  |
| Mac glyph set (GlassGlyph paths) | TCDesign/Components/Glyphs.swift:8 GlassGlyph, :95 GlassGlyphView (Controls.swift:273, 538, 623) | Segoe FontIcon glyphs (13 lines in 6 files) and hand-drawn 16 px Canvas icons in MainWindow.xaml | missing | medium, M |  | W5-D2 |
| Warning glyph | TCDesign/Components/Feedback.swift:115 GlassWarningGlyph, :141 shape; PrivateAISection.swift:211 | hand-drawn warning triangle Canvas (MainWindow.xaml:453-463, :571-581) and FontIcon E7BA: own geometry, not the Mac's | partial | low, S |  | W5-D2 |
| Spinner (quarter arc on a ring) | TCDesign/Components/Feedback.swift:15 GlassSpinner (timeline :48 at 1/60 :34, trim 0.25 :53, rotation :55); 28 app uses | stands there today: stock ProgressRing (MissionDraftsView.xaml:17, AccountLoginChangeDialog.cs:38) and indeterminate ProgressBar (MainWindow.xaml:2064, HistoryView.xaml:523) | missing | medium, S | one turn per Motion.spin 0.8 s (GlassTokens.swift:702), 60 fps cap (Feedback.swift:34) | W5-D3 |
| Skeleton breathing | TCDesign/Components/Feedback.swift:74 GlassSkeleton, :88-103 pulse; SummaryInspector.swift:186 | animation greps 0 (Z1); grep -i inspector over windows/src/TraceCommons.App: 0 lines | missing | low, S | pulse Motion.pulse 1.6 s (GlassTokens.swift:703) |  |
| Refresh button turn | TCDesign/Components/Containers.swift:71 GlassRefreshButton (timeline :112 at 1/30, rotation :158), :176 GlassCardRefresh; 6 uses (ManagedSessionsView.swift:658) | static refresh FontIcon E72C (Controls/ManagedSessionsView.cs:96, MainWindow.xaml:1045) | missing | low, S |  |  |
| Disclosure chevron rotation: only GlassExpander animates. The split-button (:1019, 180 deg), tree-row (ListRow.swift:150) and Projects (ProjectsSection.swift:428) chevrons SNAP on the Mac (no animation on `menuOpen`, `expanded` or `open`); Windows must snap them too, and a test pins that | TCDesign/Components/Controls.swift:637-657 GlassExpander (rotation 90, chevron and fade on Motion.fast 0.15 s; PopoverNotice.swift:110 the same); ToolsScreen.swift:971-972 repo disclosure (standard 0.22 s, past traces fade in) | stands there today: stock Expander (17 lines, e.g. MainWindow.xaml:1894, SettingsView.xaml:473), with WinUI's own chevron, not the Mac's | missing | low, S |  | W5-D3 |
| Switch and checkbox motion | TCDesign/Components/Controls.swift:1110 GlassToggleStyle (easeOut :1167), :1198 GlassCheckboxStyle, :1238 GlassCheckMark; FirstRunStage.swift:472 | stands there today: stock ToggleSwitch (9 lines) and CheckBox (10 lines) | missing | low, S |  | W5-D3 |
| UI font family | Mac type scale TCDesign/Generated/GlassTokens.swift:721 TypeScale | Segoe UI Variable Text, Segoe UI (Themes/DesignSystem.xaml:220), the owner's decision for Windows; sizes are hand-transcribed (:226-242), not generated, which W5's generator work covers | ported | medium, S |  |  |
| Tab ember and halo sliding between tabs | TCDesign/Components/Navigation.swift:288 matchedGeometryEffect, :320-340 elliptical halo + linear ember, :225-228, :266 spring | a static 3 px selection bar in the nav rail (MainWindow.xaml:207-213, TcGreenFillBrush), a different control; gradient greps 0 (Z2) | missing | high, S | ember reach 0.3, line 2, halo height 26 (Navigation.swift:225-228); spring 0.35/0.85 (:266); colours tabEmber/tabEmberHalo flat #0091FD, halo @0.31 (GlassTokens.swift:178-180) |  |
| Tab lift on hover and drag | TCDesign/Components/Navigation.swift:244-247 (scale, rise, spring), :296 shadow, :360-362 | animation greps 0 (Z1); shadow greps 0 (Z3) | missing | low, S | scale 1.06, rise 2, shadow @0.18 r 8 y 3 (Navigation.swift:296, 360-362); spring 0.25/0.8 (:247) |  |
| Icon card heads (symbol column) | TCDesign/Components/Containers.swift:213 GlassIconCard, icon :313-318; used in ManagedSessionsView.swift, InferenceAccount.swift, SavedAccountsSection.swift, FundingRow.swift, HomeViews.swift | cards carry no icon column; Segoe FontIcon appears in a few headers only (e.g. Controls/PrivateInferenceView.xaml:347, E756) | missing | low, S |  | W5-D2 |

**Owner decisions:**

- **W5-D1. Glass material.** The Mac draws Liquid Glass under its panes. Windows has an opaque window.
  - **Taken 2026-11-18: (b), Mica only; painted fallback.**
  - (a) Painted glass only, from the flat tokens
  - (b) Mica or Acrylic where the system supports it, with a painted fallback (Linux precedent: the owner chose real blur where the compositor allows, painted elsewhere; what landed paints only)
  - Note: Whether Mica shows on the Server 2025 box is unverified; spike S7 logs it. Windows 10 22H2 has no Mica (from memory), so the painted fallback is what it shows. The physical Windows 11 PC is where Mica is signed off.
- **W5-D2. Glyphs and icons.** The Mac draws its own `GlassGlyph` paths (`TCDesign/Components/Glyphs.swift:8`) and at least 35 SF Symbols (35 distinct literal `systemName:` and `systemImage:` names outside the Gallery, plus any built at runtime), which are Apple-only. Windows uses Segoe `FontIcon` glyphs and hand-drawn Canvas icons.
  - **Taken 2026-11-18: (a).**
  - (a) Export the Mac's own glyph paths, and map each SF Symbol to a Segoe Fluent Icons glyph in one checked-in table (Linux precedent: own glyph paths exported; stock Adwaita symbolic icons in a table for SF Symbols)
  - (b) Segoe Fluent Icons for everything, in one table
  - (c) Own glyph paths, and hand-drawn lookalikes for the SF Symbols
  - Note: Whether Segoe Fluent Icons is present on Server 2025 and on Windows 10 22H2 is unverified (spike S7); Segoe MDL2 Assets is the older font. The table names a fallback for each glyph.
- **W5-D3. Stock controls.** Windows uses stock ToggleSwitch, CheckBox, Expander, ProgressRing and ContentDialog. The Mac draws its own switch, checkbox, chevron, spinner and modal, with motion.
  - **Taken 2026-11-18: (a).**
  - (a) Re-template the stock controls to the Mac's shapes and motion, keeping their automation peers
  - (b) Keep WinUI's stock controls and their own motion
  - Note: No Linux precedent recorded here.

### W6. Map art

**Mac sources:**

- `TCDesign/Components/LibraryStation.swift`, `LibraryOrb.swift`, `ShimmerBand.swift`
- `Indicators.swift:563-885` (map field, marks, shimmer, light)
- `TCDesign/Logos/GlassBrandMark.swift`

**Windows files:**

- The map control

**C ABI:**

- None. Mark and logo paths are Swift, not ABI.

**Tests that pin the Mac:**

- Station face count 506 and corner count 2216 (#2182's pins); projected vertices from the probe
- Orb: one turn 18 s, 6 meridians, lean -14 deg
- Render diff against the Mac

**Decoration in this slice:**

| Item | Mac source | Windows today | Status | Weight | Numbers to pin | Decision |
|---|---|---|---|---|---|---|
| Map field ground: elliptical light, mark tile, dots, edge fade | TCDesign/Components/Indicators.swift:563-603 GlassMapField (EllipticalGradient :580, mask :592), :631-690 GlassMapMarks, :776 tile Canvas; GlassMapStage :605 (MonitorWindowView.swift:1143) | No map control. | missing | high, M | pitch 28, mark 5.6, dot 1.6 (Indicators.swift:635-640); field radii 0.8 x 0.6 centre (0.5, 0.55) (:573-574); flat mapFieldInner #070A1A @0.15, Outer @0.32 (GlassTokens.swift:224-225) |  |
| Map shimmer band | TCDesign/Components/Indicators.swift:787-831 GlassMapShimmerHost/GlassMapShimmer (Animatable :809); ShimmerBand.swift:6 | No map control. | missing | medium, S | first pass 1 s after shown, then every 40 s (Indicators.swift:666-667); none under reduce motion |  |
| Pointer light over the map (mark glow and bloom) | TCDesign/Components/Indicators.swift:835-885 GlassMapMarkGlow, GlassMapLight; onContinuousHover :620 | No map control. | missing | medium, S | reach 140 (Indicators.swift:677); fade on GlassMotion.fast |  |
| Library station (3D mesh round the N) | TCDesign/Components/LibraryStation.swift:23 (timeline :46, glow :51, Canvas :68, mesh :87, projection :223-227) | No map control. | missing | high, L | one turn 26 s, rest 0.6, 20 fps (LibraryStation.swift:34-39); tilt 42 deg, lean -14 deg, camera 240, light (-0.42, 0.62, 0.66) (:223-227) | W6-D1 |
| Library orb (turning meridians) | TCDesign/Components/LibraryOrb.swift:15 (timeline :42, glow :47, Canvas :56) | No map control. | missing | high, M | one turn 18 s, rest 0.4, inset 0.29, 6 meridians, lean -14 deg (LibraryOrb.swift:26-33); glow brand blue @0.5 lit / 0.15 r 10 (:47) | W6-D1 |
| Tool pile coins (springs, grow, glow, names) | TraceCommonsApp/Views/Monitor/MapToolPile.swift:75 Canvas, :137-143, :173-199, :357 blur, :439 glow | No map control. | missing | high, M | grow 1.3, glow r 9 x scale (MapToolPile.swift:137-138); springs 0.42/0.8, 0.36/0.82, 0.3/0.7 (:141-143), open 0.34/0.84, 0.3/0.86, names easeOut 0.18/0.15, close 0.28/0.9, 0.34/0.86 (:182-199) | W3-D1 |

**Owner decisions:**

- **W6-D1. Map style switch.** The Mac offers a rocket style (station) and a standard style (orb).
  - **Taken 2026-11-18: (a).**
  - (a) A switch in Settings, as on the Mac
  - (b) Rocket style only
  - Note: No Linux precedent recorded here.

### W7. First-run steps

Replaces the legacy `OnboardingWindow` with the Mac's steps: welcome, tools, uses, traces, privateAI, terms, join (`TCShellCore/FirstRun/FirstRunState.swift:9-19`).

**Mac sources:**

- `TraceCommonsApp/Views/FirstRun/*`
- `TCShellCore/FirstRun/*` (`FirstRunPlan.swift:6-38`)
- `FirstRunFrame.swift`

**Windows files:**

- `OnboardingWindow.xaml` replaced
- `ViewModels/OnboardingViewModel.cs`
- `Interop/OnboardingCopy.cs` (legacy table) and `AgentConfigurationCopy.cs`

**C ABI:**

- (c) the whole `tc_first_run_copy_json` (today only `agent_configuration`, `AgentConfigurationCopy.cs:19`)
- (b) `tc_terms_document_json`, `tc_first_run_estimate_line`, `tc_describe_folder`, `tc_install_guides_json`, `tc_cli_tool_json`
- (d) `first_run_copy`, `terms_accept`, `list_past_sessions`, `include_past_sessions`, `managed_tool_setup`, `set_private_ai`, `automatic_grant`, `grant_automatic_including_existing`
- (d) the join and account steps: `invite_lookup` (`TraceCommonsApp/DaemonClient+FirstRun.swift:80`), `account_invite_redeem` and `account_contribution_status` (`DaemonClient.swift:154-157`, called at `AppModel.swift:313-315`), `near_ai_account_sign_in` (`DaemonClient.swift:124`, called at `AppModel.swift:4004`)
- (a) non-Mac device words (W7-D2 (a): the "this computer" variants from #2177, after it lands)

**Tests that pin the Mac:**

- Step order and `can_continue` per step
- Nothing preselected
- Terms sent once

**Decoration in this slice:**

| Item | Mac source | Windows today | Status | Weight | Numbers to pin | Decision |
|---|---|---|---|---|---|---|
| First-run card open: graphic and title give way | TraceCommonsApp/Views/FirstRun/FirstRunFrame.swift:186-196 (graphic and title fade out), :225 (spatial spring 0.76 s) | Legacy onboarding window; no stage, no drawing. | missing | low, S | spatial spring 0.76 s, bounce 0.04 (FirstRunFrame.swift:225, GlassMotion.swift:40) |  |

**Owner decisions:**

- **W7-D1. Wallet sign-in in first run.** The guide says Linux dropped the Mac's wallet sign-in from first run. At 19b42d0d2 the Mac's first run signs in to NEAR AI through the browser, where a wallet can be the method (`TCShellCore/FirstRun/FirstRunPlan.swift:41`); no separate wallet step was found under `Views/FirstRun/`. Which part Linux dropped is unverified here.
  - **Taken 2026-11-18: (a).**
  - (a) Drop the wallet path from first run (Linux precedent, #2177)
  - (b) Keep whatever the Mac's sign-in offers
- **W7-D2. Device words for first run and projects.** First-run and project words exist only as "this Mac" (`first_run_copy.rs:618`, `:634`, `:672`, `:731`; `project_copy.rs:173`).
  - **Taken 2026-11-18: (a), after #2177 lands.**
  - (a) Reuse the "this computer" variants #2177 adds, after it lands (Linux precedent)
  - (b) New "this PC" variants (new words, each needing approval)

### W8. First-run stage

The Mac forces the stage dark (`TraceCommonsAppMain.swift:141`). WinUI can scope a theme to one window with `RequestedTheme` (unverified on the box).

**Mac sources:**

- `Views/FirstRun/FirstRunStage.swift`, `WelcomeScreen.swift`, `ToolsScreen.swift`, `PrivateAIScreen.swift`, `TracesScreen.swift`, `UsesScreen.swift`, `JoinScreen.swift`
- `TCDesign/Logos/GlassBrandMarkDrawing.swift`
- `OnboardingCoordinatorView.swift`

**Windows files:**

- A stage control (Win2D) under the first-run window

**C ABI:**

- None.

**Tests that pin the Mac:**

- Probe values for the spring (0.9 s, bounce 0.04), the ring (radius 130/118, tile 52, hub 64) and the marker check
- The Tools beam travels tile to N (`dashPhase: -flow`, ToolsScreen.swift:574); Private AI dashes use phase -22 per 1.1 s; the test reads the phase sign, not just the dash numbers
- Per-screen frame rates, as the Mac runs them (a single 30 fps stage clock is wrong): Welcome (pen N, words, hub glow, shine) and the Tools scan at display rate; Uses (UsesScreen.swift:431) and Traces (TracesScreen.swift:160) at 60 fps; the backdrop glow, hub breathing, Private AI, Join and the Tools beam after the scan at 30 fps. Ticks only while shown
- The final frame with animations off

**Decoration in this slice:**

| Item | Mac source | Windows today | Status | Weight | Numbers to pin | Decision |
|---|---|---|---|---|---|---|
| Pen-drawn N with shine (Welcome) | TCDesign/Logos/GlassBrandMarkDrawing.swift:71 (Canvas :87, pen tip glow :134); timing in TraceCommonsApp/Views/FirstRun/WelcomeScreen.swift:11-21, timeline :107, scale :162 | Legacy onboarding window; no stage, no drawing. | missing | high, M | draw delay 0.2 s, draw 1 s, shine first 1.5 s then every 15 s, crossing 1.35 s, word delays 1.05/1.14/1.23 s, body 1.45 s, button 1.8 s, rise 0.76 s, hero scale 44/26 (WelcomeScreen.swift:11-21) | G-3 |
| First-run scene ground and breathing glow | TraceCommonsApp/Views/FirstRun/FirstRunStage.swift:120-145 (radial sceneWarm to sceneBase, elliptical glow, breathing ellipse) | Legacy onboarding window; no stage, no drawing. | missing | high, M | breath 7 s each way, 30 fps, blur 30, scale 0.9 + 0.25 phase, opacity 0.6 + 0.4 phase, 640 x 580 (FirstRunStage.swift:132-145); glow strength 0.5 dark / 0.3 light, soft 0.14/0.08 (:116-117); canvas 860 x 760 (:14) |  |
| Step thread with gradient and glow | TraceCommonsApp/Views/FirstRun/FirstRunStage.swift:167 segmentWidth, :191-211 | Legacy onboarding window; no stage, no drawing. | missing | medium, S | segment 38 (:167); timed 0.52 (:198); current glow firstRunGlow @0.35 r 7, 0.3 s delayed 0.42 (:210-211) |  |
| Hub breathing, tool tiles with shadow and lit glow | TraceCommonsApp/Views/FirstRun/FirstRunStage.swift:247-257, :298, :310, :325 | Legacy onboarding window; no stage, no drawing. | missing | medium, M | tile shadow firstRunTileShadow r side*10/52 y side*8/52 (:298); lit glow r side*11/52 (:310) |  |
| Marker check | TraceCommonsApp/Views/FirstRun/FirstRunStage.swift:369 trim, :373 | Legacy onboarding window; no stage, no drawing. | missing | medium, S | draw 0.34 s on, 0.16 s off (:373) |  |
| Spatial spring between steps | TraceCommonsApp/Views/FirstRun/FirstRunStage.swift:517 matchedGeometryEffect; OnboardingCoordinatorView.swift:124 (opacity + offset -10), :131 spatial, :155-173 fades, :215 scale | Legacy onboarding window; no stage, no drawing. | missing | high, S | spring 0.9 s bounce 0.04 (GlassMotion.swift:39-40) |  |
| Tools ring: rotation, scan sweep, found flash (the dashed ring turn, the found flash, the tighten and the respace; the bob, the beam and the rising rows have their own rows below) | TraceCommonsApp/Views/FirstRun/ToolsScreen.swift:340-353 layout, :419-575 (rotation :463, tighten :464-465, angular sweep :478-495, flash :512, Canvas :533, dashed stream :574); FirstRunStage.swift:60-74, :533 | Legacy onboarding window; no stage, no drawing. | missing | high, L | ring turns once per 90 s (:463); radius 130 loose / 118 tight, tile 52, hub 64 (:349-352); scan lead 1 s, turn 2 s, sweep from -36 deg (FirstRunStage.swift:62-64); ring order Claude Code, Gemini CLI, Codex, Cline, OpenCode (:533) |  |
| Tools: list rows rise after the scan (RowRise) | TraceCommonsApp/Views/FirstRun/ToolsScreen.swift:740-751 RowRise (animation :749), applied :604 and :619; list fade :609 | Legacy onboarding window; no stage, no drawing. | missing | medium, S | rows opacity 0 to 1 and y 18 to 0 on the spatial spring, 0.6 s, bounce 0.04, stagger 0.08 x (index + 1) s (:749); list fades over 0.3 s (:609); none under reduce motion | |
| Tools: tiles bob (each tool on its own phase) | ToolsScreen.swift:401-402 ToolsRingLayout.bob, applied :516 | Legacy onboarding window; no stage, no drawing. | missing | low, S | y = -2 + 2 cos(2 pi (t + phase) / 5); phases 0, 1.1, 2.3, 3.4, 4.2 s by tool; display rate during the scan; 0 under reduce motion | |
| Tools: beam from the watched tile into the N | ToolsScreen.swift:433-437 (fade), :533-536 (path tile to N), :574 (dash) | Legacy onboarding window; no stage, no drawing. | missing | medium, S | fades in and out over 0.5 s (:436-437); breath 2.2 s; pulses along dash [0.1, 26], line 8, round caps, 26.1 pt per 1.1 s; `dashPhase: -flow` (:574), so the pulses travel from the tile INTO the N; 30 fps after the scan. A test pins the direction | |
| Uses: commons ring turns | UsesScreen.swift:529-532 commons (`elapsed / 40`) | Legacy onboarding window; no stage, no drawing. | missing | low, S | one turn per 40 s, 60 fps; none under reduce motion | |
| First-run segmented control pill | TraceCommonsApp/Views/FirstRun/FirstRunStage.swift:443 FirstRunSegmented | Legacy onboarding window; no stage, no drawing. | missing | low, S | chosen-pill fill moves over 0.15 s on the token curve (0.2, 0.8, 0.2, 1). Used by the W7 steps; the toolbar's GlassSegmentedTabs is a different control | |
| Private AI diagram (flowing dashes, counted pill) | TraceCommonsApp/Views/FirstRun/PrivateAIScreen.swift:84-117, :144 | Legacy onboarding window; no stage, no drawing. | missing | medium, M | 30 fps, flow period 1.1 s, dash 5/6 line 2 in firstRunGlow (:85-117) |  |
| Traces step: orbs streaming into a filling ring | TraceCommonsApp/Views/FirstRun/TracesScreen.swift:91-123, :160, :193-208, :285 | Legacy onboarding window; no stage, no drawing. | missing | medium, M | count rises over 1.9 s (doc :91); 60 fps (:160); glow @0.35 r 6 (:196); spatial 0.64 (:285) |  |
| Uses flow: tools folding in, review gate, pulse | TraceCommonsApp/Views/FirstRun/UsesScreen.swift:419-582, :645-655 | Legacy onboarding window; no stage, no drawing. | missing | medium, M | 60 fps (:431); tile shadow r 10 y 8 (:494); pulse blur 3 (:558); expressive 0.46 (:582) |  |
| Join graphic: tools circling the N on a turning dashed ring | TraceCommonsApp/Views/FirstRun/JoinScreen.swift:521-545, :688 | Legacy onboarding window; no stage, no drawing. | missing | medium, M | 30 fps; ring turns once per 90 s (:531); pulse scale 0.3 + 1.6 p, blur 4 (:542-543) |  |
| Notice mark and its symbol swap | TCDesign/Components/PopoverNotice.swift:116 GlassNoticeMark (side 30 :143), :154 contentTransition symbolEffect replace; MonitorNotices.swift:844, :882 bounce | animation greps 0 (Z1) | missing | medium, S | mark side 30 (PopoverNotice.swift:143) |  |
| Arming offer: bolt to check, spreading ring, hold and exit | TraceCommonsApp/Views/Monitor/ArmingOfferMotion.swift:18, 37, 42, 66, 72; AppModel.swift:2197, 2221; MonitorNotices.swift:882-884 | the offer is a text card (MainWindow.xaml:753, ViewModel.HasArmingOffer) with no mark and no motion; animation greps 0 (Z1) | missing | medium, S | hold 900 ms (:18); exit spring 0.42/0.9 (:37); mark spring 0.32/0.7 (:42); ring 1 to 1.9, easeOut 0.7 s (:66, :72) |  |

**Owner decisions:**

- Also applies: G-3 (the mark the stage draws).

### W9. Consent

Gated by spike S5. Nothing more permissive than the Mac.

**Mac sources:**

- `Views/Settings/SharingChoiceCard.swift`, `ConsentSection.swift`, `ProjectsSection.swift`
- `Views/Monitor/SessionReviewCard.swift`, `TracesOffers.swift`, `ArmingOfferMotion.swift`
- `TCShellCore/AutomaticGrantCopy.swift`, `ContributionModeCopy.swift`, `Flow1.swift`, `ProjectArmingCopy.swift`
- `Views/PreviewSheet.swift`

**Windows files:**

- `Controls/SettingsView.xaml` (consent scopes `:214-259`; the undo window `:279` is removed, W9-D1 (a))
- `MainWindow.xaml.cs:896-908` (arming accept)
- `Controls/PreviewSheet.xaml`, `PreviewWindow.cs`, `Controls/WithdrawDialog.cs`
- `MainViewModel` undo

**C ABI:**

- (b) `tc_automatic_grant_copy_json`, `tc_automatic_contribution_copy_json`, `tc_contribution_mode_copy_json`, `tc_contribution_override_confirm_json`, `tc_contribution_override_refusal_text`, `tc_flow1_grant_request_json`, `tc_project_ignore_reconciled_text`, `tc_withdrawal_confirmation_prompt_text`
- (d) `automatic_grant`, `grant_automatic_including_existing`, `withdraw_automatic_grant`, `set_contribution_override`, `clear_contribution_override`, `plan_project_modes`, `set_project_modes`
- (a) a non-Mac `backlog_rule` (`project_copy.rs:173`), with W7-D2

**Tests that pin the Mac:**

- Each daemon reply settles against the action pressed
- A reply that does not confirm the write leaves the old state
- Bulk plan, confirm and stale
- Never is absolute
- No `ApprovalHoldSeconds` control remains in Settings

**Decoration in this slice:**

No decoration items.

**Owner decisions:**

- **W9-D1. Undo-window setting.** `ApprovalHoldSeconds` (`Controls/SettingsView.xaml:279`) exists on Windows only and can be set low.
  - **Taken 2026-11-18: (a).**
  - (a) Remove it (Linux precedent, #2179)
  - (b) Keep it
- **W9-D2. Arming without an account grant.** Windows arms a folder by sending `set_project_mode` `auto_upload` from the arming card and never sends the automatic-grant family.
  - **Taken 2026-11-18: (a). Spike S5 still gates W9 shipping.**
  - (a) Match the Mac's grant flow before W9 ships (Linux precedent: "match the mac", #2179)
  - (b) Keep per-project arming as it is
  - Note: Spike S5 traces what the daemon then sends. It was meant to run before this was asked; with (a) taken, it still gates W9 shipping.
- **W9-D3. Where an override is cleared.** The Mac clears a contribution override from its menu-bar panel (`TraceCommonsApp/Views/Monitor/MenuPanelStore.swift:177`, `:199`), after a confirmation. Windows sends neither `set_contribution_override` nor `clear_contribution_override`. Whether the Mac offers a clear anywhere else was not checked.
  - **Taken 2026-11-18: (a).**
  - (a) Clear only where the Mac clears, so it arrives with the tray flyout (W13-D1) (Linux precedent: "match the mac", #2179)
  - (b) Also add a clear control in Settings

### W10. Brand

"Near.ai" appears 25 times in 14 files under `windows/src` and 4 times in `windows/packaging/Package.appxmanifest`. The fix reads the name from the core.

**Mac sources:**

- `crates/trace-commons-contributor/src/brand.rs:28-35`

**Windows files:**

- Every "Near.ai" literal, including `MainWindow.xaml:29`, `:97`, `OnboardingWindow.xaml:27`, `:33`, `Controls/SettingsView.xaml:208`
- `windows/packaging/Package.appxmanifest:32`, `:75`, `:92`, `:109`
- The MSIX tiles in `windows/packaging/Assets`, redrawn with the NEAR AI N (G-3 (a), tiles too)

**C ABI:**

- (a) a new export (or copy field) returning `APP_NAME`, in both header copies, with the Swift export contract updated

**Tests that pin the Mac:**

- The manifest's app-name display names (`Package.appxmanifest:32` `<DisplayName>`, `:75` `DisplayName`, `:109` the startup task's `DisplayName`) equal `APP_NAME`. The invite protocol's `<uap:DisplayName>Near.ai invite</uap:DisplayName>` (`:92`) is not the app name and is excluded until W10-D2 is answered.
- No "Near.ai" literal in shell source
- `NoCommonsNameTests` stays green

**Decoration in this slice:**

No decoration items.

**Owner decisions:**

- **W10-D2. The invite protocol's display name.** `Package.appxmanifest:92` is "Near.ai invite", the name Windows shows for the `tracecommons` protocol. `APP_NAME` is "Near AI Gateway" (`brand.rs:28-35`).
  - **Taken 2026-11-19: (a).**
  - (a) `APP_NAME` alone, no new words
  - (b) A core word such as `<APP_NAME> invite` (a new copy item, needing approval)
  - (c) Leave the literal as it is for now, excluded from the test
- **W10-D1. Internal ids.** The package name `Iqlusion.TraceCommons`, the `tracecommons` protocol, `TraceCommons.exe`, the Run value name and the install folder.
  - **Taken 2026-11-18: (a).**
  - (a) Keep all of them (Linux precedent, #2184)
  - (b) Rename some, with a migration

### W11. Home

**Mac sources:**

- `Views/Monitor/HomeViews.swift`, `HomeStore.swift`, `HomeInsightsVisual.swift`, `BalanceArrivalFlourish.swift`, `InsightsGlanceCard.swift`
- `TCDesign/Components/HeroCard.swift`, `PixelGlyph.swift`, `PixelField.swift`, `CoinBurst.swift`

**Windows files:**

- A new Home tab

**C ABI:**

- (b) `tc_nudge_copy_json`, `tc_nudge_digest_help_json`, `tc_nudge_entry_tags_json`, `tc_private_ai_campaign_json`
- (d) `commons_credit_summary`, `insights_glance`, `activity_missions_catalogue`, `activity_missions_status`, `mission_catalogue`, `model_spend`, `subscription_usage`

**Tests that pin the Mac:**

- Hero grounds from tokens (#081538, #07152F, #0A2546)
- Pixel masks pinned from the probe; 10 fps only while on screen
- Coin burst: 44 coins, 1.7 s; count-up 24 frames of 50 ms

**Decoration in this slice:**

| Item | Mac source | Windows today | Status | Weight | Numbers to pin | Decision |
|---|---|---|---|---|---|---|
| Hero cards on solid navy grounds | TCDesign/Components/HeroCard.swift:16 GlassHeroCard (sizes :75-82, hover :114-115, close :199); HomeViews.swift:210; tokens GlassTokens.swift:405-409 | No Home tab. | missing | high, M | heroCampaignCard #081538, heroMissions #07152F, heroInsights #0A2546, both stops equal (GlassTokens.swift:405-409); min height 156, graphic reserve 90, graphic width 210 (HeroCard.swift:75-79) | W11-D2 |
| Hero pixel glyph | TCDesign/Components/HeroCard.swift:229 GlassHeroSymbol (:251 rotation, :258-261); TCDesign/Components/PixelGlyph.swift:17 (timeline :57, Canvas :58) | No Home tab. | missing | high, M | 190 tall, bleed 46 x 34, angle -12 (HeroCard.swift:258-261); 10 fps (PixelField.swift:51); Bayer 4x4 (PixelField.swift:69) | W11-D1 |
| Pixel field (corner glow and waves) | TCDesign/Components/PixelField.swift:19 (timeline :54, Canvas :55, Bayer :69); GetStartedCard.swift:18 | No Home tab. | missing | medium, S | 10 fps (PixelField.swift:51) |  |
| Coin burst on balance arrival | TCDesign/Components/CoinBurst.swift:14 (timeline :135, Canvas :136); BalanceArrivalFlourish.swift:48-53 | No Home tab. | missing | medium, M | 44 coins, 1.7 s, spill 120, still at 0.45, drag 3.4, gravity 50, squash 0.72, fade from 0.55, flash 0.28 s r 46, golds #FFE58A/#F5B921/#C58A12/#8F5E0A (CoinBurst.swift:30-56) |  |
| Balance count-up | TraceCommonsApp/Views/Monitor/BalanceArrivalFlourish.swift:36-39; HomeViews.swift:1821 | No Home tab. | missing | medium, S | 24 frames x 50 ms, reduce-motion hold 1.6 s (BalanceArrivalFlourish.swift:36-39) |  |
| Home Insights graphic (bars with fade and shadow) | TraceCommonsApp/Views/Monitor/HomeInsightsVisual.swift:16, :76 gradient, :83 shadow | No Home tab. | missing | medium, S | ink 0.95 to 0.45 gradient (:76); shadow black @0.2 r 12 y 8 (:83) |  |
| Remote campaign image on the campaign hero | TCShellCore/PrivateAICampaign.swift:13 imageURL; PrivateAICampaignClient.swift:42; PrivateAICampaignStore.swift:77; HomeViews.swift:140 | No Home tab. | missing | low, M |  | W11-D3 (a): not ported; the compiled campaign only |

**Owner decisions:**

- **W11-D1. Art where the Mac samples an SF Symbol.** Hero cards draw pixel art sampled from SF Symbols.
  - **Taken 2026-11-18: (a).**
  - (a) Hand-drawn pixel art from own geometry (Linux precedent, #2187)
  - (b) Leave those cards without art
- **W11-D2. Walkthrough card.** A closable card on Home.
  - **Taken 2026-11-18: (a).**
  - (a) Leave it out (Linux precedent, #2187)
  - (b) Port it, under the notice rule
- **W11-D3. Remote campaign feed and image.** The Mac fetches a campaign feed and its image.
  - **Taken 2026-11-18: (a).**
  - (a) The compiled campaign only (Linux precedent, #2187: GTK has no HTTP client)
  - (b) Fetch the feed and image as the Mac does

### W12. Tray mark

**Mac sources:**

- `TCDesign/Components/MenuBarPanel.swift:316` (`GlassMenuBarStrip`)
- `TCShellCore/MenuBarStatus.swift`, `NudgeSurface.swift`

**Windows files:**

- `TrayIcon.cs`, `Interop/TrayModel.cs`, `Interop/MarkRaster.cs`

**C ABI:**

- (b) `tc_private_inference_menu_word`, `tc_private_inference_menu_tone`
- (d) `set_menu_bar_mark_enabled`

**Tests that pin the Mac:**

- Badge cap 99+, never red
- Ring and halo only from `status.nudge.mark`; halo inset 2.5, line 2
- One raster per DPI

**Decoration in this slice:**

| Item | Mac source | Windows today | Status | Weight | Numbers to pin | Decision |
|---|---|---|---|---|---|---|
| Menu-bar strip: N plus 24 h bars on a centre line | TCDesign/Components/MenuBarPanel.swift:316 GlassMenuBarStrip (doc :297-315, Canvas :532, cut-out :575); 10 refs in the app | Turn raster tray icon; dot for Attention and Unhealthy only (`TrayIcon.cs:625-647`); count in the tooltip. | missing | high, M | height 22, mark 14, gap 4, bar 1, step gap 1, graph 17, centre line 1 (MenuBarPanel.swift:475-493); bars only once a few days of data exist (doc :307-308) | W12-D1 (a): the mark ports, the bars are not ported by decision |
| Tray badge (count, 99+, figure cut out of the mark ink) | TCDesign/Components/MenuBarPanel.swift:302-310 (doc), :460-464 | Turn raster tray icon; dot for Attention and Unhealthy only (`TrayIcon.cs:625-647`); count in the tooltip. | missing | high, S | badge 11, glyph 7, inset 2.5, lift = halo reach (MenuBarPanel.swift:460-463); 99+ past 99 (doc :303-304); never red (doc :309-310) | W12-D2 |
| Tray news ring and idle halo | TCDesign/Components/MenuBarPanel.swift:338-346 (Mark), :389 shownMark, :470-472 | Turn raster tray icon; dot for Attention and Unhealthy only (`TrayIcon.cs:625-647`); count in the tooltip. | missing | medium, S | halo inset 2.5, line 2, reach = inset + line/2 (MenuBarPanel.swift:470-472) | W12-D3 |
| Tray attention mark | TCDesign/Components/MenuBarPanel.swift:319-334 Condition (attention, unavailable) | Turn raster tray icon; dot for Attention and Unhealthy only (`TrayIcon.cs:625-647`); count in the tooltip. Classic purple, not the flat blue. | partial | medium, S |  |  |
| Credit arrival: tray icon gleam, mark hop and mini coins | TCDesign/Components/MenuBarPanel.swift creditGleam, markHop, stripCoins; TraceCommonsApp/Views/Monitor/MenuBarGlassPanel.swift:661-712 (at origin/main 7e020ffe0, #2197) | Turn raster tray icon; static (`TrayIcon.cs:625-647`). | missing | medium, M | gleam up 0.15 s, held to 0.7 s, fades out by 1.7 s; mark hops 2 pt over 0.35 s; 10 mini coins (drag 3.8, gravity 10); 24 fps; the switch is on by default; no motion under reduce motion or with the switch off. Drawn as a per-frame raster swap of the tray icon | W12-D4 |
| Credit arrival: taskbar icon coin burst | TraceCommonsApp/DockCreditAnimator.swift; TCDesign/Components/DockCoinBurst.swift (at origin/main 7e020ffe0, #2197) | None. | missing | medium, M | 128 pt tile, 24 coins, drag 3.8, gravity 32, flash 0.32 s radius 56, icon pulse +4.5%; 1.7 s at 24 fps; a still frame at the end. On Windows an overlay on the taskbar button (overlay icon or thumbnail toolbar surface; spike to pick) | W12-D4 |
| Credit arrival: taskbar attention flash | TraceCommonsApp/DockCreditAnimator.swift:39 and :99-101 `requestUserAttention(.informationalRequest)` | None. | missing | low, S | one flash while the window is not active (the Mac's single informational bounce): FlashWindowEx with a count of one; never while the app is active. Flags picked in the spike | W12-D4 |

**Owner decisions:**

- **W12-D1. Tray icon content.** The Mac's menu-bar strip is the mark plus 24 h bars; a Windows tray icon is a small square.
  - **Taken 2026-11-18: (a).**
  - (a) Mark and badge only
  - (b) Try the bars in the tray icon
  - Note: Linux precedent (#2186) ported the mark, badge, ring and halo; whether it drew the bars is not recorded here.
- **W12-D2. Tray badge.** Windows shows a dot and puts the count in the tooltip.
  - **Taken 2026-11-18: (a).**
  - (a) The Mac's badge, 99+ cap, figure cut from the mark ink, never red (Linux precedent, #2186)
  - (b) Keep the dot and tooltip count
  - Note: the earlier Windows reason against a badge is recorded at `TrayIcon.cs:629-633`: "a 16px tray icon cannot carry two legible digits, so the count lives in the tooltip". The Mac's badge is 11 pt on a 22 pt item (`MenuBarPanel.swift:460`, `:475`). At 100% scaling a 16 px icon gives the badge about 8 px. Proposed, not yet agreed: W12 also keeps the count in the tooltip, and its gallery rows show the badge at 100%, 150% and 200% scaling so the owner sees the trade-off.
- **W12-D4. Credit-arrival motion.** The Mac animates a credit arriving on the Dock icon (coin burst, bounce) and in the menu-bar strip (gleam, mark hop, mini coins), behind one Settings switch (`GeneralSection.swift:39`).
  - **Taken 2026-11-18: tray animation plus taskbar flash.** Three things on Windows, all gated by the one Settings switch (W17): the tray icon's credit gleam, mark hop and mini coins; `FlashWindowEx` for the Mac's `requestUserAttention`; and a coin-burst overlay on the taskbar icon with the Dock's numbers (24 coins, drag 3.8, gravity 32, flash 0.32 s radius 56, pulse +4.5%, 24 fps, 1.7 s).
  - Note: a credit arrival is not a new notice: no banner, card, toast or text is added. The copy guide's rule 9 (no new notices) is untouched; the switch's label comes from the core words.
- **W12-D3. Tray news ring and idle halo.**
  - **Taken 2026-11-18: (a).**
  - (a) Port them with the Mac's conditions (Linux precedent, #2186)
  - (b) Badge only
- Also applies: G-3 (the mark in the tray).

### W13. Tray flyout and edge rail

**Mac sources:**

- `Views/Monitor/MenuBarGlassPanel.swift`, `MenuPanelStore.swift`
- `Views/EdgeRail/EdgeRailController.swift`, `EdgeRailView.swift`, `TCDesign/Components/EdgeRail.swift`, `TCShellCore/EdgeRail.swift:6-15`

**Windows files:**

- A topmost popup window from the tray click
- An edge window (off by default, as on the Mac)

**C ABI:**

- (b) `tc_private_inference_runtime_word`
- (c) edge-rail words from the monitor screens table
- (d) `set_contribution_override`, `clear_contribution_override` (listed under W9): the override clear arrives here (W9-D3 (a)), from the flyout, after the Mac's confirmation (`MenuPanelStore.swift:177`, `:199`)

**Tests that pin the Mac:**

- Rail geometry from the probe (#2188 pinned (1436, 200, 484, 680) on 1920x1080)

**Decoration in this slice:**

| Item | Mac source | Windows today | Status | Weight | Numbers to pin | Decision |
|---|---|---|---|---|---|---|
| NEAR AI wordmark | TCDesign/Logos/GlassNearWordmark.swift:14 (Canvas :80); used at TraceCommonsApp/Views/Monitor/MenuBarGlassPanel.swift:73 | no popover surface (native popup menu, see tray items); title bar text is the literal "Near.ai" (MainWindow.xaml:97) | missing | medium, S |  | W13-D1 |
| NEAR N tile and Traces nodes glyph (edge rail icons) | TCDesign/Logos/GlassNearWordmark.swift:76 GlassNearMark (EdgeRailView.swift:114); TCDesign/Logos/GlassTraceNodesGlyph.swift:7 (Canvas :27; EdgeRailView.swift:144) | grep IsAlwaysOnTop\|HWND_TOPMOST\|TOPMOST\|OverlappedPresenter over windows/src: 0 lines | missing | low, S |  | W13-D1 |
| Menu-bar glass popover | TraceCommonsApp/Views/Monitor/MenuBarGlassPanel.swift:1-10 (doc), glass lookup :427-449 | tray click opens a native popup menu: CreatePopupMenu (TrayIcon.cs:418, 474), TrackPopupMenuEx (TrayIcon.cs:531) | missing | high, L |  | W13-D1 |
| Edge rail and its slide | TraceCommonsApp/Views/EdgeRail/EdgeRailView.swift:54 (move+opacity), :59/:148 hover; EdgeRailController.swift:166; TCDesign/Components/EdgeRail.swift:8 geometry, :31 GlassRailTile | grep IsAlwaysOnTop\|HWND_TOPMOST\|TOPMOST\|OverlappedPresenter over windows/src: 0 lines; animation greps 0 (Z1) | missing | medium, L |  | W13-D1 |

**Owner decisions:**

- **W13-D1. Tray flyout and edge rail.** The Mac has a glass popover from the menu bar and an edge rail (off by default).
  - **Taken 2026-11-18: (a).**
  - (a) Both (Linux precedent: "approve both", #2188 open)
  - (b) The flyout only
  - (c) Neither; keep the native popup menu
- Also applies: W9-D3 (the override clear lives in the Mac's menu-bar panel).

### W14. Terminals and launch

**Mac sources:**

- `Views/ManagedSessionsView.swift`, `Views/Settings/ManagedTerminalPicker.swift`, `ManagedFolderOptions.swift`, `ManagedFolderExplainer.swift`

**Windows files:**

- `Controls/ManagedSessionsView.cs`, `ManagedTerminalPicker.cs`
- Rust `managed/terminal.rs:73` for any new terminal

**C ABI:**

- (c) `tc_managed_copy_json`, `tc_managed_tools_json` (bound, never called)
- (d) `managed_session_focus`, `managed_session_resume`, `managed_session_stop`, `managed_session_start_again`, `managed_cli_model`, `managed_model_catalogue`

**Tests that pin the Mac:**

- A hostile folder name arrives intact in argv
- A real launch on the box

**Decoration in this slice:**

| Item | Mac source | Windows today | Status | Weight | Numbers to pin | Decision |
|---|---|---|---|---|---|---|
| Launch art: folder slides into the terminal (managed folder handoff) | TraceCommonsApp/Views/ManagedFolderExplainer.swift:68, :166 tool logo, :182-184, :211, :221 | the managed sessions view builds plain text Buttons (Controls/ManagedSessionsView.cs:78-82) and one Launch button (:133); animation greps 0 (Z1) | missing | medium, S | 1.2 s on the token curve (:184, :221); rest scale 0.9 (:211) |  |
| Per-tool launch buttons with logos | TraceCommonsApp/Views/ManagedSessionsView.swift:155 ManagedToolButton | one Launch text button for all tools (Controls/ManagedSessionsView.cs:133) | missing | low, M |  | W14-D3 |

**Owner decisions:**

- **W14-D1. Terminals.** Windows offers Windows Terminal only (`terminal.rs:73`).
  - **Taken 2026-11-18: (c).** **Taken 2026-11-19: (b), replacing (c): Windows Terminal, with the PowerShell console host as the fallback when Windows Terminal is not installed. Both must pass the argv check. No other terminals.**
  - (a) Windows Terminal only
  - (b) Add the PowerShell console host
  - (c) Add others after an argv check
  - Note: Linux precedent: four terminals added (#2170), each from an argv vector.
- **W14-D2. "Show" (raise a running session).**
  - **Taken 2026-11-18: (a).** **Taken 2026-11-19: ship "Show" only if its spike finds a reliable way to raise the session window that never steals focus unexpectedly; otherwise it stays Mac-only (b). Gated on the spike.**
  - (a) Spike it on Windows
  - (b) Leave it Mac-only (Linux precedent: Wayland gave no safe way)
- **W14-D3. Per-tool launch buttons.** The Mac has a launch button per tool, with its logo. That is a new per-tool start flow, not only decoration.
  - **Taken 2026-11-18: (a).**
  - (a) Port it with the per-tool flow
  - (b) Leave it out (Linux precedent, #2187)

### W15. Background service, only if wanted

**Mac sources:**

- `TraceCommonsApp/DaemonHost.swift`, `TCBridge/TCAttach.swift`

**Windows files:**

- `DaemonHost.cs`, `RunAtLogin.cs`, `StartupRegistration.cs`

**C ABI:**

- (b) `tc_daemon_attach`, `tc_attach_copy`, `tc_subscribe_with_accepts`
- (b) `tc_resolve_state_dir`, `tc_state_dir_override_refused_line` (Mac: `TraceCommonsApp/DaemonHost.swift`, `TCShellCore/StateDirectory.swift`)
- (a) service dialog words, which are not on main

**Tests that pin the Mac:**

- One watcher per login, if S4 shows two daemons can collide
- The named-pipe ACL step still refuses a second account

**Decoration in this slice:**

No decoration items.

**Owner decisions:**

- **W15-D1. Background service.** The Windows app is not sandboxed and hosts the daemon in-process.
  - **Taken 2026-11-19: (a), unless spike S4 shows two daemons collide.** Gated on S4.
  - (a) No background-service flow on Windows
  - (b) A CLI-owned daemon the app attaches to, with dialogs approved for Windows
  - Note: Linux precedent: (b), "Flatpak = host daemon" (#2171), forced by the sandbox. Spike S4 runs first.

### Not applicable at 19b42d0d2

The Mac does not ship these, so they are not ported. Listed so a later Mac change is noticed.

| Item | Mac source | Why not applicable |
|---|---|---|
| Live Trace mark (gradient lattice) | TCDesign/Logos/GlassTraceMark.swift:15 | Mac does not ship it at 19b42d0d2: 0 references outside its own file and the gallery |
| Hero radial light | removed from HeroCard.swift by fc4ecc1c6 (2026-11-07); no RadialGradient in HeroCard.swift at 19b42d0d2 | Mac does not ship it at 19b42d0d2 |
| Home stat tile icons and tilted watermark | TraceCommonsApp/Views/Monitor/HomeViews.swift:549 ("No icon (owner, 2026-11-07: its watermark ...") | Mac does not ship them at 19b42d0d2 |
| Promo treatments and glow edge | TCDesign/Styling/GlassPromo.swift:11-320 (GlassPromoCard :57, GlassGlowEdge :253) | not reached at 19b42d0d2: GlassIconCard forwards promo (Containers.swift:284, :415) but homeCard defaults it to nil (HomeViews.swift:306) and no caller passes one; GlassGlowEdge has 0 refs outside the gallery |
| Unused indicator components | TCDesign/Components/MenuBarPanel.swift:34 GlassStatePill, :207 GlassDayGraph; Navigation.swift:514 GlassStepProgress | 0 references outside their own files and the gallery at 19b42d0d2 |


## Slices for the surfaces W1-W15 did not cover

G-4 (a) is taken: these Mac surfaces get slices W16 to W25. The W0 survey
listed them as "Surfaces no slice covers". Their decisions were taken
2026-11-19: each takes option (a), the one that matches the Mac, and the
other options are kept for the record.

Every slice from W2 on also carries the keyboard and accessibility-name
acceptance line: every control it adds is reachable by keyboard and named
from core words. W25 audits the whole app against it at the end.

### W16. Settings: account, devices, payouts and referrals

The Account group of the Mac's Settings (`Views/SettingsSections.swift:12-79`:
account, payouts, referrals). Lands in the Settings window from W2 (W2-D2 (a)).

Five of the Mac's 19 sections have no slice of their own: watching, watched
folders, public profile, changes and witness. Windows has counterparts for
some today (the witness at `Controls/SettingsView.xaml:814`, the public
profile through `get_public_profile`, the session roots window); whether
they match the Mac is unverified. They move into the Settings window with
W2. W16-D1 (a) would put them in the Mac's order; porting their Mac layout
is still to be asked.

**Mac sources:**

- `Views/Settings/AccountSection.swift` (set-up-again copy `:156-359`), `AccountLoginChangeViews.swift`
- `Views/AccountDevicesView.swift`, `TCShellCore/AccountDevicesSurface.swift`
- `Views/PayoutLinkView.swift`, `Views/Settings/PayoutsSection.swift`, `TCShellCore/PayoutLinkSurface.swift`
- `Views/Settings/ReferralsSection.swift`, `TCShellCore/ReferralSurface.swift`
- `TraceCommonsApp/DaemonClient+NativeIdentity.swift` (account session, sign-in, sign-out)

**Windows files:**

- `Controls/SettingsView.xaml` account area. Its comment at `:168-171` says "This shell has no payouts section"; only the "payouts need a NEAR AI login" line exists (`:179`).
- `ViewModels/ContributorSettingsViewModel.cs`, `Controls/AccountLoginChangeDialog.cs`, `Interop/AccountLoginChange.cs`
- New interop surfaces for devices, payouts and referrals in `Interop/`

**C ABI:**

- (b) `tc_account_devices_copy_json`, `tc_payout_link_copy_json`, `tc_referral_copy_json`, `tc_referral_code_display`, `tc_set_up_again_copy_json`
- (d) `account_devices_list`, `account_device_code_mint`, `account_device_code_status`, `account_device_rename`, `account_device_revoke`
- (d) `payout_link_start`, `payout_link_confirm`, `payout_link_orgs`, `payout_link_unlink`, `payout_rails_status`
- (d) `referral_status`, `referral_apply`, `referral_codes_list`, `referral_codes_create`, `referral_codes_disable`
- (d) `account_session_status`, `account_sign_in`, `account_sign_out`

**Tests that pin the Mac:**

- Section order and the six groups from `SettingsSections.swift:12-79`
- Payouts and Referrals are not drawn before onboarding (`availableBeforeOnboarding`)
- Each daemon reply settles against the action pressed; a refused write leaves the old state
- A failed read shows a dash, never 0

**Decoration in this slice:**

No decoration items.

**Owner decisions:**

- **W16-D1. Settings section list.** The Mac lists 19 sections in 6 groups, with a hairline between groups and no group headings.
  - **Taken 2026-11-19: (a).**
  - (a) The Mac's list, order and groups
  - (b) Windows' current Settings layout, with the missing sections added
  - Note: No Linux precedent recorded here.
- **W16-D2. Payouts and referrals.** Windows has neither section today.
  - **Taken 2026-11-19: (a).**
  - (a) Both, as on the Mac
  - (b) Payouts only
  - (c) Neither yet; the existing "payouts need a NEAR AI login" line stays
- **W16-D3. Device codes.** The Mac mints a code to add another device and lists, renames and revokes devices.
  - **Taken 2026-11-19: (a).**
  - (a) All of it, as on the Mac
  - (b) List, rename and revoke only; no minting from Windows

### W17. Settings: tools, saved accounts, Insights switches, notifications, startup, updates and general

The Capture, Private AI and App groups of the Mac's Settings, less what W9
(consent, projects) and W19 (Private AI) carry.

**Mac sources:**

- `Views/Settings/ToolsSection.swift`, `CliToolRow.swift`, `ManagedFavoritesCard.swift`, `ManagedCLIOptionsSection.swift`
- `Views/Settings/SavedAccountsSection.swift`
- `Views/Settings/InsightsSettingsCard.swift`, `NudgeSettingsSection.swift`, `SubscriptionUsageSettingsSection.swift`
- `Views/Settings/StartupSection.swift` (`NotificationsSection` `:74`, `UpdatesSection` `:180`)
- `Views/Settings/GeneralSection.swift` (appearance, the edge rail switch, the map style, promotions and the credit-animation switch `:39`; doc `:5-9`)

**Windows files:**

- `Controls/SettingsView.xaml` (start at sign-in `:208`, the notification interval `:293`), `ViewModels/ContributorSettingsViewModel.cs`
- `Controls/ManagedSessionsView.cs` (already sends `managed_account_add`, `_remove`, `_rename`, `_reconnect`, `_set_key`)
- `AppUpdater.cs`, `RunAtLogin.cs`, `StartupRegistration.cs`

**C ABI:**

- (b) `tc_cli_tool_json` (shared with W7)
- (a) appearance words: the Mac's "Appearance" is a Swift literal (`Views/SettingsSections.swift:193`), and `appearance_copy` is not on main
- (d) `managed_cli_options_set`, `managed_folder_settings_get`, `managed_folder_settings_reset`, `managed_model_favorite_set`, `set_managed_herdr` (Mac: `AppModel.swift`)
- (d) `managed_account_verify` (Mac: `SavedAccountsSection.swift`)
- (d) `set_notifications_enabled`, `set_notify_kind`, `set_suggestions_enabled` (Mac: `TCShellCore/DataContract/LiveDaemonClient.swift`)
- Shared: `subscription_usage` (W11), `set_menu_bar_mark_enabled` (W12), the map style switch (W6-D1 (a)), the edge rail switch (W13)

**Tests that pin the Mac:**

- Each switch writes, then reads back; a refused write leaves the old state
- The appearance choice applies at once and is kept for the next launch
- `ShellWordingTests` baseline goes down

**Decoration in this slice:**

| Item | Mac source | Windows today | Status | Weight | Numbers to pin | Decision |
|---|---|---|---|---|---|---|
| Credit-animation switch (General) | TraceCommonsApp/Views/Settings/GeneralSection.swift:39 | None. | missing | low, S | one switch gates the tray gleam, hop and mini coins, the taskbar flash and the taskbar coin burst (W12-D4); the switch itself moves on the Mac's switch motion (easeOut 0.22 s, W5); off or reduce motion shows the still frame; its words come from the core | W12-D4 |

**Owner decisions:**

- **W17-D1. Appearance choice.** The Mac offers Light, Dark or the system's. Its words are not in the core.
  - **Taken 2026-11-19: (a).**
  - (a) Port it, after core words are added (new words, each needing approval)
  - (b) Follow the system only
- **W17-D2. Notification kinds.** The Mac switches notifications per kind (`set_notify_kind`). Windows has one interval setting. A kind Windows does not notify today is a new notice on Windows.
  - **Taken 2026-11-19: (a).**
  - (a) Port the switches; each kind new to Windows is asked separately
  - (b) Keep Windows' interval setting only
- **W17-D3. Updates section.** Windows has its own updater (`AppUpdater.cs`) and the update InfoBar (`MainWindow.xaml:397-411`).
  - **Taken 2026-11-19: (a).**
  - (a) The Mac's Updates section and words, driving the Windows updater
  - (b) Keep Windows' update surface as it is
- **W17-D4. Managed CLI options and `herdr`.** What `set_managed_herdr` controls on Windows is unverified.
  - **Taken 2026-11-19: (a).**
  - (a) Port the CLI options; `herdr` only after a check that it applies on Windows
  - (b) Leave the managed CLI options Mac-only for now

### W18. Settings: compute

The Mac's Compute section. The Settings window draws it: `SettingsModal`
is the body of the Settings window, "Compute included"
(`Views/Monitor/SettingsModal.swift:5-13`), and draws `ComputeView` itself
(`:211`). The shared section body draws nothing for `.compute`
(`Views/Settings/GlassSettingsContent.swift:36`, `case .compute: EmptyView()`).

**Mac sources:**

- `Views/ComputeView.swift`, `TCBridge/TCCompute.swift`
- `Views/Monitor/SettingsModal.swift:211`

**Windows files:**

- A new compute section in the Settings window
- A compute surface in `Interop/`

**C ABI:**

- (b) the nine `tc_compute_*` exports: `tc_compute_open`, `tc_compute_open_local`, `tc_compute_status_json`, `tc_compute_command_json`, `tc_compute_copy_json`, `tc_compute_resource_begin_json`, `tc_compute_resource_event_json`, `tc_compute_shutdown`, `tc_compute_free`

**Tests that pin the Mac:**

- The allowance field accepts only a whole number above zero (`ComputeView.swift` doc)
- The handle from `tc_compute_open` or `tc_compute_open_local` is freed with `tc_compute_free` when its owner goes away, as `TCBridge/TCCompute.swift` does (`deinit` `:54`, free `:159`)

**Decoration in this slice:**

No decoration items.

**Owner decisions:**

- **W18-D1. Compute on Windows.**
  - **Taken 2026-11-19: (a).**
  - (a) Port the section as on the Mac
  - (b) Leave it out until a Windows compute use is asked for
  - Note: No Linux precedent recorded here.

### W19. The Private AI tab body

The Inference tab. W3 carries the Private AI map view; this slice carries the
tab body and its inspector.

**Mac sources:**

- `Views/Monitor/InferenceViews.swift` (`InferenceTabView` `:18`, `PrivateAIGuideCard` `:386`, `PrivateAIInspectorView` `:419`), `InferenceStore.swift`, `InferenceAccount.swift`
- `Views/Monitor/SpendOrganizationPrompt.swift`, `Views/Settings/SpendOrganizationCard.swift`, `Views/Settings/PrivateAISection.swift`
- `TCShellCore/CredentialSurface.swift` (`near_ai_credential_migrate` `:171`)
- `AppModel.swift:2447` (`probe_routing`)

**Windows files:**

- `Controls/PrivateInferenceView.xaml`, `ViewModels/PrivateInferenceViewModel.cs`
- `Interop/PrivateInferenceCopy.cs`, `NearAiCredentialSurface.cs`, `NearAiBalanceSurface.cs`, `NearAiFunding.cs`
- The routing override in `Controls/SettingsView.xaml` (`:419-496`)

**C ABI:**

- (b) `tc_inference_connection_copy_json`
- (d) `inference_summary`, `inference_activity`, `inference_calls`, `inference_call_proof`
- (d) `near_ai_spend_organizations`, `near_ai_spend_organization_select`, `near_ai_credential_migrate`, `probe_routing`

**Tests that pin the Mac:**

- Activity and calls from the samples (spike S6 names the missing Private AI reads)
- A call proof shows only what the core verified; a failed read shows a dash
- Each daemon reply settles against the action pressed

**Decoration in this slice:**

No decoration items.

**Owner decisions:**

- **W19-D1. Tab body.** Windows has a Private inference pane of its own.
  - **Taken 2026-11-19: (a).**
  - (a) The Mac's tab body and inspector replace it
  - (b) Keep Windows' pane and add the missing reads to it
- **W19-D2. Spend organisation prompt.** The Mac asks for a spend organisation (`SpendOrganizationPrompt.swift`). On Windows that prompt would be new.
  - **Taken 2026-11-19: (a).**
  - (a) Port the prompt, under the notice rule (approved as a class, 2026-11-19)
  - (b) The Settings card only

### W20. Insights

**Mac sources:**

- `Views/InsightsView.swift` (`:205-207`), `InsightsOverviewTab.swift`, `InsightsPatternsTab.swift`, `InsightsBenchmarksTab.swift`, `InsightsBreakdownInspector.swift`
- `Views/InsightsSkillsTab.swift`, `InsightsSkillsHomeView.swift`, `InsightsSkillsCover.swift`, `SkillLearningView.swift`, `SkillsCreatorFollowButton.swift`
- `Views/InsightsUsageCards.swift`, `InsightsTokensByDay.swift`, `InsightsSubscriptionsCard.swift`, `InsightsCreditViews.swift`, `InsightsWorkViews.swift`, `InsightsGetStarted.swift`
- `Views/SentTracesTrainingViews.swift`, `TCShellCore/SentTracesTrainingSurface.swift`
- `TraceCommonsApp/DaemonClient+SkillLearning.swift`

**Windows files:**

- `Controls/InsightsView.xaml`, `InsightsView.xaml.cs`
- `Interop/LocalInsights.cs` (`tc_insights_call`), `InsightsSummary.cs`, `InsightsAnalytics.cs`, `InsightCards.cs`, `InsightEpisodes.cs`

**C ABI:**

- (b) `tc_insights_copy_json`, `tc_skill_learning_copy`, `tc_skill_learning_error_line`, `tc_skill_draft_validate`, `tc_sent_traces_training_copy_json`
- (d) `insights_benchmarks`, `insights_skills`, `insights_skills_follow`, `insights_submitted`, `insights_model_answers`
- (d) `skill_candidate`, `skill_evaluate`, `skill_review`, `skill_install_plan`, `skill_install_commit`, `skill_install_rollback`, `skill_install_status`
- (d) `model_training_consent_status`, `model_training_consent_grant`
- `insights_week` and `insights_session_drill`: the Mac sends these to the daemon. Windows already reads `week_overview` and `session_drill` locally through `tc_insights_call` (`LocalInsightsOperation`, `crates/trace-commons-contributor-insights/src/insights/service.rs:472`). See W20-D1.
- Shared: `insights_glance` (W11)

**Tests that pin the Mac:**

- Tabs and their order from `InsightsView.swift`
- Skill install plan, commit and rollback settle against the daemon's replies; a refused commit leaves nothing installed
- Sent-traces training consent: nothing preselected; no grant sent without the action
- The clock-driven refresh periods: 60 s, 30 s and 1 s (row below)

**Decoration in this slice:**

Moved here from the W0 inventory's G-4 rows.

| Item | Mac source | Windows today | Status | Weight | Numbers to pin | Decision |
|---|---|---|---|---|---|---|
| Skill cover art | TraceCommonsApp/Views/InsightsSkillsCover.swift:175 gradient, :179 Canvas SkillCoverArt, :199 text shadow | gradient greps 0 (Z2); grep -i skill in Controls/InsightsView.xaml: 0 lines | missing | medium, M |  |  |
| Insights bars grow and numbers roll | TraceCommonsApp/Views/InsightsSkillsHomeView.swift:196 expand, :343-344 and :372-373 numericText, :440-443 hover, :472-495 grow | static Rectangle bars (Controls/InsightsView.xaml:57-58, 119-120, 136, 189, 282-283); animation greps 0 (Z1) | missing | medium, S |  |  |
| Clock-driven text refresh. Spans Insights usage (60 s), the balance row (30 s) and the inspector prompt (1 s); each surface's slice adopts W5's shared clock, and this slice lands the shared refresh. | TraceCommonsApp/Views/InsightsUsageCards.swift:45 (60 s), BalanceRow.swift:82 (30 s), Monitor/InspectorPrompts.swift:190 (1 s) | the 1 s undo count already ticks (ViewModels/MainViewModel.cs:48, :150-157); no 60 s or 30 s text refresh; the other timers are the 10 s poll (Controls/ManagedSessionsView.cs:20) and the visibility debounce (MainWindow.xaml.cs:98) | partial | low, S | 60 s, 30 s, 1 s (cited lines) |  |

**Owner decisions:**

- **W20-D1. Week and drill reads.** The Mac asks the daemon; Windows computes them locally.
  - **Taken 2026-11-19: (a).**
  - (a) Send the daemon methods the Mac sends
  - (b) Keep the local `tc_insights_call` reads
- **W20-D2. Skill install.** The Mac plans, installs, reviews and rolls back skills in a tool's folder.
  - **Taken 2026-11-19: (a).**
  - (a) All of it, as on the Mac
  - (b) Browse and follow only; no install from Windows
- **W20-D3. Sent-traces training consent.** A consent surface new to Windows.
  - **Taken 2026-11-19: (a).**
  - (a) Port it with the Mac's flow, nothing preselected
  - (b) Leave it Mac-only for now

### W21. History detail and public runs

History is a Home sub-page from W2 (W2-D1 (a)).

**Mac sources:**

- `Views/Monitor/HistoryInspector.swift`, `Views/HistoryView.swift`, `Views/SessionDetailView.swift`
- `TCBridge/TCPublicRun.swift`
- `TraceCommonsApp/DaemonClient.swift` (`publish_public_run`, `unpublish_public_run`)

**Windows files:**

- `Controls/HistoryView.xaml`, `HistoryView.xaml.cs`, `ViewModels/HistoryRecordViewModel.cs`
- `Interop/HistoryCopy.cs`, `HistoryProtocol.cs`
- The inspector pane from W2

**C ABI:**

- (b) `tc_public_run_copy`, `tc_public_run_error_line`, `tc_public_run_validate_editor`, `tc_session_detail_error_line`
- (d) `publish_public_run`, `unpublish_public_run`
- Shared: `history_detail` (W3)

**Tests that pin the Mac:**

- The detail inspector against the samples; a failed read shows a dash
- The editor is validated by `tc_public_run_validate_editor`, not by C#
- Unpublish returns the run to its earlier state; a refused publish leaves it private

**Decoration in this slice:**

No decoration items.

**Owner decisions:**

- **W21-D1. Public runs.**
  - **Taken 2026-11-19: (a).**
  - (a) Publish and unpublish, as on the Mac
  - (b) The detail inspector only; no publishing from Windows

### W22. Preview sheet

**Mac sources:**

- `Views/PreviewSheet.swift` (Cmd-F `:228`), `Views/SessionSendDisclosureView.swift`, `Views/ScrubbingCaveat.swift`

**Windows files:**

- `Controls/PreviewSheet.xaml`, `PreviewSheet.xaml.cs`, `PreviewWindow.cs`
- `ViewModels/PreviewSheetViewModel.cs`, `Interop/PreviewSummary.cs`

**C ABI:**

- (b) `tc_preview_turns_json`, `tc_redaction_summary_json`
- (d) `preview_unsure_spans`
- Windows already opens the preview (`tc_preview_open`) and searches the original (`tc_search_original`)

**Tests that pin the Mac:**

- Turns and unsure spans from the samples
- The redaction summary's counts match the core's
- Find opens from the keyboard

**Decoration in this slice:**

No decoration items.

**Owner decisions:**

- **W22-D1. Find in the preview.** The Mac has Cmd-F (`PreviewSheet.swift:228`).
  - **Taken 2026-11-19: (a).**
  - (a) Ctrl+F, as on the Mac
  - (b) No find key
- **W22-D2. Unsure spans.**
  - **Taken 2026-11-19: (a).**
  - (a) Mark them in the text, as on the Mac
  - (b) A count in the summary only

### W23. Missions

A Home sub-page (W2-D1 (a)). Depends on W2 and W11.

**Mac sources:**

- `Views/Monitor/MissionsViews.swift`, `Views/MissionDraftsView.swift`

**Windows files:**

- `Controls/MissionDraftsView.xaml`, `MissionDraftsView.xaml.cs`, `Interop/MissionDrafts.cs`

**C ABI:**

- Shared: `mission_catalogue`, `activity_missions_catalogue`, `activity_missions_status` (W11)

**Tests that pin the Mac:**

- Missions against the samples; a failed read shows a dash, never 0

**Decoration in this slice:**

No decoration items.

**Owner decisions:**

- **W23-D1. Missions and Mission drafts.** Windows has a Mission drafts page; the Mac has a Missions sub-page.
  - **Taken 2026-11-19: (a).**
  - (a) The Mac's Missions page replaces Mission drafts
  - (b) Both, side by side under Home

### W24. About window

**Mac sources:**

- `Views/AboutWindowView.swift:10` (icon 96 x 96, name, the version with its build, a Copy button, the copyright)
- `TraceCommonsAppMain.swift:173` (the window), `:422` (the menu item)

**Windows files:**

- A new About window
- `PackageIdentity.cs` for the version (whether it exposes the version and build is unverified)

**C ABI:**

- (c) `about_app` from `tc_monitor_screens_copy_json` ("About " plus the app name, `preview_copy.rs:1591`)
- The Copy button's word is `SettingsLegacyWords.copy` on the Mac; whether it is a core word is unverified. If not, it is bucket (a).

**Tests that pin the Mac:**

- The version line is `<version> (<build>)`, or the version alone when the build is missing or the same (`AboutLines.line`)
- Copy puts exactly the shown line on the clipboard

**Decoration in this slice:**

No decoration items.

**Owner decisions:**

- **W24-D1. Where About opens.** The Mac opens it from the app menu.
  - **Taken 2026-11-19: (a).**
  - (a) A tray menu row and a row in Settings
  - (b) Settings only

### W25. Keyboard and accessibility audit

The cross-cutting acceptance items, checked across the whole app after W24.
The Mac sets `accessibilityLabel` at 155 sites; Windows sets
`AutomationProperties` at 40.

**Mac sources:**

- Every `accessibilityLabel` site in `macos/Sources/TraceCommonsApp`
- `TraceCommonsAppMain.swift:291-303`, `:435` (keys)

**Windows files:**

- Every XAML file and code-built control
- A test that reads XAML as text (beside the existing ones in `TraceCommons.Interop.Tests.csproj:163-178`)

**C ABI:**

- None beyond the slices. Every name comes from words they bind.

**Tests that pin the Mac:**

- Every interactive element in XAML has `AutomationProperties.Name` bound to a core word
- Every tab, toolbar toggle, map node and notice is reachable by keyboard, checked on the test box with the UI Automation client

**Decoration in this slice:**

No decoration items.

**Owner decisions:**

- **W25-D1. UI Automation driver.**
  - **Taken 2026-11-19: (a).**
  - (a) The UI Automation client from PowerShell, no new dependency
  - (b) A driver library (a new dependency, needing approval)
- **W25-D2. The credential-store self-check.** `tc_credential_store_self_check` checks the Mac code signature's keychain entitlements (`TCBridge/TCCredentialStore.swift:12-14`, called at `TraceCommonsApp/SelfTest.swift:56`).
  - **Taken 2026-11-19: (a).**
  - (a) A Windows self-check for its own credential store
  - (b) Not applicable on Windows

## Corrections to the guide (#2190)

Checked at 19b42d0d2:

- `GlassThreePane` is used at `MonitorWindowView.swift:302`, not `:259`.
  The pane numbers are at `ThreePane.swift:44` and `:48`; seeding is at
  `:204-207`.
- The wording guards are at `ShellWordingTests.cs:192` and `:249`. The
  baseline is 40 files and 337 sentences.
- The MSIX manifest has four "Near.ai" literals (`Package.appxmanifest:32`,
  `:75`, `:92`, `:109`), not one.
- The rail has up to six entries, not four (Mission drafts shows only while `MissionsShown` is set).
- Most of the Linux stack has landed: shell, map, transfers, look, Home
  decoration and brand. #2177, #2171 and #2185 have not.
- `scripts/parity/gallery.sh` is on main.
- The arming path is now traced as far as the daemon (spike S5).
- The repo's dev VM is a GCE Server 2022 box; the owner's box is Azure
  Server 2025.
- The Linux decoration inventory files were not available, so this
  inventory was rebuilt from a fresh Mac sweep and is not mapped item by
  item onto Linux's 67.

## What this breaks, deliberately

- **`ShellWordingTests`.** Moving words to the core lowers the baseline
  (`ShellWordingTests.cs:192`). No new words are authored in the shell.
- **`NoCommonsNameTests`** (`NoCommonsNameTests.cs:45`) stays green; W10
  adds a test that the manifest's app-name display names (`:32`, `:75`,
  `:109`) equal `APP_NAME`. The invite protocol's name (`:92`) waits on
  W10-D2.
- **The undo-window setting** (`ApprovalHoldSeconds`,
  `Controls/SettingsView.xaml:279`) is removed (W9-D1 (a)), with whatever
  reads it.
- **Tests that read XAML as text** (`TraceCommons.Interop.Tests.csproj:163-178`)
  move with `MainWindow.xaml` and `SettingsView.xaml`.
- **Keyboard accelerators** (`MainWindow.xaml:44-49`) change with the tabs:
  Ctrl+1 to Ctrl+4 by tab position, Ctrl+comma for Settings, Ctrl+5 gone,
  Ctrl+Shift+M kept (W2-D5 (a)).
- **Tauri on Windows** is retired once the WinUI app matches (G-1 (a)).
  CLAUDE.md is updated in a separate change.
- **The tray menu rows** change with the tabs and W13.
- **`XamlCommentSyntaxTests`** still forbids `--` in XAML comments.
- **The named-pipe ACL step** is rerun whenever the daemon's IPC hosting is
  touched (W15).

## Acceptance

- For every sample set, tab, pane combination and appearance, the gallery
  shows the same structure, words, states and pane geometry within a few
  pixels. Decoration and motion are compared too, from renders and still
  frames.
- The same under larger text and with animations off.
- Every tab, toolbar toggle, map node and notice is reachable by keyboard and
  named from core words.
- Selection, consent and send boundaries are no more permissive than the
  Mac's. Nothing is preselected. A failed read shows a dash, never 0.
- No new words authored in the shell. No new notices without approval.
- "Trace Commons" appears nowhere a user can see.
- The interop suite, the `windows contributor app` job (with the new
  render-test project), the contributor crate tests, fmt and clippy pass.
- A run on the Azure Server 2025 box is recorded for each slice. The release
  gate is a final run on a physical Windows 11 PC, which is not yet bought;
  until then nothing is called released on Windows. Windows 10 22H2 shows
  the painted and font fallbacks correctly.

## Owner decisions

**Taken 2026-11-18** (see "Decisions taken 2026-11-18" for each answer):
G-1 to G-5; W1-D1, W1-D2; W2-D1 to W2-D5; W3-D1, W3-D2; W4-D1; W5-D1 to
W5-D3; W6-D1; W7-D1, W7-D2; W9-D1 to W9-D3; W10-D1; W11-D1 to W11-D3;
W12-D1 to W12-D4; W13-D1; W14-D1 to W14-D3.

**Taken 2026-11-19** (see "Decisions taken 2026-11-19"): W10-D2 (a); W14-D1
(b), replacing (c); W14-D2 (a), gated on its spike; W15-D1 (a), gated on
spike S4; the Windows 10 22H2 floor; W16-D1 to W25-D2, each the option that
matches the Mac (a); cards and notices new to Windows, approved as a class;
the W2 deferrals to W3 and W9.

**Still open:** only the spike-gated W14-D2 (whether "Show" ships) and
W15-D1 (whether two daemons collide).
