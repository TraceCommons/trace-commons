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

    /// K2 (#1173): whether the daemon reports a developer dry run, read
    /// from a `status` result. The daemon decides the mode (from
    /// `TC_DEV_DRY_RUN`, in a debug build of the Rust library only) and
    /// reports it as `dev_dry_run`; the app never parses the environment
    /// itself, so its notice always matches what the daemon is doing.
    /// Absent, or anything but `true`, is off.
    static func devDryRun(fromStatus data: Data) -> Bool {
        guard let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            return false
        }
        return object["dev_dry_run"] as? Bool == true
    }
    #endif
}
