# Platform behavior and model limits

Source baseline: `8218cc8ae408eb9c2c8a16d5768d9f6e0f7f83d6`.
Native macOS is the reference client. The shared contributor daemon is modeled
once, but the four shells have materially different navigation, first run,
process ownership and platform integrations. This inventory records source
behavior, not installed/deployed platform availability or a parity certification.

| Area | Native macOS | Native Windows | Linux GTK | Tauri |
| --- | --- | --- | --- | --- |
| Navigation | Home overview/history/missions/drafts, Traces tree/history, Insights, inference, idle-session destination and section-specific Settings requests. `MonitorNavigation.swift:6`, `:26`, `:32`; `Views/SettingsSections.swift:11`. | Rail handlers for Waiting, History, Private inference, Insights, Mission drafts and Settings. `MainWindow.xaml:137`, `:222`, `:280`, `:327`, `:328`, `:336`. | Ordinary launch defaults to a daemon-free Insights surface. The full Contributions window has Insights, Queue, History, Private AI and Settings, plus conditional Mission drafts; that window initially opens Queue. `main.rs:142`; `ui/mod.rs:81`, `:137`, `:384`. | Router has Insights, Waiting, History, Compute, Private AI, Mission drafts, Profile, Settings and an internal automatic-contributing flow. `/` redirects to Insights; unknown paths show NotFound. `app/routes.ts:1`, `app/app-routes.tsx:40`. |
| First run | Six current cases: welcome, tools, uses, traces, privateAI, join. Private-AI-only and watch-only are explicit paths. `TCShellCore/FirstRun/FirstRunState.swift:9`, `:20`; `FirstRun/FirstRunNavigation.swift:133`. | Welcome, Connect, Consent, Scan, Watch, Done. `ViewModels/OnboardingViewModel.cs:727`. | Welcome, Connect, Consent, Scan, Watch, Done; Scan is conditionally skipped. `ui/onboarding.rs:49`, `:455`. | Twelve Flow 1 steps including roots, path, privacy, inference, two disclosures, grant and projects. Backtracking/branching has its own reducer. `features/onboarding/flow1.ts:73`, `:97`. |
| Settings presentation | A separate SwiftUI Window is actually registered. `TraceCommonsAppMain.swift:140` renders `SettingsWindowView`, whose body embeds `SettingsModal` at `:165`. `MainWindowNavigation.swift:56` retains an older modal description; executable scene registration resolves that discrepancy. | Settings is a pane selected by the rail, with its own WinUI implementation. `MainWindow.xaml.cs:1115`. | Settings is a stack page. `ui/mod.rs:357`, `:368`. | Settings is a React route, and opens the automatic contribution consent flow through route navigation. `app/app-routes.tsx:52`, `:125`. |
| Daemon startup/ownership | Off-main construction attempts local ownership; an already-running error attaches to the existing daemon. Pending start cancellation owns and disposes late results. `DaemonStartup.swift:27`, `:60`, `:91`. | `DaemonHost.StartAsync` constructs the FFI `TcDaemon` off the UI thread and subscribes after adoption. This method contains no attach fallback. `DaemonHost.cs:181`. | Local Insights startup defers contributor services; entering that surface lazily calls `start_or_ask`. When opened, `Backend::open` attaches to an existing daemon or hosts the loop with lock-race fallback; systemd ownership is supported. `main.rs:142`, `:148`; `backend.rs:25`, `:144`. | Runtime supports embedded and attached daemon connections; ensure-start may reconnect/attach. `src-tauri/src/runtime.rs:20`, `:91`; `src-tauri/src/state.rs:222`, `:251`. |
| Close versus quit | Closing a window is separate from application termination. The menu-bar scene and other windows remain independent; termination uses a shared confirmation and bounded compute shutdown response. `TraceCommonsAppMain.swift:84`, `:123`, `:140`, `:195`; `AppDelegate.swift:85`. | A window close hides to a reachable tray; without a tray it asks to quit. An actual Closed event disposes tray and daemon host. `MainWindow.xaml.cs:183`, `:1766`. | Window close opens a role-specific dialog. Attached mode can close the app while keeping the separate watcher, or explicitly call `shutdown` too; hosting mode has different copy. `ui/mod.rs:642`. | Native close is prevented and emits `quit-requested` to React. Confirmation calls the native quit command; actual application exit triggers runtime cleanup. `src-tauri/src/app.rs:405`, `:465`; `app/quit-confirmation.tsx:26`; `src-tauri/src/commands/platform.rs:181`. |
| Mission visibility | Home pages `shown(missions:)` redirect hidden mission pages to overview. `MonitorNavigation.swift:17`. | Mission drafts rail visibility is bound to the view model. `MainWindow.xaml:328`. | `missions_enabled` is read once at startup and fixed for the process; widgets, tray and accelerators use that setting. `ui/mod.rs:98`, `:119`, `:365`. | Mission drafts navigation and deep-link eligibility are filtered by the setting, and the route redirects while hidden. `app/routes.ts:77`, `:83`; `app/app-routes.tsx:66`. |
| Update integration | Update policy is chosen using Homebrew install detection and the configured feed. `UpdateController.swift:27`, `:38`. | Update application is an awaited platform operation which may terminate the process. `MainWindow.xaml.cs:1754`. | Flatpak portal performs managed updates; source builds have an Unmanaged state and make no install call. `update.rs:47`, `:116`, `:211`, `:328`. | Native commands report Homebrew-managed or installer-managed update guidance, both with `feed_configured: false`. `src-tauri/src/commands/platform.rs:105`. No parity with the Swift updater is inferred. |

Paths in the table are relative to their shell roots: macOS is
`macos/Sources/TraceCommonsApp/` except `TCShellCore/`, which is relative to
`macos/Sources/`; Windows is `windows/src/TraceCommons.App/`; GTK is
`crates/trace-commons-contributor-gtk/src/`; Tauri frontend is
`tauri-desktop/frontend/src/`, and its `src-tauri/` paths are relative to
`tauri-desktop/`. Every extracted entry also has an unambiguous full repository
path and source line in [coverage.json](coverage.json).

The common IPC dispatcher exposes the same method surface to the socket and
local callers (`crates/trace-commons-contributor/src/daemon/ipc.rs:94`, `:4005`,
`:7079`). That does not make all shell request/response lifetime behavior equal.
GTK uses a job thread and a separate event thread (`worker.rs:119`, `:144`);
Windows uses its own connection generation and UI-dispatched event processing
(`DaemonHost.cs:202`); Swift startup owns cancellation/adoption explicitly.
Per-screen request cancellation and stale-result guards must be checked in the
corresponding UI implementation before claiming cross-platform coverage.

The transport access-control boundary also differs. Unix IPC verifies the state
directory before binding (`crates/trace-commons-contributor/src/daemon/ipc.rs:9505`);
Windows creates named pipes with a restricted descriptor
(`crates/trace-commons-contributor/src/daemon/win_pipe.rs:166`). The Quint suite
does not model filesystem permissions, Windows ACL evaluation or real OS
credential stores. Source inventory and Rust compilation cannot verify those
runtime controls.

Manual local HTTP callbacks are separately indexed because they are not Axum
routes: native account sign-in, wallet provisioning, Cloud credential bounce and
deposit, provider chooser, wallet callback/result and wallet assets. Their
authentication/ceremony input differs from ordinary server API authority. The
legacy native account callback sends its browser response before validating the
path and does not check the HTTP method
(`crates/trace-commons-contributor/src/account_auth.rs:290`, `:349`); this is a
current-behavior observation, not an assertion that browser success proves
account authentication. IronWire's embedded listener starts from
`crates/trace-commons-contributor/src/daemon/private_inference.rs:1166`; its router
implementation is an external dependency and stays outside this repository's
source inventory.

The [platform_clients model](models/platform_clients.md) separately projects
Windows/GTK/Tauri startup ownership, first run, navigation, close/quit, selected
account-change and sign-in paths, subscriptions and update boundaries. It
preserves the current native Windows/GTK scope-confirmation behavior: the flow
advances after an IPC error response, because the source callbacks do not inspect
that result (`ViewModels/OnboardingViewModel.cs:536`;
`ui/onboarding.rs:882`). This observation does not include an exception that
prevents the callback from running. A failed native completion write can also
reach Done, as described in that model's source notes.

The native macOS six-step first-run model does not cover the other platform
flows merely because some step names coincide; their explicit mappings name
`platform_clients`. Screen/navigation rows remain gaps unless a reviewed row
names corresponding actions and fields. Full update/installer implementation,
per-widget loading/error states, tray accessibility, exact portal behavior,
every local sheet and native callback transport parsing require further
correspondence. Screen declaration candidates intentionally include components;
their count is not a count of separately reachable screens.

The [provider model](models/client_providers.md) uses two symbolic tool families
(Claude and Codex), independently of subscription/API-key/NEAR AI connection
kind. OpenCode exists in source but is outside that explicit bound, including
its subscription restriction and named-model requirements. Its absence is an
omission, not platform parity. The [consent model](models/client_consent.md)
separates saved grant identity, grant mutation replies and the read-only
`automatic_grant` getter/callback that updates displayed sharing choice.
