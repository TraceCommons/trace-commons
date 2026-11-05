import Darwin
import Dispatch

/// Turns SIGTERM (`kill <pid>`, `pkill`, a logout's last resort) into the
/// app's own quit path.
///
/// The daemon inside this process installs no signal handlers (#1949), so
/// without this SIGTERM takes its default action and the process dies on the
/// spot: no `applicationShouldTerminate`, so no compute stop, and no
/// `willTerminateNotification`, so no `tc_daemon_stop`.
///
/// The disposition is a caught no-op handler, not `SIG_IGN`. An ignored
/// signal stays ignored across `exec`, so every process this app starts
/// afterwards (a terminal session, which is stopped with SIGTERM, the compute
/// worker) would ignore SIGTERM too. A caught handler is reset to the default
/// in the child at `exec`. The dispatch source sees the signal either way.
@MainActor
final class TerminationSignal {
    private var source: DispatchSourceSignal?

    /// Installs once; later calls do nothing. `onSignal` runs on the main
    /// queue, once per SIGTERM delivered.
    func install(onSignal: @escaping @MainActor @Sendable () -> Void) {
        guard source == nil else { return }
        signal(SIGTERM) { _ in }
        let source = DispatchSource.makeSignalSource(signal: SIGTERM, queue: .main)
        source.setEventHandler {
            MainActor.assumeIsolated { onSignal() }
        }
        source.resume()
        self.source = source
    }
}
