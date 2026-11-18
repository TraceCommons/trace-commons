# Bringing the Windows app to Mac parity: a guide from the Linux port

Status: guide, 2026-11-18. Nothing here is implemented, and nothing here
approves anything. Facts about the code are cited at `origin/main` 202e4cdd5.

Who it is for: whoever ports the Windows app (`windows/`) to the native macOS
app next, people or agents. It covers what the Linux GTK port (November 2026)
taught, where the Windows app stands today, a slice plan, and the
Linux-to-Windows mappings.

How claims are marked:

- **[read]**: read in source at 202e4cdd5, with a file:line.
- **[PR]**: taken from a PR body or the Linux working reports. Many of those
  PRs are still open drafts, so cite them by number, not by paths on `main`.
- **[inferred]**: reasoned from the code or from platform knowledge, not
  observed. Check it before relying on it.

## 0. Before any work: two facts

1. **Windows development is frozen.** On 2026-11-11 the owner unfroze the
   Linux GTK app for development only; Windows and Tauri stayed frozen
   (`CLAUDE.md`, "What this repo is"; #2084). Unfreezing Windows is owner
   decision zero. Do not start a slice on the strength of this guide.
2. **Most of the Linux stack has not landed.** Merged: #2159 (spec), #2168,
   #2170, #2172, #2176, #2180. Open drafts: #2160, #2161, #2162 (spike, not
   for merge), #2163, #2164, #2165, #2171, #2175, #2177, #2179, #2181, #2182,
   #2184, #2185, #2186, #2187, #2188. Several parts have never run where they
   matter: KDE blur, the GNOME Shell extension (#2188), the scoped dark stage
   on a GTK 4.16+ box, and the Flatpak runtime-48 portal round trip. Read the
   Linux PRs as a record of method, not as finished, landed code.

## 1. Where Windows stands today

The Windows app is a WinUI 3 shell over the same C ABI the Mac uses
(`windows/README.md`). Project facts [read]:

- `TraceCommons.App.csproj:5-6`: `net8.0-windows10.0.19041.0`, minimum
  platform 10.0.17763.0.
- `:58-59`: Windows App SDK self-contained.
- `:114-115`: the only packages are `Microsoft.WindowsAppSDK` 1.6.240923002
  and `Microsoft.Windows.SDK.BuildTools`. There is no Win2D and no Community
  Toolkit.
- The interop project and its tests target plain `net8.0`
  (`TraceCommons.Interop.csproj:19`, `TraceCommons.Interop.Tests.csproj:4`),
  so they run on macOS and Linux against a `.dylib` or `.so` build of the FFI.

### Surface by surface

| Surface | Mac (parity target) | Windows today | Mark |
|---|---|---|---|
| Window and shell | `GlassThreePane` main 440 \| map 440 (min 360) \| inspector 293 (`TCDesign/Components/ThreePane.swift`, used at `Views/MonitorWindowView.swift:259`). Toolbar row with Graph/Map/Inspector toggles and a gear; tabs Home, Inference, Traces, Insights; Settings in its own window | A 184 px nav rail plus one content column: `MainWindow.xaml:107` `ColumnDefinitions="184,*"`. Panes swapped as ContentPresenters: History `:898`, Private inference `:919`, Insights `:924`, Mission drafts `:927`, Settings `:930`. The queue is the default pane. No Home, no inspector, no map. Settings is a pane, not a window. The title bar is customised (`MainWindow.xaml.cs:129-132`, `ExtendsContentIntoTitleBar`) | [read] |
| Map | `Views/Monitor/FlowMapScene.swift`, `FlowMapView.swift`, `MapToolPile.swift`; the station and orb in `TCDesign/Components/LibraryStation.swift`, `LibraryOrb.swift` | None. No map, graph, station or orb. Canvas appears only in 16 px icons and `Controls/BrandMark.xaml` | [read] |
| Transfers and motion | `MapTransferFeed.swift`, `MapTransfers.swift`, `TraceTransferCard.swift`; motion tokens in `TCDesign/Styling/GlassMotion.swift` | Nothing animates. No Storyboard, no Composition animation, no read of the system's animation setting. The only timers poll (`Controls/ManagedSessionsView.cs:20`) | [read] |
| Look and tokens | Flat glass theme from `design-tokens/glass.tokens.json` via `TCDesign/Generated/GlassTokens.swift`; live material in `Styling/GlassBackdrop.swift` | `scripts/design-tokens/generate.py:51-63` emits only the 11 `SHELL_ROLES` (accent and status colours) to `Themes/BrandTokens.xaml`, plus 11 legacy keys (`:539`). No glass tiers, radii, spacing, type, shadows or motion. `Themes/DesignSystem.xaml` is hand-written, "transcribed from design-import/DESIGN-SPEC.md". No `SystemBackdrop`, Mica or Acrylic anywhere in the app (grep: 0 hits). Fonts: Segoe UI Variable, Segoe UI, Consolas. Icons: Segoe `FontIcon` glyphs and hand-drawn Canvas shapes | [read] |
| First run | `Views/FirstRun/` (Welcome, Tools, Folders, Private AI, Traces, Uses, Rules, Terms, Join, and the animated `FirstRunStage.swift`); state in `TCShellCore/FirstRun/` | `OnboardingWindow`, five steps: `enum OnboardingStep { Welcome, Connect, Consent, Watch, Done }` (`ViewModels/OnboardingViewModel.cs:675-681`). No stage, no NEAR mark, no Terms or Private AI step. This is the shape Linux had before #2177 replaced it | [read] |
| First-run animated stage | `FirstRunStage.swift`: pen-drawn N with shine, breathing hub, tool ring and scan, orbs, thread, spatial spring between steps | None | [read] |
| Consent | `Views/Settings/SharingChoiceCard.swift`, `ConsentSection.swift`, `ProjectsSection.swift`; `PreviewSheet.swift` | Preview in `Controls/PreviewSheet.xaml`, hosted by `PreviewWindow.cs`. Withdraw in `Controls/WithdrawDialog.cs`. An undo-window setting, `NumberBox ApprovalHoldSeconds` (`Controls/SettingsView.xaml:279`), which the Mac does not have: Linux removed its own in #2179. Per-project modes only; no sharing-choice card was found | [read]; "no grant UI" is [inferred], the arming path was not traced |
| Daemon hosting | In-process daemon, `TraceCommonsApp/DaemonHost.swift` | In-process too: `DaemonHost.cs` starts `TcDaemon` over the FFI. Fail-closed roots: `DaemonHost.cs:181-186` says the daemon refuses to start without both sources declared, and the shell routes the refusal to `SessionRootsWindow`. State in `%LOCALAPPDATA%/trace-commons` | [read]; the Rust refusal itself is [inferred] from the comment |
| Run at login, install paths | `LoginItemManager.swift` | `RunAtLogin.cs` (HKCU Run, unpackaged) and `StartupRegistration.cs` (StartupTask when packaged). `AppUpdater.cs` uses the App Installer feed when packaged. MSIX manifest `windows/packaging/Package.appxmanifest`. Three install paths: MSIX App Installer, portable zip, CLI via `install.ps1` | [read]; install paths from memory notes |
| Launch and terminals | `ManagedSessionsView.swift`, `Settings/ManagedTerminalPicker.swift` | `Controls/ManagedSessionsView.cs`, `Controls/ManagedTerminalPicker.cs`, filled from `managed_snapshot`. The list is in Rust: `crates/trace-commons-contributor/src/managed/terminal.rs:73`, `WINDOWS: [TerminalApp; 1] = [TerminalApp::WindowsTerminal]`. Linux offers ten (`:77`, `LINUX: [TerminalApp; 10]`) | [read] |
| Brand name | The core's `brand.rs`: `app_name!()` is "Near AI Gateway" (`crates/trace-commons-contributor/src/brand.rs:30`). No `Near.ai` literal in `macos/Sources` outside comments | "Near.ai", the former name (`brand.rs` `FORMER_APP_NAMES`), hard-coded: 25 occurrences in 14 files under `windows/src`, for example `MainWindow.xaml:29` (window title) and `:97` (title bar). Also `Package.appxmanifest:32` `DisplayName`. No FFI call returns the app name | [read] |
| Decoration | NEAR N (`TCDesign/Logos/GlassBrandMark.swift`), tool logos (`GlassLogoPaths.swift`), hero cards (`Components/HeroCard.swift`), pixel art, coin burst, station, orb, map field | `Controls/BrandMark.xaml`: the "Turn" mark, pure geometry, no gradients. No gradients, glows, halos or shadows in `BrandMark`, `DesignSystem.xaml` or `MainWindow.xaml` (grep). No Home, no hero cards, no tool logos | [read] |
| Tray | Menu-bar strip with N, badge, ring and halo (`Components/MenuBarPanel.swift`); glass popover (`Views/Monitor/MenuBarGlassPanel.swift`) | `TrayIcon.cs`: Win32 `Shell_NotifyIcon` and a native popup menu (`CreatePopupMenu`, `TrackPopupMenuEx`). Four states (`TraceCommons.Interop/TrayModel.cs:17-29`: Idle, Paused, Unhealthy, Attention); a dot for Attention and Unhealthy. No ring, no halo, no flyout, no popup window | [read] |
| Edge rail | `Views/EdgeRail/EdgeRailController.swift`, `EdgeRailView.swift` (off by default) | None. No topmost or edge window | [read] |
| Glass | Liquid Glass material under a veil; painted under Reduce Transparency | Opaque `TcWindowBackgroundBrush` (`MainWindow.xaml:31`) | [read] |

### Guards Windows already has

These are worth knowing, because a port must keep them green [read]:

- `ShellWordingTests.cs:191`
  `NoWordingIsAuthoredInThisShellBeyondTheRecordedBaseline`. The baseline
  (`:50-123`) is a ceiling and a floor: adding or removing a counted sentence
  both fail. `TheRustOwnedSurfacesAreNotGivenAWordingAllowance` (`:248`).
- `SourceCheckTests.cs:138` `TheSettingsScreenAsksForTheRowRatherThanWritingIt`.
- `NoCommonsNameTests.cs:45` `NothingThisShellShowsNamesTheCommons`. It scans
  C#, XAML and the MSIX manifest. The one allowed literal is the HKCU Run value
  name.
- `XamlCommentSyntaxTests.cs:39`: no `--` inside a XAML comment, which would
  otherwise fail only in the `windows contributor app` job.
- `BrandTokensTests.cs`: contrast ratios and high-contrast aliases for every
  brand key.
- Per-surface "no wording authored" tests for consent, eligibility, witness
  and Private AI.

What is not guarded [read]: nothing compares the 110 `DllImport` declarations
in `TraceCommons.Interop/NativeMethods.cs` with the Rust exports.
`crates/trace-commons-contributor-ffi/tests/abi_header_surface.rs` checks only
the two C header copies. A drifted binding fails at call time on Windows.

## 2. The order of work that worked on Linux, and what to change

### 2.1 Spec first

**Linux.** An earlier parity pass (the 2026-11-11 workflow,
`docs/superpowers/plans/2026-11-11-linux-parity-workflow.md`) matched
structure and tokens. The owner then launched the app and said it "doesn't look
like the mac app at all". The 2026-11-16 spec (#2159,
`docs/superpowers/specs/2026-11-16-linux-mac-window-and-map-design.md`) reset
the target to "as close to the Mac as possible". It read the Mac's numbers from
source, listed what the change breaks, named the spikes, and asked four owner
questions before any code.

**Windows.** Write one spec of the same shape before slice 1. Start from the
Mac's look, not from a shared visual family. Reuse the Linux spec's "What the
Mac window is" section; it is Mac-side fact. Add a Windows column for each
decision: title bar, glass, fonts, icons, tray, packaging. The owner also
decided that window and map logic is ported per shell, not moved into shared
Rust, because windowing drifts by platform. Windows reproduces it in C#.

### 2.2 Recorded samples and a capture gallery before any look work

**Linux.** #2161 did three things:

- GTK reads the Mac's recorded daemon replies from
  `macos/Sources/TCShellCore/DataContract/RecordedSamples/` (sets `empty`,
  `normalDay`, `busyQueue`, `heldSessions`, `armedFolder`, `unknownCounts`),
  chosen by `TRACE_COMMONS_SAMPLE`, in debug builds only.
- The Mac's debug hook (`TraceCommonsApp/DebugScreenshot.swift`) renders the
  whole Monitor window per sample, tab, pane set and appearance.
- `scripts/parity/gallery.sh` (on #2161, not yet on `main`) pairs the two
  sides into one HTML page.

Every later look claim was checked against that gallery.

**Windows.** Do the same first, with these differences:

- Add a debug-only sample client in the interop layer that answers `tc_call`
  from the same files, refuses every other method as unrecorded, and never
  starts a daemon. Make it impossible in Release, as on the Mac and Linux.
- Put it in `TraceCommons.Interop` (plain `net8.0`) so its tests run on macOS.
- Capture WinUI on the dev VM with `windows/scripts/win-capture.ps1`. WinUI
  cannot run from the SSH session (session 0); it dies with a stowed exception
  (`windows/docs/dev-vm.md`).
- Name captures with the gallery's pattern,
  `<set>-<tab>-<panes>-<appearance>.png`, so `gallery.sh` pairs them unchanged.

Known limits of the Mac side, learnt on Linux:

- The XCTest process defaults to the `classic` theme. The app ships `flat`
  (`TCDesign/Tokens/GlassTheme.swift`). Render with `TC_GLASS_THEME=flat`, or
  you will chase a purple that the shipping app does not draw.
- `ImageRenderer` draws content inside a `ScrollView` blank (tab bodies, the
  tree, the inspector), and draws glass as painted. Compare those areas from
  source.
- The samples record no `account_devices_list` and no Private AI network reads
  (`macos/SAMPLE_DATA.md` on #2161).

### 2.3 Shell, then map, then transfers, then look, as a stack

**Linux.** Four stacked PRs: shell #2160, map #2163, transfers #2164, look
#2165. Each slice ran only `cargo check` while it was written, per the repo's
multi-task rule, and the full suites ran once at the end. Fix passes went in
on the base and were carried up by merge.

**Windows.** Keep the order. The shell decides where everything else sits.

- **Shell.** Port `GlassPaneLayout` as one pure function: main 440, map 440
  (min 360), inspector 293, 10 px gaps. Build the toolbar row inside a custom
  title bar. Windows already has `ExtendsContentIntoTitleBar`; keep the system
  caption buttons, as Linux kept the desktop's own.
- **Window growth.** On the Mac a pane opening grows the window at its right
  edge and slides it left at the screen edge (owner ruling, Ron, 2026-10-30).
  Wayland would not let GTK do this. Win32 can, with `AppWindow.Resize` and
  `Move`, so Windows can match the Mac exactly [inferred].
- **Inspector demand.** Port the Mac's `InspectorDemand` keys
  (`Monitor/MonitorShell.swift:46-66`). The owner approved the inspector
  opening itself, transfer cards at the map's foot, and notices laid over the
  tab, as on the Mac (spec, decision 1).
- **Notices.** Linux tried a column above the tab, and the owner sent it back
  ("b") to the Mac's peeking stack laid over the tab. Start with the Mac's
  overlay and its X rules (#2160, fix passes 2 and 3).

### 2.4 First run and consent

**Linux.** The owner said "FTUX parity is needed": step for step, with wallet
sign-in dropped on Linux (#2177). The animated stage came later and
separately (#2185). For consent, the owner said "match the mac" (#2179):

- Port every Mac consent control.
- Remove the Linux-only undo-window setting, which could be set to 0 s.
- Settle each daemon reply against the action that was pressed, not
  whatever a single slot holds when the reply lands.
- Nothing more permissive than the Mac.

**Windows.** The same three slices apply, with these changes:

- The Windows `OnboardingWindow` is the legacy Welcome, Connect, Consent,
  Watch, Done shape. Replace it, as #2177 replaced GTK's.
- Remove the `ApprovalHoldSeconds` setting (`SettingsView.xaml:279`), as
  #2179 did on Linux. Ask the owner first; it is the same decision.
- The Mac forces its first-run stage dark. WinUI can scope a theme to one
  window with `FrameworkElement.RequestedTheme` on its root, so the
  app-wide fallback GTK 4.14 needed is not required [inferred].
- Port the stage only after the steps. Pin its numbers with the Swift probe
  (section 3.2).

### 2.5 Background service and daemon hosting

**Linux.** The Flatpak sandbox could not open host terminals. The owner chose
"Flatpak = host daemon" (#2171):

- The window attaches over a socket to the CLI's systemd user service.
- One `watcher.lock` per login keeps two daemons from watching the same
  sessions.
- Three owner-approved dialogs cover the service being not installed, not
  running, or too old.

The words are approved and pinned in `managed::copy::background_service` on
that branch.

**Windows.** The analogue is different, and you must establish it before
copying anything:

- The Windows app is not sandboxed. The MSIX declares `runFullTrust`
  (`Package.appxmanifest:116-125`), and the portable zip is a plain process.
  It hosts the daemon in-process (`DaemonHost.cs`), as the Mac does. There is
  no Windows reason for a host-daemon split [inferred].
- Check what the CLI's `daemon run` does on Windows when the app's daemon is
  also running. The `watcher.lock` from #2171 lives in `$XDG_RUNTIME_DIR` and
  is Linux-only as written. Whether two Windows daemons can watch the same
  `%USERPROFILE%` sessions today is unverified. Find out on the dev VM before
  deciding.
- The transport is a named pipe. Its DACL is the only access control on the
  daemon IPC on Windows. The `verify-pipe-acl.ps1` step in CI is the only
  evidence for it (see section 4).
- Run at login and updates exist already, with two code paths each: HKCU Run
  or StartupTask, and the App Installer feed only when packaged. Every
  install-path change must keep both flavours working. CI asserts the default
  build stays unpackaged (`clients.yml`, the `windows contributor app` job).
- If the owner does want a background-service flow on Windows, the dialogs
  are new notices on a new platform. They need approval of the notice and its
  words, even though Linux has approved words.

### 2.6 Terminals and launch

**Linux:**

- Added four terminals (#2170), each started from an argv vector so no shell
  parses the folder or the ticket.
- Refused folders a terminal would mangle (Ptyxis and `%#?`, after the
  Windows Terminal `;` precedent).
- Made the launch line name the picked terminal (#2168).
- Left "Show" (bring a session to the front) Mac-only, because Wayland gave
  no safe way to raise another app's window.

**Windows.** Windows offers only Windows Terminal (`terminal.rs:73`). Any
second terminal is a Rust core change in `managed/terminal.rs`, with the same
argv test discipline and a real-launch smoke on the dev VM. Which terminals to
offer (PowerShell's own console host, `cmd`) is an owner call. Win32 can raise
a window it can find, so "Show" may be feasible on Windows; spike it, do not
assume it.

### 2.7 Brand

**Linux.** The owner ruled that "Trace Commons" never appears anywhere a user
can see; use the brand. The Rust code already read `brand::APP_NAME`; what
was left was packaging text that cannot read `brand.rs` (#2184). Tests now pin
the desktop entry, the store name and the flatpakref title to `APP_NAME`, and
the guard catches the name wrapped across two lines.

**Windows.** Windows has the opposite problem. It already avoids "Trace
Commons" (`NoCommonsNameTests.cs`), but it hard-codes the former name
"Near.ai" in 25 places and in the MSIX `DisplayName`. The fix has three parts:

- An FFI surface that returns `APP_NAME`. It does not exist today.
- Shell reads in place of every literal.
- A test that the manifest's `DisplayName` and `VisualElements` equal
  `APP_NAME`, as #2184 did for the desktop entry.

The owner has to decide what stays as an internal id: the `Iqlusion.TraceCommons`
package name, the `tracecommons` protocol, `TraceCommons.exe`, the Run value
name, and the install directory. Linux kept every internal id.

### 2.8 Decoration: inventory first, not last

**Linux.** The flows, words and layout were ported first. The station, the
logo coins and the first run's NEAR mark were caught late, by the owner, side
by side with the Mac: "I feel like decorative elements in Mac built have been
under emphasized in the port". The inventory that followed listed 67 items:
26 missing, 20 partial, 11 ported and 10 not applicable.

**Windows.** Inventory the decorative layer in the first week, against a
stated `main` SHA, and attach it to the spec. Go through:

- drawn marks and logos;
- glows, halos, gradients, orbs, shines and fields;
- breathing and pulse animations;
- transitions between steps;
- hero cards, pixel art and the coin burst;
- the tray badge, ring and halo.

Two lessons from that inventory:

- **Pin the SHA.** #2187 found that the inventory quoted the hero-card
  gradients from before a Mac cleanup (fc4ecc1c6): the Mac now ships solid
  grounds and no radial light. Confirmed at 202e4cdd5:
  `TCDesign/Generated/GlassTokens.swift:405` gives `heroCampaignCard` #081538
  at both stops. #2187 ported what the Mac ships, from the
  tokens.
- **"Not applicable" is an owner call.** The inventory marked the edge rail
  and the menu-bar popover not applicable on Linux. The owner then approved
  both ("approve both"; #2188). Never mark a Mac surface not applicable on
  your own.

## 3. Rules that kept parity honest, and mistakes to avoid

### 3.1 Decoration and motion are parity

Port them with the Mac's numbers. Never defer them as a later "look slice"
without saying so and getting a decision.

### 3.2 Pin the Mac's numbers with throwaway Swift probes

Linux pinned Mac values by compiling the Mac's own Swift in a scratch SwiftPM
package and printing the numbers:

- `FirstRunStage.swift`, `WelcomeScreenLayout`, `ToolsRingLayout` and
  `GlassBrandMark`, with TCShellCore types stubbed (#2185);
- `EdgeRailGeometry` (#2188);
- `GlassMenuBarStrip` (#2186);
- the hero pixel art (#2187).

The tests then pin those values, to 1e-6 where they are numbers. The probe is
never committed.

Write tests that fail on a wrong port, not tests that restate the
implementation. #2181's review replaced five tests that matched CSS text with
pixel tests that paint the real functions and read pixels back, and with
literal Swift values cited by file and line. Each new test was run against the
old behaviour first and failed. Linux used these red-first and mutation checks:

- a wrong spring damping;
- the wrong ring order;
- a halo inset of 3 instead of 2.5;
- a native byte order where the tray needs ARGB big-endian.

On Windows, put the pure ports (pane layout, map scene, transfer player, stage
layout, tray badge) where the plain-`net8.0` tests can reach them. Then they
run on macOS beside the probe, without the VM.

### 3.3 Render offscreen on both sides and diff

Linux compared cairo renders of the real paint functions with the Mac's
`ImageRenderer` renders of the same samples, and reported numbers:

- the station: mean 1.3 to 1.6 of 255 per channel;
- the Private AI map: mean 3.3 to 3.5, with words and grounds masked;
- the first-run stage: alpha IoU at least 0.96.

On Windows the offscreen path is WinUI's `RenderTargetBitmap` or a Composition
capture in the interactive session [inferred, not built]. Mask text before
diffing, because fonts differ.

### 3.4 Read the shared core copy only

- No English authored in the shell. Windows enforces this already with a
  wording baseline that must go down, not up.
- No new notices without the owner approving both the notice and its words
  (`docs/ui-copy-guide.md`, rule 9). A spec that calls for a notice is not
  approval.
- The Mac's own words name "this Mac". The owner approved "this computer"
  variants for Linux (#2163, #2177); Windows can use the same fields.

The biggest Linux-to-Windows difference is how words arrive. GTK links the
contributor crate in-process and reads Rust constants directly. Windows gets
words only through the C ABI. Several strings approved during the Linux work
were deliberately kept out of the export:

- the background-service dialog words, typed constants outside the `copy()`
  JSON (#2171);
- `appearance_copy`, outside `ShellWordsCopy` and the C ABI (#2175).

So a Windows slice that needs a word Linux already shows may first need export
work:

- a Rust field;
- both C header copies, kept identical (`abi_header_surface.rs` enforces it);
- the Swift decode, whose export contract test fails if the field is
  unexpected;
- the C# decode;
- the pinned counts.

`tc_monitor_traces_copy_json` is in the header but not bound in
`NativeMethods.cs` [read]. Expect gaps like that.

### 3.5 Port the flat theme, not classic

The Mac ships `flat`; `classic` exists for tests and the gallery. Under flat,
every tier gradient has equal stops and cards and controls have no edge
layers. Linux withdrew a colour "gap" (G17) once the Mac render was redone
under flat.

### 3.6 Take decisions to the owner as concrete options

Every owner decision on Linux was a short list of options with a
recommendation:

- notices over the tab (a) or above it (b);
- GNOME 48 or a newer runtime;
- Standard or Omarchy;
- "Read host + normalize".

Answers came back as a letter or a phrase, and were then recorded beside the
code and in the PR body. Do the same. Mark every DRAFT word as DRAFT in the
core until it is approved.

### 3.7 Never show "Trace Commons"

Use the brand from `brand.rs`. Internal ids stay. Packaging text counts as
user-visible: the MSIX `DisplayName`, the App Installer page, the Start menu
entry, notifications and the tray tooltip.

### 3.8 Consent fails closed

- An unread signal never draws a flow.
- A failed read is a dash, never 0.
- Nothing is preselected.
- A reply that does not confirm the write leaves the old state.
- A refusal keeps the card.
- Never is absolute.

#2179's review checked each Linux control against the Mac and found nothing
more permissive. Hold Windows to the same review.

### 3.9 Reduce motion, and timers on hidden windows

Every animated surface has a still frame and stops its clock under reduced
motion. Every clock stops when its surface is hidden:

- the map's clock runs only while the canvas is mapped (#2182);
- the stage caps at 30 fps and ticks only while shown (#2185);
- the hero art runs at 10 fps only while its card is on screen (#2187).

A mounted test asserts no ticks and no draws while hidden. On Windows, read
`UISettings.AnimationsEnabled` and stop timers on window visibility and
minimise [inferred].

### 3.10 Owner calls, not yours

- **Trademarks and logos.** The owner allowed tool logos "as needed" on
  Linux. Reuse exactly the Mac's path data from `GlassLogoPaths.swift`,
  record its source in the PR, and ask before extending it to Windows.
- **The app mark.** Linux moved to the NEAR AI N for the app, tray and
  Flatpak icon (#2186). The Turn geometry stays in `trace-commons-mark` only
  "for the Windows tiles". Whether Windows moves to the N is a decision.
- **SF Symbols.** These are Apple-only. Linux used stock Adwaita symbolic
  icons for the graph range and inspector glyphs, and hand-drew pixel art
  where the Mac samples an SF Symbol. Nothing was traced from Apple art. On
  Windows the candidate is Segoe Fluent Icons, mapped in one checked-in
  table; the mapping needs approval.
- **Fonts.** In August 2026 the owner ruled against bundling Inter in the
  apps. On 2026-11-16 he chose Inter (OFL) for Linux, pending a dependency
  write-up. #2165, still a draft, leaves it out. Ask before bundling anything.
- **New dependencies.** Win2D, a UI Automation driver library, the Windows
  Community Toolkit. Give the full write-up: adoption, transitive count,
  maintenance, licence, advisories.
- **Blur.** Mica and Acrylic need no new package; their use is still a look
  decision.

### 3.11 Re-check the premise before adding a dependency

The Linux blur spike (#2162) added five crates and 18 lock packages for a
KDE-only protocol. A later read of GTK 4.24 found that GTK binds the standard
`ext-background-effect-v1` itself from CSS `backdrop-filter`, with no crate.
The spike is kept as a record and not for merge. On Windows, check what the
Windows App SDK already gives before proposing a package.

## 4. Testing

### What Linux used

- **Real desktops on GCP.** GNOME (Ubuntu 24.04, GTK 4.14) and KDE (Ubuntu
  26.04, GTK 4.22), later an Omarchy box (Arch, Hyprland, GTK 4.24). Bugs that
  only showed on a real older toolkit:
  - buttons clipped on GTK 4.14 because of a cached request mode (#2160, fix
    pass 3);
  - a preview sheet cut off on libadwaita 1.5;
  - a keyboard trap in a flow box (#2179).
- **The weston job.** `linux-shell desktop integration (weston + portal)` in
  `clients.yml` runs headless Weston, an OCR gate, and every mounted
  `#[ignore]` test listed in `weston-portal-verify-inner.sh`. A mounted test
  not on that list never runs in CI.
- **AT-SPI.** It drove dropdowns and read accessible text dumps where a
  capture could not reach.

### Windows equivalents

- **The dev VM.** A Windows Server 2022 GCE box with repo tooling. Use it; do
  not build a harness. `windows/docs/dev-vm.md` has the setup.
  - `windows/scripts/win-exec.sh` runs PowerShell on the box over IAP.
  - `provision-dev-vm.sh` has `create`, `start`, `stop`, `status`, `rdp` and
    `delete`.
  - `win-capture.ps1` launches the app in the interactive session and
    screenshots it.
  - SSH is off until `win-enable-ssh.ps1` runs.
- **Windows CI jobs** (`.github/workflows/clients.yml`; none is a required
  check, and they run on a PR only when it touches a client path). The
  `CLAUDE.md` list of job names is out of date: three of them were merged
  into one.
  - `windows contributor crates (tests, named-pipe ACL, update conformance)`
    (`:229`):
    - the contributor, sources, insights, platform and FFI crate tests on
      `windows-latest`;
    - real Credential Manager round trips;
    - Authenticode update conformance;
    - the named-pipe ACL step;
    - the FFI DLL artifact.
  - `windows contributor app` (`:604`): the MSIX manifest checks, the interop
    tests against the real DLL, and the packaged build with its App Installer
    feed. It is the only thing that compiles the WinUI project, and so the
    only thing that catches a XAML compile error.
  - `install.ps1 against the real release` (`:334`).
- **UI Automation** is the AT-SPI analogue. Set `AutomationProperties.Name`
  from core words, and check names on the VM with the UIA client in
  PowerShell [inferred, not built]. A driver library (FlaUI, WinAppDriver)
  would be a new dependency and an owner call. No shell has UI automation
  today.
- **The named-pipe ACL job is not a smoke test.** It creates a second,
  non-administrator account and requires it to be refused at the daemon's
  pipe. Do not weaken it. Rerun it when anything touches the daemon's IPC
  hosting.
- **A stale FFI DLL fakes results both ways.** Rebuild
  `trace-commons-contributor-ffi` in your own worktree and point
  `TC_FFI_LIB_DIR` at it before trusting any interop result. Two stale
  artifacts once agreed with each other and passed (106 vs 106).
  `windows/global.json` pins .NET SDK 8. On a Mac with a newer default
  `dotnet`, set `DOTNET_ROOT`.

### Box hygiene

- Never sign in, enroll or share on a test box. On Linux the owner signed in
  himself over RDP when a real NEAR AI launch was needed. The boxes'
  first-run walks stopped short of Start sharing, and `terms_accept` was
  never sent.
- Clean up after each run:
  - disable and remove any service you installed;
  - delete the test CLI and test folders;
  - restore the colour scheme;
  - stop helper processes;
  - leave no app instance running.
- Watch disk. GTK `target/` directories ran to 13 GB per checkout, and the
  owner later allowed the Linux boxes' disks to grow to 200 GB. That ruling
  covers resizes only; any other GCP change is the owner's. Move `target/`
  between checkouts; never copy it.
- The Windows box is licensed per hour. Stop it when done and confirm it is
  terminated through the API.
- Capture only the app's own window, located through the accessibility API
  at the moment of capture. Never take a full-screen capture or use remembered
  coordinates on a machine someone is using.
- Hand-edited sample data on a box is reverted afterwards and never
  committed.

## 5. Process and merge mechanics

- **Stacked PRs.** Linux used one base (#2160) and children by merge. Repo
  facts:
  - `main` merges through a merge queue whose method is squash (ruleset
    "main merge queue").
  - `delete_branch_on_merge` is on. Merging a base deletes its branch and
    auto-closes every child PR, and a closed child cannot be reopened or
    retargeted.
  - So collapse a stack top-down into its base, or retarget every child to
    `main` before the base enters the queue. Squash the base once.
  - After a squash merge, cherry-pick a child's own commits onto fresh
    `origin/main`; do not rebase.
  - Admin merge (`gh pr merge --admin`) needs the owner's say-so on that PR.
  - `gh pr merge` exit 0 is not proof of a merge; check the PR's state.
- **Cancelled runs read as failures.** `gh pr checks` shows every job of a
  cancelled run as `fail`. Twenty unrelated jobs red in two minutes is a
  cancellation. Check the run's status before reading a log.
- **macOS app tests.** The owner has said this job may be ignored. It has
  often hit its 45-minute limit with no failing test. That holds for PRs that
  change no Swift. A PR that touches Swift waits for it.
- **Shared-machine turn locks.** Builds and heavy test suites go through
  `~/.claude/turn-locks/turn.sh build|test "<label>" -- <command>`. Builds:
  at most three at once, four jobs each. Heavy suites, including the Windows
  interop suite: at most two at once. Never hold both. Rerun a timing test
  alone before calling it a failure.
- **Agents and workflows.**
  - Put the owner's authorization, verbatim and dated, in every agent
    prompt, because a workflow agent treats the latest user message as its
    request.
  - Never message an agent that is running inside a workflow. That resumes a
    second copy working in the same worktree.
  - Check that each task's commit exists before reporting it.
- **One worktree per branch.** Agents in parallel worktrees never share a
  stash. Take over a finished worktree (`git worktree move`) rather than
  copying a `target/`.
- **Conflicts in copy modules.** Do not union-resolve conflicts in copy
  modules such as `private_inference_copy.rs` or the shared copy tables. Git
  splits hunks at the closing braces and splices one block into another
  function. Reset to `origin/main` and re-apply by anchor from the pre-rebase
  commit. After any conflict, grep for markers before committing.
- **Cross-shell core changes reach Windows untested.** #2180 changed C#
  without a Windows runner; CI was the first compile, and it lowered the
  Windows wording baseline with a Python port of the counting rule. Expect
  the same whenever the core changes. Watch `main`'s client run after the
  merge and fix forward.

## 6. A concrete Windows plan

Slices are PR-sized and ordered. W1 to W4 are a stack; W5 can start beside
W2. Every slice follows the
repo's multi-task rule:

- while writing: a compile check only;
- at the end: the interop suite, the WinUI build on CI or the VM, the
  contributor crate tests for any core change, and fmt and clippy.

| # | Slice | Mac sources to port | Windows files touched | Pin in tests | Likely owner decisions, as options |
|---|---|---|---|---|---|
| W0 | Spec and decoration inventory | All of the below, read, not ported | A new spec in `docs/superpowers/specs/` | Nothing; cite file:line at a stated SHA | Unfreeze: (a) Windows now, (b) after a macOS release. Look: (a) Mac look, as Linux, (b) Fluent look with Mac structure. Title bar: (a) Mac toolbar row with system caption buttons, (b) standard title bar |
| W1 | Samples and gallery | `TCShellCore/DataContract/RecordedSamples/`, `DebugScreenshot.swift` | Debug sample client in `TraceCommons.Interop`; a capture driver run through `win-capture.ps1`; gallery naming | Sample client refuses unrecorded methods and never starts a daemon; compiled out of Release | None if no package is added. Capture runs: (a) on the dev VM only, (b) also as a CI artifact (the spec kept CI uploads off) |
| W2 | Window shell | `ThreePane.swift`, `MonitorWindowView.swift`, `Monitor/MonitorShell.swift`, `MonitorNotices.swift`, `TCShellCore/MonitorScreensCopy.swift` | `MainWindow.xaml(.cs)` rebuilt; a pure pane-layout class in the net8.0 project; `SettingsView` moved to its own window; tray and accelerators to the four tabs | Pane widths and gaps, first-launch seeding (map from 1100, inspector from 900), demand keys, notice stack order; `ShellWordingTests` baseline goes down | Rail destinations: (a) History and Mission drafts become sub-pages as on the Mac, (b) keep them in the rail. Window growth at the screen edge: (a) the Mac's slide-left, (b) panes reallocate in place |
| W3 | Traces map | `FlowMapScene.swift`, `FlowMapView.swift`, `MapToolPile.swift`, `TracesTree.swift` | New map control; scene and tree as pure C#; `tool_destinations` and `harness_list` reads | Port `FlowMapSceneTests` against the samples, as #2163 and #2182 did | Drawing: (a) XAML shapes and Composition, no new package, (b) Win2D, a new package with a write-up |
| W4 | Transfers and motion | `MapTransferFeed.swift`, `MapTransfers.swift`, `TraceTransferCard.swift` | Map control; transfer player as pure C# | Player on an explicit clock (5 s hold, stack of 6, 3 drawn); still frame under reduced motion; no ticks while hidden | Transfer cards: (a) the Mac's peeking stack, (b) a short column as on Linux #2164 |
| W5 | Look | `glass.tokens.json`, `GlassTheme.swift`, `GlassBackdrop.swift`, `Glyphs.swift`, `Navigation.swift`, `Indicators.swift` | `generate.py` extended to emit the flat tiers, radii, spacing, type and motion to XAML; `DesignSystem.xaml` reduced to what the generator does not cover; glyph paths exported | `generate.py --check`; contrast per tier in both schemes; pixel tests of the tab ember, status dot and graph matrix | Glass: (a) painted only, (b) Mica or Acrylic where supported with a painted fallback. Icons: (a) Segoe Fluent mapping table, (b) export the Mac's own glyph paths where they exist and Segoe for the rest. Font: (a) Segoe UI Variable, (b) bundle Inter |
| W6 | Map art | `LibraryStation.swift`, `LibraryOrb.swift`, `GlassLogoPaths.swift`, `GlassBrandMark.swift`, `GlassMapField` in `Indicators.swift` | Map control; generated logo and mark path data | Station face count 506 and corner count 2216 (#2182's pins), projected vertices from the probe, render diff | Logos: (a) the Mac's path data, as on Linux, (b) initials only. Map style switch: (a) in Settings as on Mac, (b) rocket only |
| W7 | First-run steps | `Views/FirstRun/*`, `TCShellCore/FirstRun/*`, `FirstRunCopy.swift` | `OnboardingWindow` replaced; `OnboardingViewModel` | Step order, `can_continue` per step, nothing preselected, terms sent once | Wallet sign-in: (a) drop, as on Linux, (b) keep. Device words: (a) reuse the approved "this computer" fields, (b) new "this PC" variants (new words) |
| W8 | First-run stage | `FirstRunStage.swift`, `WelcomeScreen.swift`, `ToolsScreen.swift`, `GlassBrandMarkDrawing.swift` | A stage control under the first-run window | Probe values for the spring, ring and marker check; reduced-motion final frame | App mark in the stage: (a) the NEAR N, as Mac and Linux, (b) the Turn |
| W9 | Consent | `Settings/SharingChoiceCard.swift`, `ConsentSection.swift`, `ProjectsSection.swift`, `PreviewSheet.swift`, `SessionReviewCard.swift` | `SettingsView`, `PreviewSheet`, `MainViewModel` undo | Undo settled against the pressed action; grant owner and account generation; bulk plan, confirm and stale; override shown with no clear | Undo-window setting: (a) remove, as #2179, (b) keep. Clear-override control: (a) none, as on the Mac, (b) add one |
| W10 | Brand | `brand.rs` | A new FFI read of `APP_NAME`; the 25 literals; `Package.appxmanifest` | Manifest `DisplayName` equals `APP_NAME`; no literal "Near.ai" in shell source | Internal ids: (a) keep all (package name, protocol, exe, Run value, install folder), as Linux did, (b) rename some, with a migration |
| W11 | Home and decoration | `Monitor/HomeViews.swift`, `HeroCard.swift`, `PixelGlyph.swift`, `PixelField.swift`, `CoinBurst.swift`, `BalanceArrivalFlourish.swift`, `HomeInsightsVisual.swift` | A new Home tab | Hero grounds from tokens; probe-pinned pixel masks; coin offsets; count steps | SF Symbol art: (a) hand-drawn pixel art, as #2187, (b) leave those cards without art. Walkthrough card: (a) leave out, as #2187, (b) port (it is a closable card under the notice rule) |
| W12 | Tray mark | `Components/MenuBarPanel.swift` (`GlassMenuBarStrip`), `TCShellCore/MenuBarStatus.swift`, `NudgeSurface.swift` | `TrayIcon.cs`, `TrayModel.cs`, `MarkRaster.cs` | Badge cap 99+, ring and halo only from `status.nudge.mark`, halo inset 2.5 | Mark: (a) NEAR N, (b) keep the Turn. Ring and halo: (a) port with the Mac's conditions and caps, as approved on Linux, (b) badge only |
| W13 | Tray flyout and edge rail | `Monitor/MenuBarGlassPanel.swift`, `MenuPanelStore.swift`, `EdgeRail/EdgeRailController.swift`, `EdgeRailView.swift`, `TCShellCore/EdgeRail.swift` | A topmost popup window from the tray click; an edge window | Rail geometry from the probe (#2188 pinned open (1436, 200, 484, 680) on 1920x1080) | (a) Both a tray flyout and the edge rail, as Linux, (b) the flyout only, (c) neither |
| W14 | Terminals and launch | `ManagedSessionsView.swift`, `Settings/ManagedTerminalPicker.swift` | Rust `managed/terminal.rs` for any new terminal; `ManagedSessionsView.cs` | Argv arrives intact for a hostile folder; a real launch on the VM | Terminals: (a) Windows Terminal only, (b) add PowerShell's console host, (c) add others after an argv check. Show: (a) spike it, (b) leave it Mac-only |
| W15 | Background service, only if needed | `DaemonHost.swift`, #2171 for the pattern | `DaemonHost.cs`, `RunAtLogin.cs`, `StartupRegistration.cs` | One watcher per login, if two daemons can collide | (a) No Windows background-service flow, (b) a CLI-owned daemon with approved dialogs |

W5 can start beside W2 once its decisions are made. W7 and W9 depend only on
W2. W11 depends on W2 and W5.

## Appendix: Linux-to-Windows mappings

The Windows column names APIs from platform knowledge, not from this repo or
from Microsoft's documentation read for this guide. Treat every entry not
already used in `windows/src` as [inferred] and check it before building on
it. Already in use: `ExtendsContentIntoTitleBar`, `SetTitleBar`,
`Shell_NotifyIcon`, theme dictionaries.

| Linux (GTK) | Windows (WinUI 3) | New dependency? |
|---|---|---|
| GTK CSS tokens in `brand_tokens.rs`, from `generate.py` | XAML `ResourceDictionary` theme dictionaries in `Themes/BrandTokens.xaml`, from the same generator | No |
| `@define-color` per scheme; `:backdrop` for an unfocused window | `ThemeDictionaries` Light, Default and HighContrast; `Window.Activated` with `WindowActivationState.Deactivated` for the inactive dim | No |
| libadwaita high contrast | `HighContrast` theme dictionary; `AccessibilitySettings.HighContrast` | No |
| cairo and GSK drawing in a `DrawingArea` | XAML `Path` and `Canvas`, or `Microsoft.UI.Composition` shape visuals (part of the Windows App SDK) | No |
| The same, with immediate-mode drawing | Win2D (`Microsoft.Graphics.Win2D`) | **Yes, owner approval** |
| Frame-clock tick | `CompositionTarget.Rendering`, or Composition animations | No |
| `gtk-enable-animations` | `UISettings.AnimationsEnabled` | No |
| GSK shadow for a glow | `ThemeShadow`, or a Composition `DropShadow` | No |
| Painted glass; KDE blur spike; `ext-background-effect-v1` via CSS | `SystemBackdrop`: `MicaBackdrop` (Windows 11 only), `DesktopAcrylicBackdrop`; a painted fallback when the system's transparency effects are off or unsupported | No [inferred from Microsoft docs; check on the target builds] |
| `gtk::WindowHandle` toolbar row with `gtk::WindowControls` | `ExtendsContentIntoTitleBar` plus `SetTitleBar` (already used, `MainWindow.xaml.cs:129-132`) | No |
| Wayland: no reliable resize or move of a mapped window | `AppWindow.Resize` and `AppWindow.Move` work | No |
| SNI tray icon, `IconPixmap` per size | `Shell_NotifyIcon` with an `HICON` per DPI (already in `TrayIcon.cs`) | No |
| SNI `Activate(x, y)` opens the panel at the click | Tray click message with the cursor position, then a topmost popup window | No |
| layer-shell rail and panel (KDE, Hyprland) | A topmost, tool-window, borderless `AppWindow` at the screen edge (`OverlappedPresenter` with `IsAlwaysOnTop`) | No |
| GNOME Shell extension that places windows | Not needed; Win32 places its own windows | No |
| Flatpak bundle and Flathub runtime | MSIX with an App Installer feed; the portable zip | No |
| Flatpak `finish-args` permissions | MSIX capabilities (`runFullTrust` today) | No |
| Document portal and `GetHostPaths` | Not needed; the app sees real paths | No |
| systemd user unit for the host daemon | In-process daemon, HKCU Run or StartupTask at login | No |
| Unix socket at `$XDG_RUNTIME_DIR` with `SO_PEERCRED` | Named pipe with a DACL (the CI ACL step) | No |
| AT-SPI | UI Automation, `AutomationProperties.Name` | No for names; **yes** for a driver library |
| Headless Weston and `weston-screenshooter` | The dev VM's interactive session and `win-capture.ps1` | No |
| Adwaita symbolic icons for SF Symbols | Segoe Fluent Icons glyphs | No package; the mapping needs approval |
| Inter, considered and not shipped | Segoe UI Variable today | Bundling a font is an owner call |
| `TRACE_COMMONS_SAMPLE` debug backend in GTK | A debug sample client in `TraceCommons.Interop` | No |

## What this guide did not verify

- That the CLI daemon and the app daemon can both watch on Windows, or what
  stops them.
- Which `SystemBackdrop` kinds work on the minimum platform 10.0.17763 and on
  the dev VM's Windows Server 2022 image. Mica is documented as Windows 11
  only.
- That `RenderTargetBitmap` renders the views a capture needs, and what it
  shows for a backdrop.
- Where Windows arms automatic contributing today. The arming offer path was
  not traced.
- The Windows API names in sections 2.3, 2.4, 3.9 and the appendix, other
  than those already used in `windows/src`.
- The owner's statement that the macOS app tests job may be ignored. It is
  recorded here as relayed, scoped to PRs that change no Swift.
