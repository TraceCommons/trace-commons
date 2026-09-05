# Holonear macOS integration map

Companion to `2026-09-04-holonear-compute-contribution.md`. This records source
inspection of the macOS shell at `5cf13a27`; it does not declare a working pilot.

Preparation delivered: `TCBridge/TCCompute.swift` owns the new controller ABI
handle and serializes status, commands, and close. Five real-dylib Swift tests
cover unavailable enable without persisted consent, restore-paused/revoke,
idempotent close, concurrent command/close, and embedded-NUL directory rejection.
The app target builds with the bridge. No Compute destination, power adapter,
worker packaging, or running worker is included yet.

## Navigation and ownership

`macos/Sources/TraceCommonsApp/Views/MainWindowView.swift` currently renders its
sidebar only after the watcher has started, trace enrollment has succeeded, and
trace onboarding has completed. Adding a Compute row inside that sidebar alone
would require compute-only contributors to configure trace collection first.

Move destination selection above the watcher startup/onboarding switch. Keep the
existing switch as the trace destination's content, including its single stable
`OnboardingCoordinatorView` identity. Compute must remain reachable when trace
roots are undeclared, enrollment is absent, or the watcher refuses to start.
Existing Waiting, History, and Settings behavior should remain covered by the
existing onboarding tests. Add a navigation regression test for each of those
three compute-only entry conditions before exposing the destination.

Create `ComputeModel.swift` as an app-owned observable object, independent of
`AppModel`'s daemon client. Resolve its directory with the same
`DaemonHost.resolveConfigDirectory()` used by `AppModel.start()`. Do not require
`TCDaemon`, `refreshAll()`, session discovery, trace enrollment, or trace consent
to open the compute controller. Keep the model alive above window/view lifetime
in `TraceCommonsAppMain.swift`; closing the window must not free its controller.

`Views/ComputeView.swift` should consume Rust-supplied copy and snapshots. It must
not infer pool admission from a live process, invent resource measurements, or
optimistically change state after a command. A missing capability leaves start
unavailable. First-run controls belong in this destination, not trace onboarding.

## Bridge

`Sources/TCBridge/TCCompute.swift` should exclusively own the opaque compute
handle, free every C string, serialize calls against close, and expose fixed
refusal labels. The caller runs blocking FFI work on a dedicated queue rather
than the main thread. Decodable snapshots and commands carry no raw pointers.
Tests in `Tests/TCBridgeTests` must exercise the real dylib with a fresh temporary
directory: no trace enrollment, disabled defaults, unavailable worker refusal,
pause persistence, repeated close, and command/close concurrency.

Shared title, introduction, resource labels, action labels, status, and refusal
copy belong in contributor Rust and its C ABI. Swift should not turn labels into
sentences. Unknown or incomplete snapshots disable commands.

## Lifecycle and power

`AppDelegate.applicationShouldTerminate` currently confirms and returns
`.terminateNow`. `Launcher.launch()` registers `willTerminateNotification` to run
`AppModel.shutdown()` synchronously. Neither can await a worker drain.

Before enabling a real worker, change explicit Quit to `.terminateLater` after
confirmation. Route a single idempotent termination request to the app-owned
compute controller on its queue, await a bounded stop result, then call
`reply(toApplicationShouldTerminate:)`. Repeated Cmd-Q must not launch duplicate
drains. Keep the existing trace shutdown at final termination. Controller close
must not be mistaken for confirmed coordinator drain, and a timeout must retain
its distinct outcome. Test cancellation of Quit, repeated Quit, slow drain, and
forced termination through an injectable lifecycle adapter.

No existing AC/battery, thermal, memory-pressure, or sleep hooks were found in the
app. Add an app-owned native event adapter, with an initial power/thermal sample
before Resume, plus updates through IOKit power notifications,
`ProcessInfo.thermalStateDidChangeNotification`, a memory-pressure dispatch source,
and `NSWorkspace` sleep/wake notifications. Feed typed observations to the Rust
policy/controller; do not duplicate policy in views. Unknown power state must not
authorize an AC-only start. Resume remains manual after an automatic pause.

Sleep notification delivery does not guarantee enough time to drain. Record an
interruption when acknowledgement is absent and revalidate telemetry after wake.
Do not install wake locks. Tests can validate event mapping and policy inputs;
real-device sleep, battery, and pressure evidence remains a pilot acceptance gate.

## Packaging and release gates

`macos/scripts/make-app-bundle.sh` builds universal arm64/x86_64 app and FFI
binaries, rewrites their install names, embeds Sparkle, and signs development
bundles. It embeds no Holonear worker. An Apple Silicon-only worker must not be
treated as proof the existing universal app supports compute on Intel.

Reserve `Contents/Helpers/holonear` for the pinned worker and a reviewed manifest
under `Contents/Resources` for contract version, supported architectures, assets,
and expected worker identity. The exact launch path must come from the installed
bundle, never PATH, the working directory, or an environment override. Presence
of a file alone is not an eligibility or signature check.

`macos/scripts/make-release-dmg.sh` signs Sparkle's nested code, then all dylibs
under Frameworks, then the app. It currently knows nothing about worker identity.
Do not append a blanket worker re-signing step: confirm which signing identity,
entitlements, executable measurement, and bundled MLX assets the pool accepts.
Audit the existing Frameworks dylib signing loop if worker assets are added there.
The final worker bytes and assets must be checked after signing and after DMG
installation; verification of a pre-packaging executable is insufficient.

Before wiring build inputs, settle the pinned artifact, checksum/signature source,
contract capabilities, attestation acceptance, and MLX runtime dependencies with
the worker implementation. Required installed-app checks: an absolute verified
worker path; arm64 eligibility; no external library paths; capability handshake;
isolated child environment/state/cache; missing/tampered/incompatible worker
refusal; actual test-pool admission/work/drain. No developer signing credentials,
production pool, or release publishing changes are part of this preparation.
