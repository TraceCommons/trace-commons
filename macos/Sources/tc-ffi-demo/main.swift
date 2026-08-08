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

    daemon.stop()
    daemon.close()
    print("daemon stopped and handle freed")
} catch {
    print("FAILED: \(error)")
    exit(1)
}

try? FileManager.default.removeItem(atPath: configDir)
