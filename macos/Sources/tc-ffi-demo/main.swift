import Foundation
import TCBridge

// Milestone demo: prove trace-commons-contributor-ffi's C ABI is callable
// end to end from Swift. Starts the daemon against a throwaway temp
// directory, calls "hello" and "status", frees every owned string, and
// tears the daemon down cleanly.

// MARK: - Pick a SHORT temp directory.
//
// The daemon's unix socket lives at "<config_dir>/daemon.sock" and refuses
// to start once that path exceeds 104 bytes (MAX_SOCKET_PATH_BYTES in
// daemon/ipc.rs). NSTemporaryDirectory() on macOS resolves to a long
// per-user path under /var/folders/... that blows the limit once
// "/daemon.sock" is appended, so this demo asks mktemp for a short name
// directly under /tmp instead.
func makeShortTempDir() -> String {
    let template = "/tmp/tccfg-XXXXXX"
    var buf = Array(template.utf8CString)
    let result = buf.withUnsafeMutableBufferPointer { ptr -> String? in
        guard let base = ptr.baseAddress, mkdtemp(base) != nil else { return nil }
        return String(cString: base)
    }
    guard let dir = result else {
        fatalError("mkdtemp failed: \(String(cString: strerror(errno)))")
    }
    return dir
}

// MARK: - Pre-seed daemon-settings.json.
//
// The C ABI has no call to set claude_root/codex_root before
// tc_daemon_start -- tc_call's "set_settings" (available only after start)
// covers quiescence_secs/digest_interval_secs/local_notifications only, not
// the session roots. Left at their default of None, the daemon watches the
// developer's REAL ~/.claude and ~/.codex trees, which this demo must never
// do. So this writes daemon-settings.json directly, in the same shape and
// location `trace_commons_contributor::daemon::settings::DaemonSettings`
// would (crates/trace-commons-contributor/src/config.rs:
// ConfigStore::daemon_path joins DAEMON_SETTINGS_FILE = "daemon-settings.json"
// onto config_dir), pointing both roots at empty temp subdirectories before
// the daemon ever starts.
func seedSettings(configDir: String) throws {
    let claudeRoot = configDir + "/claude-root"
    let codexRoot = configDir + "/codex-root"
    try FileManager.default.createDirectory(atPath: claudeRoot, withIntermediateDirectories: true)
    try FileManager.default.createDirectory(atPath: codexRoot, withIntermediateDirectories: true)

    // Mirrors DaemonSettings::default() (daemon/settings.rs) with
    // claude_root/codex_root overridden, serialized the way serde_json
    // would (field names only matter -- #[serde(default)] covers the rest
    // if this demo's field list ever drifts from the Rust struct).
    let settings: [String: Any] = [
        "schema_version": "trace_commons.daemon_settings.v1",
        "poll_interval_secs": 60,
        "quiescence_secs": 1800,
        "digest_interval_secs": 14400,
        "queue_ttl_days": 14,
        "growth_factor": 2.0,
        "growth_min_new_bytes": 65536,
        "max_reuploads": 3,
        "max_uploads_per_day": 50,
        "max_bytes_per_day": 209_715_200,
        "max_queue_entries": 500,
        "history_poll_secs": 1800,
        "canary_interval_secs": 3600,
        "local_notifications": false,
        "near_ai": NSNull(),
        "claude_root": claudeRoot,
        "codex_root": codexRoot,
    ]
    let data = try JSONSerialization.data(withJSONObject: settings, options: [.prettyPrinted])
    let path = configDir + "/daemon-settings.json"
    try data.write(to: URL(fileURLWithPath: path))
    try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: path)
}

// MARK: - Teardown stress (TC_DEMO_TEARDOWN_STRESS=1).
//
// The regression harness for the use-after-free this bridge used to have:
// the app fires every user action on a detached task that calls through the
// raw tc_handle*, and quitting used to free that handle without waiting for
// them. Here, worker threads hammer tc_call and a subscription is live while
// teardown runs, so `shutdown` is exercised with calls genuinely mid-flight.
//
// A clean run prints `.freed` and exits 0. A regression shows up as a crash
// (SIGSEGV / malloc double-free), not as a failed assertion, which is why
// this is a stress loop rather than a unit test.
final class Counter {
    private let lock = NSLock()
    private var refused = 0
    private var served = 0

    func record(refused isRefused: Bool) {
        lock.lock()
        if isRefused { refused += 1 } else { served += 1 }
        lock.unlock()
    }

    var snapshot: (served: Int, refused: Int) {
        lock.lock()
        defer { lock.unlock() }
        return (served, refused)
    }
}

func stressTeardown(_ daemon: TCDaemon) {
    let counter = Counter()
    let deadline = Date().addingTimeInterval(3.0)
    let subscription = daemon.subscribe { _ in }
    print("stress: subscription \(subscription == nil ? "refused" : "registered")")

    for _ in 0..<8 {
        let thread = Thread {
            while Date() < deadline {
                let response = daemon.call("status")
                counter.record(refused: response.contains("handle-freed"))
            }
        }
        thread.start()
    }

    // Long enough that several threads are inside tc_call right now.
    Thread.sleep(forTimeInterval: 0.5)
    // Evidence that teardown really does begin with calls mid-flight: a
    // nonzero count here is the exact condition the old code freed under.
    print("stress: calls inside the ABI at teardown: \(daemon.inFlightCalls)")
    // TC_DEMO_TEARDOWN_DRAIN_TIMEOUT=0 exercises the other branch: teardown
    // that cannot prove the handle is idle must LEAK it, not free it.
    let drainTimeout = ProcessInfo.processInfo
        .environment["TC_DEMO_TEARDOWN_DRAIN_TIMEOUT"]
        .flatMap(Double.init) ?? 3.0
    let outcome = daemon.shutdown(unsubscribing: subscription, drainTimeout: drainTimeout)
    let counts = counter.snapshot
    print("stress: shutdown -> \(outcome) (served=\(counts.served) refused=\(counts.refused))")

    // Let the workers run past the deadline so their post-teardown calls are
    // observed too, then confirm they were all refused rather than served.
    Thread.sleep(forTimeInterval: 3.0)
    let after = counter.snapshot
    print("stress: after workers finished (served=\(after.served) refused=\(after.refused))")
    if outcome != .freed {
        print("stress: handle was LEAKED, not freed -- safe, but investigate")
    }
}

let configDir = makeShortTempDir()
print("config dir: \(configDir)")

do {
    try seedSettings(configDir: configDir)
    try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: configDir)

    let daemon = try TCDaemon(configDir: configDir)
    print("daemon started")

    let helloResponse = daemon.call("hello")
    print("hello -> \(helloResponse)")

    let statusResponse = daemon.call("status")
    print("status -> \(statusResponse)")

    if ProcessInfo.processInfo.environment["TC_DEMO_TEARDOWN_STRESS"] == "1" {
        stressTeardown(daemon)
    } else {
        daemon.stop()
        print("teardown -> \(daemon.close())")
    }
} catch {
    print("FAILED: \(error)")
    exit(1)
}

try? FileManager.default.removeItem(atPath: configDir)
