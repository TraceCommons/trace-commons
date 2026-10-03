import Foundation
import TCBridge
import TCShellCore

/// The one place a screen's `DaemonDataClient` is chosen (C1 of #1173).
///
/// Screens take an `any DaemonDataClient` and never construct one. Moving
/// a screen from sample data to the daemon is changing the call site from
/// `DaemonDataWiring.sample(.normalDay)` to `DaemonDataWiring.live(daemon)`;
/// no screen code changes.
enum DaemonDataWiring {
    /// The real client, over the same `tc_call` path `DaemonClient` uses
    /// (`preview_unsure_spans` included). Returned as the concrete type
    /// because its owner (`AppModel`) also feeds it events and ends them at
    /// teardown; screens still receive `any DaemonDataClient`.
    static func live(_ daemon: TCDaemon) -> LiveDaemonClient {
        LiveDaemonClient(transport: daemon)
    }

    #if DEBUG
    /// Sample data for previews and development. Debug builds only.
    static func sample(_ set: SampleDaemonClient.SampleSet) -> any DaemonDataClient {
        SampleDaemonClient(set)
    }

    /// K2 (#1173): the developer-only dry-run switch, for running `live`
    /// against your own real Claude Code and Codex sessions with no risk of
    /// sending anything.
    ///
    /// Read once, here, from the process environment -- `TC_DEV_DRY_RUN=1`,
    /// set before launch (an Xcode scheme's environment variables, or
    /// `TC_DEV_DRY_RUN=1 .build/debug/TraceCommonsApp`). This property does
    /// not change what `live` does or what the daemon watches: a developer
    /// still runs the ordinary first-run flow and points it at their own
    /// real session folders, exactly as any contributor does. The entire
    /// guarantee -- that approving, arming, granting automatic upload,
    /// enrolling, or any other send is refused, and that IronWire is never
    /// hosted -- is enforced by the daemon itself, in the Rust contributor
    /// library, from the same environment variable
    /// (`daemon::dev_dry_run_enabled` in `trace-commons-contributor`), read
    /// once at daemon startup and never reachable over IPC afterward. See
    /// `macos/SAMPLE_DATA.md`.
    ///
    /// This property exists so a Debug build can tell the developer which
    /// mode they are in (`AppModel` logs a console notice when it is true)
    /// and so a Release build can be proven never to offer it at all --
    /// see `DevDryRunReleaseTests` in `TraceCommonsAppTests`, which builds
    /// this package `-c release` and greps the product for the environment
    /// variable's name.
    static var devDryRunActive: Bool {
        ProcessInfo.processInfo.environment["TC_DEV_DRY_RUN"] == "1"
    }
    #endif
}
