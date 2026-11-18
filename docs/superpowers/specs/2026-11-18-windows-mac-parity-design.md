# Windows app, matching the native Mac app

Status: draft for owner review. Nothing here is implemented. Facts about the
code are cited at `origin/main` 19b42d0d2. Anything not read in source is
marked unverified.

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
- Font: Segoe UI Variable. It is already the default (`Themes/DesignSystem.xaml:220`).
- Test box: an Azure Windows Server 2025 machine. There is no Windows 11 licence yet, so Mica is unavailable there.
- Decoration and motion are parity, not polish. Each item lands in the slice it belongs to.
- "Trace Commons" is never user-visible. The name comes from `brand.rs`.

Every other choice is listed per slice, as options, under "Owner decisions".
Where Linux made the same choice, that option is marked "Linux precedent".
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
- Tauri on Windows. It is not touched.
- Mac surfaces that no slice covers yet. They are listed below and wait for
  decision G-4.

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
  The rail has six entries: Waiting (the default), History, Private
  inference, Insights, Mission drafts and Settings. "Insights" and "Mission
  drafts" are words written in the XAML (`:327`, `:328`). Panes are
  ContentPresenters (`:898-930`).
- **Missing.** No map, no inspector, no Home tab, no graph. Settings is a
  pane. Nothing animates: no Storyboard, Composition animation, gradient,
  shadow, `SystemBackdrop` or `UISettings` read anywhere in `windows/src`.
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
- Nothing compares the C# bindings with the header. W1 adds that test.

Each slice below sorts its ABI work into four buckets:

| Bucket | Meaning | Work |
|---|---|---|
| (a) | Not in Rust or the header at all | Rust field or export, both header copies, Swift decode contract, C# |
| (b) | In the header, not bound in C# | `DllImport`, decoder and tests |
| (c) | Bound, but unused or partly decoded | C# decoder only |
| (d) | A daemon method Windows never sends | C# call and view model; no ABI work |

The daemon dispatches 196 methods
(`crates/trace-commons-contributor/src/daemon/ipc.rs`). The Mac sends 96
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
  These run on Windows only (decision W1-D1, W1-D2), not in the macOS run.
- **The gallery.** `scripts/parity/gallery.sh` (on main) pairs captures named
  `<set>-<tab>-<panes>-<appearance>.png`. Windows captures come from the test
  box through `windows/scripts/win-capture.ps1`, which launches the app in
  the interactive session.
- **Accessibility names.** Each ported surface sets
  `AutomationProperties.Name` from core words. Names are checked on the box
  with the UI Automation client in PowerShell (unverified; a driver library
  would be a new dependency).
- **Tests that fail on a wrong port.** Each new test is run against the old
  or a mutated behaviour first and must fail: a wrong spring, a wrong ring
  order, a halo inset of 3 instead of 2.5.
- **The test box.** Azure Windows Server 2025. No Mica there. The repo's
  scripts drive a GCE box only (decision G-5). Never sign in, enroll or share
  on the box. Clean up after each run, and stop the box when done.
- **Owner sign-off** is on the gallery, not on test counts.

## Spikes

- **S1. Win2D on the pinned SDK.** Does Win2D work with Windows App SDK
  1.6.240923002 and minimum platform 10.0.17763? Does it draw on the Server
  2025 box, which may have no GPU (software rendering is unverified)? Can it
  render offscreen to a PNG for the diffs? Pick the package version from
  this spike, not from memory.
- **S2. Window growth.** `AppWindow.Resize` and `Move` with an extended
  title bar, across DPI scales. Feeds W2-D3.
- **S3. Exec and capture on the Azure box.** How commands reach the box, and
  whether `win-capture.ps1` works there unchanged. Feeds G-5.
- **S4. Two daemons.** What the CLI's `daemon run` does on Windows while the
  app's in-process daemon runs, and whether both watch the same sessions.
  Feeds W15-D1.
- **S5. The arming gate.** Windows arms through `set_project_mode`
  `auto_upload` (`MainWindow.xaml.cs:896-908`). The daemon records the grant
  terms in force, which may be none (`ipc.rs:6040-6044`, `.flatten()`).
  Whether the automatic pass then sends without an account-level grant is
  not traced. This must be settled before W9-D2 is asked, and before W9
  ships.
- **S6. Sample coverage.** Which reads the map, tree, graph, inspector and
  Home need that the recorded sets lack (account devices and the Private AI
  network reads are known gaps).
- **S7. Motion settings.** `UISettings.AnimationsEnabled` on the box, and
  the window visibility and minimise events that stop each clock.

### General decisions

These apply across slices.

- **G-1. Unfreeze and which Windows client.** CLAUDE.md still says Windows stays on Tauri, frozen, until macOS ships (#1173 D13). The owner asked to start Windows parity in the WinUI app (`windows/`).
  - (a) The WinUI app in `windows/` is unfrozen for parity work now and becomes the Windows client; Tauri on Windows is retired once it matches, as on macOS; CLAUDE.md is updated in a separate change (Linux precedent: GTK unfrozen for development on 2026-11-11, releases off until it catches up)
  - (b) Spec work only for now; slices wait
- **G-2. Look target.** Match the Mac's look as closely as Windows allows, or keep a Fluent look with the Mac's structure.
  - (a) The Mac look, as close as possible (Linux precedent, 2026-11-16)
  - (b) A Fluent look with the Mac's structure
  - Note: The owner's remark that decoration and motion are parity points to (a). Confirm it.
- **G-3. App mark.** Which mark the app shows in the title bar, the tiles, the tray and the first-run stage. Asked once here; W2, W8 and W12 apply it.
  - (a) The NEAR AI N everywhere, tiles too (Linux precedent: the N for the app, tray and Flatpak icon, #2186, still open)
  - (b) The N in the app, tray and stage; the Turn stays on the MSIX tiles only
  - (c) Keep the Turn everywhere
- **G-4. Surfaces no slice covers.** Mac surfaces outside W1-W15 (section "Surfaces no slice covers"), including three Insights decoration items.
  - (a) Add slices W16 and later for them
  - (b) Fold each into the nearest existing slice
  - (c) Mark some not applicable (only the owner can choose this, per surface)
  - Note: No Linux precedent: the Linux spec left screen content out of scope.
- **G-5. Tooling for the Azure test box.** The repo's dev-VM scripts drive a GCE Windows Server 2022 box through gcloud and IAP. Nothing drives the Azure Windows Server 2025 box.
  - (a) Add `az`-based equivalents of `provision-dev-vm.sh` and `win-exec.sh`, keeping `win-capture.ps1` on the box
  - (b) Drive the box by hand over RDP and run `win-capture.ps1` there
  - Note: No Linux precedent (Linux used GCP desktops).

## Slices

Each slice is one PR. W1 to W4 are a stack. W5 can start beside W2. W7 and
W9 depend on W2. W11 depends on W2 and W5. While a slice is written it runs
only a compile check. At its end it runs the interop suite, the WinUI build
(CI or the box), the contributor crate tests for any core change, and fmt
and clippy.

Decoration rows come from the W0 inventory: 80 items, of which 68 are
missing, 6 partial, 1 ported and 5 not applicable. Decoration lands in the
slice it belongs to. It is not deferred. Four items moved from the
inventory's slice to the first slice that draws them; each says so.

### W1. Samples, gallery and the binding test

**Mac sources:**

- `TCShellCore/DataContract/RecordedSamples/` (sets `empty`, `normalDay`, `busyQueue`, `heldSessions`, `armedFolder`, `unknownCounts`, `shared`)
- `TraceCommonsApp/DebugScreenshot.swift`
- `MonitorWindowView.swift:221-235` (`sampleClient()`: `TRACE_COMMONS_SAMPLE`, debug builds only)

**Windows files:**

- A debug-only sample client in `Interop/`, chosen by `TRACE_COMMONS_SAMPLE`
- A capture driver run through `windows/scripts/win-capture.ps1`
- Gallery naming for `scripts/parity/gallery.sh`

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
  - (a) On the test box only, never uploaded from CI (Linux precedent)
  - (b) Also as a CI artifact
- **W1-D2. Render tests in CI.** Win2D pixel tests cannot run in the plain `net8.0` test project on macOS.
  - (a) A new `net8.0-windows` render-test project run in the `windows contributor app` job
  - (b) Render tests on the test box only; CI keeps the pure-logic tests
  - Note: Linux precedent: its pixel tests run in the GTK suite and the weston job, so closer to (a).
- Also applies: G-5 (how the test box is driven).

### W2. Window shell

The toolbar row moves into `AppTitleBar` with the system caption buttons (decided). The rail is replaced by four tabs. Mac notices with no Windows card found in the survey (unverified card by card; each is a new notice on Windows and needs its own yes under W2-D4): a daemon that refused to start, onboarding owed, the attached-daemon limits, first run's last word, a failed or finished action, the traces moving now, the undos, arming rewordings, a consent gate hold, the backlog and idle suggestions, the idle notifications offer, the first-contribution note, Resume while uploads are paused, History's verdict news and verdict notifications offer, and the NEAR AI sign-in card (`Views/Monitor/MonitorNotices.swift:6-25`). Windows today has the update InfoBar, the health banner, the switch-on and grant-void notices, the legacy migration card, the private inference offer, the arming offer and the witness capacity banner.

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
- `Controls/SettingsView.xaml` hosted in its own window (if W2-D2 a)
- `TrayIcon.cs` menu rows and the accelerators, for the four tabs
- `Controls/HistoryView.xaml`, `MissionDraftsView.xaml` moved (W2-D1)

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
- Notice stack: peek 8, 3 drawn, 4% narrowing a level, opacity 0.9 and 0.7, spring 0.36/0.86
- Pane open and close on `Motion.standard` 0.22 s, none with animations off
- `ShellWordingTests` baseline goes down

**Decoration in this slice:**

| Item | Mac source | Windows today | Status | Weight | Numbers to pin | Decision |
|---|---|---|---|---|---|---|
| NEAR AI N mark (app mark). Moved from W12: the title bar draws the mark first. W3, W6, W8 and W12 reuse the path. | TCDesign/Logos/GlassBrandMark.swift:10 (path); drawn by the map marks (Indicators.swift:759, 849), the stage (FirstRunStage.swift:261), the strip (MenuBarPanel.swift:316) | Windows draws the Turn instead: Controls/BrandMark.xaml:32-58 (two bracket paths, "no gradients"), used in the title bar (MainWindow.xaml:92) and the empty queue (MainWindow.xaml:1876); MSIX tiles in windows/packaging/Assets are the green Turn (Square150x150Logo.png inspected) | missing | high, M |  | G-3 |
| Toolbar row in the title bar | TCDesign/Components/ThreePane.swift (GlassThreePane); MonitorWindowView.swift:979 GlassRuleTabs, :1146 GlassSegmentedTabs | custom title bar exists (MainWindow.xaml.cs:129-130 ExtendsContentIntoTitleBar/SetTitleBar; AppTitleBar MainWindow.xaml:86) but holds only the Turn and the literal "Near.ai", no toolbar row | partial | high, M |  |  |
| Pane open and close (map, inspector) | TCDesign/Components/ThreePane.swift:358-376 (map and inspector fade), :538-550 (window frame change on the standard curve) | No map control. | missing | medium, S | Motion.standard 0.22 s on the token curve (ThreePane.swift:547, GlassTokens.swift:694); none under reduce motion (:540) |  |
| Traces graph footer: bars, period slide, open and close. Moved from W5: the footer is part of the shell, as in the Linux spec. | TCDesign/Components/Indicators.swift:358 GlassBarGraph (Canvas :433, hover :460), :414-416 and :537 GlassBarSlide; TraceCommonsApp/Views/Monitor/MonitorShell.swift:233-290 (period steps slide, zoom swaps in place), :292-293; MonitorWindowView.swift:410-418 (footer rises from the bottom), :943, :1034 | no graph footer; Insights draws static Rectangle bars (Controls/InsightsView.xaml:57-58, 119-120, 136, 189, 282-283), a different chart; animation greps 0 (Z1) | missing | medium, M | period slide 0.45 s on (0.4, 0, 0.2, 1) (MonitorShell.swift:293); zoom on GlassMotion.fast (:284); footer opens over 0.25 s (MonitorWindowView.swift:943, :1034) |  |
| Notice stack: peeking plates and arrival | TCDesign/Components/PopoverNotice.swift:182 GlassPopoverStack, :201-202, :225 arrival, :238 spring, :258-260 | no notice overlay; the one InfoBar is the update notice (MainWindow.xaml:397-411); animation greps 0 (Z1) | missing | high, M | peek 8, 3 drawn (PopoverNotice.swift:201-202); plates narrow 4% a level, opacity 0.9/0.7 (:258-260); spring 0.36/0.86 (:238) | W2-D4 |
| Notice mark and its symbol swap | TCDesign/Components/PopoverNotice.swift:116 GlassNoticeMark (side 30 :143), :154 contentTransition symbolEffect replace; MonitorNotices.swift:844, :882 bounce | animation greps 0 (Z1) | missing | medium, S | mark side 30 (PopoverNotice.swift:143) |  |
| Arming offer: bolt to check, spreading ring, hold and exit | TraceCommonsApp/Views/Monitor/ArmingOfferMotion.swift:18, 37, 42, 66, 72; AppModel.swift:2197, 2221; MonitorNotices.swift:882-884 | the offer is a text card (MainWindow.xaml:753, ViewModel.HasArmingOffer) with no mark and no motion; animation greps 0 (Z1) | missing | medium, S | hold 900 ms (:18); exit spring 0.42/0.9 (:37); mark spring 0.32/0.7 (:42); ring 1 to 1.9, easeOut 0.7 s (:66, :72) |  |
| Tree search field reveal | TraceCommonsApp/Views/Monitor/NudgeViews.swift:150 (opacity + move trailing), :153/:161 fast | animation greps 0 (Z1) | missing | low, S |  |  |

**Owner decisions:**

- **W2-D1. Rail destinations.** Where History and Mission drafts go once the rail is replaced by four tabs.
  - (a) Home sub-pages behind a breadcrumb, as on the Mac (Linux precedent, #2160)
  - (b) Keep a rail beside the tabs
- **W2-D2. Settings window.** Settings is a pane on Windows and its own 780x640 window on the Mac.
  - (a) Its own window, as on the Mac (Linux precedent)
  - (b) Stay a pane
- **W2-D3. Window growth at the screen edge.** On the Mac a pane opening grows the window at its right edge and slides it left at the screen edge.
  - (a) The Mac's behaviour, with `AppWindow.Resize` and `Move` (spike S2)
  - (b) The window keeps its size and the panes reallocate inside it (Linux precedent: forced by Wayland, not chosen)
- **W2-D4. Notices and inspector demand.** Windows stacks inline cards above the pane with no X. The Mac lays a peeking stack over the tab and opens the inspector on demand.
  - (a) Both as on the Mac: the overlay stack with its X rules, and the inspector opening itself on the Mac's demand keys (Linux precedent: decision 1, "all three approved")
  - (b) Re-house the existing cards only; the inspector opens on selection only
  - Note: Either way, a card the Mac shows that Windows lacks today is a new notice on Windows (copy guide rule 9). The spec lists those cards under W2; each needs its own yes.
- **W2-D5. Keyboard map.** Windows has Ctrl+1 to Ctrl+5 in rail order (`MainWindow.xaml:44-48`). The Mac has Cmd-1 to Cmd-4 by tab position and Cmd-, for Settings.
  - (a) Ctrl+1 to Ctrl+4 by tab position, Home first, and Ctrl+, for Settings (Linux precedent: `<Control>comma` at `crates/trace-commons-contributor-gtk/src/ui/shortcuts.rs:44`)
  - (b) Keep the rail-order keys
- Also applies: G-3 (the mark in the title bar).

### W3. Traces map and tree

Drawn with Win2D (decided). Nodes stay focusable elements with names; arcs and motion are drawn.

**Mac sources:**

- `Views/Monitor/FlowMapScene.swift`, `FlowMapView.swift`, `MapToolPile.swift`
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
- (d) `tool_destinations`, `keep`, `list_kept`, `undo_keep`, `history_detail`, `include_past_sessions`, `list_past_sessions`

**Tests that pin the Mac:**

- Port `FlowMapSceneTests` against the samples
- Folder column at 30%, 10 folders then "+N" revealing 6; piles at 50%; hub at 72%
- An arc flows only for an auto-upload folder, gate open, sessions bound for the commons
- Arc dash 5/5; flowing dash 4/8 at width x1.3, 18 pt/s

**Decoration in this slice:**

| Item | Mac source | Windows today | Status | Weight | Numbers to pin | Decision |
|---|---|---|---|---|---|---|
| Tool logos (path data). Moved from W6: the map's tool nodes draw logos first. | TCDesign/Logos/GlassLogoPaths.swift:21, GlassToolLogo.swift:36 GlassToolLogoShape, SVGPathData.swift:13 | grep -w logo\|logos over windows/src *.xaml and *.cs: 0 lines; icons are 13 FontIcon lines in 6 files and hand-drawn 16 px Canvas glyphs; Win2D greps 0 (Z5) | missing | high, M |  | W3-D1 |
| Tool tiles. Moved from W11: the Traces rows draw tiles first. W11 and W14 reuse them. | TCDesign/Components/Indicators.swift:251 GlassToolTile; sizes GlassTokens.swift:577-578; 15 call sites in TraceCommonsApp (first ManagedSessionsView.swift:343) | no tile control; grep -w logo\|logos over windows/src *.xaml and *.cs: 0 lines | missing | medium, M | tile 22, large 30 (GlassTokens.swift:577-578) |  |
| Map arcs with moving dashes | TraceCommonsApp/Views/Monitor/FlowMapView.swift:80-84 (timeline + Canvas), :239 restingMotion, :263-298 drawArcs (dashPhase :294) | No map control. | missing | high, M | line 1.4 x scale; dashed 5/5; flowing dash 4/8 at width x1.3 moving 18 pt/s (FlowMapView.swift:82, 268-294) |  |
| Map nodes: discs, laptop, launch pad, key glyph, rings | TraceCommonsApp/Views/Monitor/FlowMapView.swift:313 drawNode, :435 drawLaptop, :455 drawPad, :488 keyGlyph, :511 ringMetrics, :767 still Canvas | No map control. | missing | high, L |  |  |
| Map node card | TCDesign/Components/Indicators.swift:887 GlassNodeCard; FlowMapView.swift:680; nodeCardEdge GlassTokens.swift:449 | No map control. | missing | medium, S |  |  |
| Traces folder drag settle and scroll | TraceCommonsApp/Views/Monitor/TracesViews.swift:503 (scroll on GlassMotion.fast), :764 (drag release spring 0.3/0.85); ToolFolderInspectors.swift:88 (fast) | animation greps 0 (Z1) | missing | low, S |  |  |

**Owner decisions:**

- **W3-D1. Tool logos.** The map coins, tool tiles, first-run ring and launch art draw tool logos.
  - (a) The Mac's own path data from `GlassLogoPaths.swift`, generated, with its source recorded (Linux precedent: logos allowed "as needed", #2182)
  - (b) Initials only
- **W3-D2. Private AI map view.** The map's second view reads devices, harnesses and destinations.
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
| Transfer animation: coin rim light, rocket and flame, orb rise | TraceCommonsApp/Views/Monitor/MapToolPile.swift:215 MapTransferDrawing (orb frame :220, rocket :379, flame :428-429, paths :533-592); MapTransfers.swift:93-232; FlowMapView.swift:107-118 | No map control. | missing | high, L | step 1.05, inbound 0.70, orbit 0.45, outbound 0.70, settle 0.5, flight 2.6, linger 1.3, kept linger 1.6, pulse 0.9, ascent 1.1 s (MapTransfers.swift:112-125); rocket 44 (MapToolPile.swift:379); 60 fps only while moving (FlowMapView.swift:107) |  |

**Owner decisions:**

- **W4-D1. Transfer cards at the map foot.** How transfer cards stack while another tab is showing.
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
| Reduce motion and the hidden-window clock stop | TCDesign/Components/AnimationTimeline.swift:4-32 GlassAnimationTimeline (pauses unless the window is visible); GlassMotion.swift:17 systemReducesMotion | grep UISettings over windows/src: 0 lines; the only timer is the 10 s poll in Controls/ManagedSessionsView.cs:20 | missing | high, S |  |  |
| Status dot with halo and ring options | TCDesign/Components/Indicators.swift:4-31 GlassStatusDot (halo ink 0.08 line 2; ring status @0.25 line 3); 5 app uses (HomeViews.swift:815) | a plain 7 px Ellipse for Private inference working (MainWindow.xaml:307-310), no halo or ring | partial | low, S |  |  |
| Mac glyph set (GlassGlyph paths) | TCDesign/Components/Glyphs.swift:8 GlassGlyph, :95 GlassGlyphView (Controls.swift:273, 538, 623) | Segoe FontIcon glyphs (13 lines in 6 files) and hand-drawn 16 px Canvas icons in MainWindow.xaml | missing | medium, M |  | W5-D2 |
| Warning glyph | TCDesign/Components/Feedback.swift:115 GlassWarningGlyph, :141 shape; PrivateAISection.swift:211 | hand-drawn warning triangle Canvas (MainWindow.xaml:453-463, :571-581) and FontIcon E7BA: own geometry, not the Mac's | partial | low, S |  | W5-D2 |
| Spinner (quarter arc on a ring) | TCDesign/Components/Feedback.swift:15 GlassSpinner (timeline :48 at 1/60 :34, trim 0.25 :53, rotation :55); 28 app uses | stands there today: stock ProgressRing (MissionDraftsView.xaml:17, AccountLoginChangeDialog.cs:38) and indeterminate ProgressBar (MainWindow.xaml:2064, HistoryView.xaml:523) | missing | medium, S | one turn per Motion.spin 0.8 s (GlassTokens.swift:702), 60 fps cap (Feedback.swift:34) | W5-D3 |
| Skeleton breathing | TCDesign/Components/Feedback.swift:74 GlassSkeleton, :88-103 pulse; SummaryInspector.swift:186 | animation greps 0 (Z1); grep -i inspector over windows/src/TraceCommons.App: 0 lines | missing | low, S | pulse Motion.pulse 1.6 s (GlassTokens.swift:703) |  |
| Refresh button turn | TCDesign/Components/Containers.swift:71 GlassRefreshButton (timeline :112 at 1/30, rotation :158), :176 GlassCardRefresh; 6 uses (ManagedSessionsView.swift:658) | static refresh FontIcon E72C (Controls/ManagedSessionsView.cs:96, MainWindow.xaml:1045) | missing | low, S |  |  |
| Disclosure chevron rotation | TCDesign/Components/Controls.swift:637-657 GlassExpander (rotation 90), :1019 split button 180; ListRow.swift:150; ProjectsSection.swift:428; ToolsScreen.swift:971-972 | stands there today: stock Expander (17 lines, e.g. MainWindow.xaml:1894, SettingsView.xaml:473), with WinUI's own chevron, not the Mac's | missing | low, S |  | W5-D3 |
| Switch and checkbox motion | TCDesign/Components/Controls.swift:1110 GlassToggleStyle (easeOut :1167), :1198 GlassCheckboxStyle, :1238 GlassCheckMark; FirstRunStage.swift:472 | stands there today: stock ToggleSwitch (9 lines) and CheckBox (10 lines) | missing | low, S |  | W5-D3 |
| UI font family | Mac type scale TCDesign/Generated/GlassTokens.swift:721 TypeScale | Segoe UI Variable Text, Segoe UI (Themes/DesignSystem.xaml:220), the owner's decision for Windows; sizes are hand-transcribed (:226-242), not generated, which W5's generator work covers | ported | medium, S |  |  |
| Tab ember and halo sliding between tabs | TCDesign/Components/Navigation.swift:288 matchedGeometryEffect, :320-340 elliptical halo + linear ember, :225-228, :266 spring | a static 3 px selection bar in the nav rail (MainWindow.xaml:207-213, TcGreenFillBrush), a different control; gradient greps 0 (Z2) | missing | high, S | ember reach 0.3, line 2, halo height 26 (Navigation.swift:225-228); spring 0.35/0.85 (:266); colours tabEmber/tabEmberHalo flat #0091FD, halo @0.31 (GlassTokens.swift:178-180) |  |
| Tab lift on hover and drag | TCDesign/Components/Navigation.swift:244-247 (scale, rise, spring), :296 shadow, :360-362 | animation greps 0 (Z1); shadow greps 0 (Z3) | missing | low, S | scale 1.06, rise 2, shadow @0.18 r 8 y 3 (Navigation.swift:296, 360-362); spring 0.25/0.8 (:247) |  |
| Icon card heads (symbol column) | TCDesign/Components/Containers.swift:213 GlassIconCard, icon :313-318; used in ManagedSessionsView.swift, InferenceAccount.swift, SavedAccountsSection.swift, FundingRow.swift, HomeViews.swift | cards carry no icon column; Segoe FontIcon appears in a few headers only (e.g. Controls/PrivateInferenceView.xaml:347, E756) | missing | low, S |  | W5-D2 |

**Owner decisions:**

- **W5-D1. Glass material.** The Mac draws Liquid Glass under its panes. Windows has an opaque window.
  - (a) Painted glass only, from the flat tokens
  - (b) Mica or Acrylic where the system supports it, with a painted fallback (Linux precedent: the owner chose real blur where the compositor allows, painted elsewhere; what landed paints only)
  - Note: Mica is unavailable on the Server 2025 test box, so (b) cannot be checked there; Acrylic support there is unverified.
- **W5-D2. Glyphs and icons.** The Mac draws its own `GlassGlyph` paths (`TCDesign/Components/Glyphs.swift:8`) and about 30 SF Symbols, which are Apple-only. Windows uses Segoe `FontIcon` glyphs and hand-drawn Canvas icons.
  - (a) Export the Mac's own glyph paths, and map each SF Symbol to a Segoe Fluent Icons glyph in one checked-in table (Linux precedent: own glyph paths exported; stock Adwaita symbolic icons in a table for SF Symbols)
  - (b) Segoe Fluent Icons for everything, in one table
  - (c) Own glyph paths, and hand-drawn lookalikes for the SF Symbols
  - Note: Whether Segoe Fluent Icons is present on Server 2025 is unverified; Segoe MDL2 Assets is the older font.
- **W5-D3. Stock controls.** Windows uses stock ToggleSwitch, CheckBox, Expander, ProgressRing and ContentDialog. The Mac draws its own switch, checkbox, chevron, spinner and modal, with motion.
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
- (a) non-Mac device words (W7-D2)

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
  - (a) Drop the wallet path from first run (Linux precedent, #2177)
  - (b) Keep whatever the Mac's sign-in offers
- **W7-D2. Device words for first run and projects.** First-run and project words exist only as "this Mac" (`first_run_copy.rs:618`, `:634`, `:672`, `:731`; `project_copy.rs:173`).
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
- Stage at 30 fps, only while shown
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
| Tools ring: rotation, scan sweep, found flash | TraceCommonsApp/Views/FirstRun/ToolsScreen.swift:340-353 layout, :419-575 (rotation :463, tighten :464-465, angular sweep :478-495, flash :512, Canvas :533, dashed stream :574); FirstRunStage.swift:60-74, :533 | Legacy onboarding window; no stage, no drawing. | missing | high, L | ring turns once per 90 s (:463); radius 130 loose / 118 tight, tile 52, hub 64 (:349-352); scan lead 1 s, turn 2 s, sweep from -36 deg (FirstRunStage.swift:62-64); ring order Claude Code, Gemini CLI, Codex, Cline, OpenCode (:533) |  |
| Private AI diagram (flowing dashes, counted pill) | TraceCommonsApp/Views/FirstRun/PrivateAIScreen.swift:84-117, :144 | Legacy onboarding window; no stage, no drawing. | missing | medium, M | 30 fps, flow period 1.1 s, dash 5/6 line 2 in firstRunGlow (:85-117) |  |
| Traces step: orbs streaming into a filling ring | TraceCommonsApp/Views/FirstRun/TracesScreen.swift:91-123, :160, :193-208, :285 | Legacy onboarding window; no stage, no drawing. | missing | medium, M | count rises over 1.9 s (doc :91); 60 fps (:160); glow @0.35 r 6 (:196); spatial 0.64 (:285) |  |
| Uses flow: tools folding in, review gate, pulse | TraceCommonsApp/Views/FirstRun/UsesScreen.swift:419-582, :645-655 | Legacy onboarding window; no stage, no drawing. | missing | medium, M | 60 fps (:431); tile shadow r 10 y 8 (:494); pulse blur 3 (:558); expressive 0.46 (:582) |  |
| Join graphic: tools circling the N on a turning dashed ring | TraceCommonsApp/Views/FirstRun/JoinScreen.swift:521-545, :688 | Legacy onboarding window; no stage, no drawing. | missing | medium, M | 30 fps; ring turns once per 90 s (:531); pulse scale 0.3 + 1.6 p, blur 4 (:542-543) |  |

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

- `Controls/SettingsView.xaml` (consent scopes `:214-259`, undo window `:279`)
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

**Decoration in this slice:**

No decoration items.

**Owner decisions:**

- **W9-D1. Undo-window setting.** `ApprovalHoldSeconds` (`Controls/SettingsView.xaml:279`) exists on Windows only and can be set low.
  - (a) Remove it (Linux precedent, #2179)
  - (b) Keep it
- **W9-D2. Arming without an account grant.** Windows arms a folder by sending `set_project_mode` `auto_upload` from the arming card and never sends the automatic-grant family.
  - (a) Match the Mac's grant flow before W9 ships (Linux precedent: "match the mac", #2179)
  - (b) Keep per-project arming as it is
  - Note: Spike S5 traces what the daemon then sends; it runs before this is asked.
- **W9-D3. Where an override is cleared.** The Mac clears a contribution override from its menu-bar panel (`TraceCommonsApp/Views/Monitor/MenuPanelStore.swift:177`, `:199`), after a confirmation. Windows sends neither `set_contribution_override` nor `clear_contribution_override`. Whether the Mac offers a clear anywhere else was not checked.
  - (a) Clear only where the Mac clears, so it arrives with the tray flyout (W13-D1) (Linux precedent: "match the mac", #2179)
  - (b) Also add a clear control in Settings

### W10. Brand

"Near.ai" appears 25 times in 14 files under `windows/src` and 4 times in `windows/packaging/Package.appxmanifest`. The fix reads the name from the core.

**Mac sources:**

- `crates/trace-commons-contributor/src/brand.rs:28-35`

**Windows files:**

- Every "Near.ai" literal, including `MainWindow.xaml:29`, `:97`, `OnboardingWindow.xaml:27`, `:33`, `Controls/SettingsView.xaml:208`
- `windows/packaging/Package.appxmanifest:32`, `:75`, `:92`, `:109`

**C ABI:**

- (a) a new export (or copy field) returning `APP_NAME`, in both header copies, with the Swift export contract updated

**Tests that pin the Mac:**

- The manifest's display names equal `APP_NAME`
- No "Near.ai" literal in shell source
- `NoCommonsNameTests` stays green

**Decoration in this slice:**

No decoration items.

**Owner decisions:**

- **W10-D1. Internal ids.** The package name `Iqlusion.TraceCommons`, the `tracecommons` protocol, `TraceCommons.exe`, the Run value name and the install folder.
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
| Remote campaign image on the campaign hero | TCShellCore/PrivateAICampaign.swift:13 imageURL; PrivateAICampaignClient.swift:42; PrivateAICampaignStore.swift:77; HomeViews.swift:140 | No Home tab. | missing | low, M |  | W11-D3 |

**Owner decisions:**

- **W11-D1. Art where the Mac samples an SF Symbol.** Hero cards draw pixel art sampled from SF Symbols.
  - (a) Hand-drawn pixel art from own geometry (Linux precedent, #2187)
  - (b) Leave those cards without art
- **W11-D2. Walkthrough card.** A closable card on Home.
  - (a) Leave it out (Linux precedent, #2187)
  - (b) Port it, under the notice rule
- **W11-D3. Remote campaign feed and image.** The Mac fetches a campaign feed and its image.
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
| Menu-bar strip: N plus 24 h bars on a centre line | TCDesign/Components/MenuBarPanel.swift:316 GlassMenuBarStrip (doc :297-315, Canvas :532, cut-out :575); 10 refs in the app | Turn raster tray icon; dot for Attention and Unhealthy only (`TrayIcon.cs:625-647`); count in the tooltip. | missing | high, M | height 22, mark 14, gap 4, bar 1, step gap 1, graph 17, centre line 1 (MenuBarPanel.swift:475-493); bars only once a few days of data exist (doc :307-308) | W12-D1 |
| Tray badge (count, 99+, figure cut out of the mark ink) | TCDesign/Components/MenuBarPanel.swift:302-310 (doc), :460-464 | Turn raster tray icon; dot for Attention and Unhealthy only (`TrayIcon.cs:625-647`); count in the tooltip. | missing | high, S | badge 11, glyph 7, inset 2.5, lift = halo reach (MenuBarPanel.swift:460-463); 99+ past 99 (doc :303-304); never red (doc :309-310) | W12-D2 |
| Tray news ring and idle halo | TCDesign/Components/MenuBarPanel.swift:338-346 (Mark), :389 shownMark, :470-472 | Turn raster tray icon; dot for Attention and Unhealthy only (`TrayIcon.cs:625-647`); count in the tooltip. | missing | medium, S | halo inset 2.5, line 2, reach = inset + line/2 (MenuBarPanel.swift:470-472) | W12-D3 |
| Tray attention mark | TCDesign/Components/MenuBarPanel.swift:319-334 Condition (attention, unavailable) | Turn raster tray icon; dot for Attention and Unhealthy only (`TrayIcon.cs:625-647`); count in the tooltip. Classic purple, not the flat blue. | partial | medium, S |  |  |

**Owner decisions:**

- **W12-D1. Tray icon content.** The Mac's menu-bar strip is the mark plus 24 h bars; a Windows tray icon is a small square.
  - (a) Mark and badge only
  - (b) Try the bars in the tray icon
  - Note: Linux precedent (#2186) ported the mark, badge, ring and halo; whether it drew the bars is not recorded here.
- **W12-D2. Tray badge.** Windows shows a dot and puts the count in the tooltip.
  - (a) The Mac's badge, 99+ cap, figure cut from the mark ink, never red (Linux precedent, #2186)
  - (b) Keep the dot and tooltip count
- **W12-D3. Tray news ring and idle halo.**
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
  - (a) Windows Terminal only
  - (b) Add the PowerShell console host
  - (c) Add others after an argv check
  - Note: Linux precedent: four terminals added (#2170), each from an argv vector.
- **W14-D2. "Show" (raise a running session).**
  - (a) Spike it on Windows
  - (b) Leave it Mac-only (Linux precedent: Wayland gave no safe way)
- **W14-D3. Per-tool launch buttons.** The Mac has a launch button per tool, with its logo. That is a new per-tool start flow, not only decoration.
  - (a) Port it with the per-tool flow
  - (b) Leave it out (Linux precedent, #2187)

### W15. Background service, only if wanted

**Mac sources:**

- `TraceCommonsApp/DaemonHost.swift`, `TCBridge/TCAttach.swift`

**Windows files:**

- `DaemonHost.cs`, `RunAtLogin.cs`, `StartupRegistration.cs`

**C ABI:**

- (b) `tc_daemon_attach`, `tc_attach_copy`, `tc_subscribe_with_accepts`
- (a) service dialog words, which are not on main

**Tests that pin the Mac:**

- One watcher per login, if S4 shows two daemons can collide
- The named-pipe ACL step still refuses a second account

**Decoration in this slice:**

No decoration items.

**Owner decisions:**

- **W15-D1. Background service.** The Windows app is not sandboxed and hosts the daemon in-process.
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


## Surfaces no slice covers

These Mac surfaces have no slice in W1-W15. Each waits for decision G-4.
None is marked not applicable here.

- **Settings sections:** account and devices, payouts, referrals, saved
  accounts, notifications and nudges, Insights settings, tools as on the Mac
  (CLI tool row, managed favourites), updates, compute and appearance.
  ABI: bucket (b) `tc_account_devices_copy_json`, `tc_payout_link_copy_json`,
  `tc_referral_copy_json`, the ten `tc_compute_*` exports and others; bucket
  (d) the `account_device_*`, `payout_link_*` and `referral_*` methods.
- **The Private AI tab body:** activity, calls, call proof, spend
  organisation and the campaign card (`Views/Monitor/InferenceViews.swift`).
  Bucket (d) `inference_activity`, `inference_calls`, `inference_call_proof`,
  `near_ai_spend_organizations`.
- **Insights:** benchmarks, skills, skill learning, the breakdown inspector,
  submitted traces and sent-traces training (`Views/InsightsView.swift:205-207`).
  Bucket (b) `tc_insights_copy_json`, `tc_skill_learning_copy` and others.
- **History:** the detail inspector (`Views/Monitor/HistoryInspector.swift`)
  and public-run publishing. Bucket (d) `history_detail`,
  `publish_public_run`.
- **Preview sheet:** turns, unsure spans and the redaction summary. Bucket
  (b) `tc_preview_turns_json`, `tc_redaction_summary_json`.
- **Missions,** a Home sub-page (`Views/Monitor/MissionsViews.swift`).
- **The About window** (`Views/AboutWindowView.swift`).
- **Keyboard and accessibility names,** as acceptance items across every
  slice. The Mac sets `accessibilityLabel` at 155 sites; Windows sets
  `AutomationProperties` at 40.

Decoration with no slice:

| Item | Mac source | Windows today | Status | Weight | Numbers to pin | Decision |
|---|---|---|---|---|---|---|
| Skill cover art | TraceCommonsApp/Views/InsightsSkillsCover.swift:175 gradient, :179 Canvas SkillCoverArt, :199 text shadow | gradient greps 0 (Z2); grep -i skill in Controls/InsightsView.xaml: 0 lines | missing | medium, M |  | G-4 |
| Insights bars grow and numbers roll | TraceCommonsApp/Views/InsightsSkillsHomeView.swift:196 expand, :343-344 and :372-373 numericText, :440-443 hover, :472-495 grow | static Rectangle bars (Controls/InsightsView.xaml:57-58, 119-120, 136, 189, 282-283); animation greps 0 (Z1) | missing | medium, S |  | G-4 |
| Clock-driven text refresh | TraceCommonsApp/Views/InsightsUsageCards.swift:45 (60 s), BalanceRow.swift:82 (30 s), Monitor/InspectorPrompts.swift:190 (1 s) | only DispatcherTimer is the 10 s poll in Controls/ManagedSessionsView.cs:20 | missing | low, S |  | G-4 |


## Corrections to the guide (#2190)

Checked at 19b42d0d2:

- `GlassThreePane` is used at `MonitorWindowView.swift:302`, not `:259`.
  The pane numbers are at `ThreePane.swift:44` and `:48`; seeding is at
  `:204-207`.
- The wording guards are at `ShellWordingTests.cs:192` and `:249`. The
  baseline is 40 files and 337 sentences.
- The MSIX manifest has four "Near.ai" literals (`Package.appxmanifest:32`,
  `:75`, `:92`, `:109`), not one.
- The rail has six entries, not four.
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
  adds a test that the manifest's display names equal `APP_NAME`.
- **Tests that read XAML as text** (`TraceCommons.Interop.Tests.csproj:163-178`)
  move with `MainWindow.xaml` and `SettingsView.xaml`.
- **Keyboard accelerators** (`MainWindow.xaml:44-49`) change with the tabs.
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
- The interop suite, the `windows contributor app` job, the contributor
  crate tests, fmt and clippy pass. A final run on the test box is recorded.

## Owner decisions

All pending decisions, by slice. Each slice above repeats its own.

- G-1 (General): Unfreeze and which Windows client
- G-2 (General): Look target
- G-3 (General): App mark
- G-4 (General): Surfaces no slice covers
- G-5 (General): Tooling for the Azure test box
- W1-D1 (W1): Where captures run
- W1-D2 (W1): Render tests in CI
- W2-D1 (W2): Rail destinations
- W2-D2 (W2): Settings window
- W2-D3 (W2): Window growth at the screen edge
- W2-D4 (W2): Notices and inspector demand
- W2-D5 (W2): Keyboard map
- W3-D1 (W3): Tool logos
- W3-D2 (W3): Private AI map view
- W4-D1 (W4): Transfer cards at the map foot
- W5-D1 (W5): Glass material
- W5-D2 (W5): Glyphs and icons
- W5-D3 (W5): Stock controls
- W6-D1 (W6): Map style switch
- W7-D1 (W7): Wallet sign-in in first run
- W7-D2 (W7): Device words for first run and projects
- W9-D1 (W9): Undo-window setting
- W9-D2 (W9): Arming without an account grant
- W9-D3 (W9): Where an override is cleared
- W10-D1 (W10): Internal ids
- W11-D1 (W11): Art where the Mac samples an SF Symbol
- W11-D2 (W11): Walkthrough card
- W11-D3 (W11): Remote campaign feed and image
- W12-D1 (W12): Tray icon content
- W12-D2 (W12): Tray badge
- W12-D3 (W12): Tray news ring and idle halo
- W13-D1 (W13): Tray flyout and edge rail
- W14-D1 (W14): Terminals
- W14-D2 (W14): "Show" (raise a running session)
- W14-D3 (W14): Per-tool launch buttons
- W15-D1 (W15): Background service

