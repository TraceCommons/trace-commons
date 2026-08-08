import Foundation

/// Where the in-process daemon keeps its state, and the refusal rules that
/// decide whether it may start at all.
///
/// The C ABI has no call to set `claude_root` / `codex_root` before
/// `tc_daemon_start` (a `tc_daemon_start_with_settings` export is landing on
/// the daemon branch; adopt it here when it does). Left unset, the daemon
/// watches the real `~/.claude` and `~/.codex`. This shell will not let that
/// happen by accident: if `daemon-settings.json` does not already declare
/// both roots, startup is REFUSED with a sentence, rather than silently
/// watching a developer's actual work.
///
/// That is the same fail-closed posture the rest of the product takes, and
/// it is why "the first thing the app ever does is nothing" survives the
/// missing onboarding flow.
enum DaemonHost {
    struct Resolution {
        let path: String
    }

    enum Refusal: Error, CustomStringConvertible {
        case noDirectory
        case pathTooLong(Int)
        case notADirectory
        case settingsMissing
        case rootsNotDeclared

        var description: String {
            switch self {
            case .noDirectory:
                return """
                No state directory is set. Set TRACE_COMMONS_CONTRIBUTOR_DIR to a \
                directory holding a daemon-settings.json that says which session \
                folders to watch. Nothing is being watched.
                """
            case .pathTooLong(let bytes):
                return """
                The state directory path is too long (\(bytes) bytes). The control \
                socket lives inside it and cannot exceed the system limit. Nothing \
                is being watched.
                """
            case .notADirectory:
                return "The state directory does not exist. Nothing is being watched."
            case .settingsMissing:
                return """
                The state directory has no daemon-settings.json, so there is nothing \
                telling this app which session folders to watch. It will not guess. \
                Nothing is being watched.
                """
            case .rootsNotDeclared:
                return """
                daemon-settings.json does not say which session folders to watch. \
                This app will not fall back to watching everything on this machine. \
                Nothing is being watched.
                """
            }
        }
    }

    /// The daemon's socket is `<config_dir>/daemon.sock` and the daemon
    /// refuses a socket path over 104 bytes, so this budget is checked here
    /// rather than surfacing as an opaque start failure.
    private static let maxSocketPathBytes = 104
    private static let socketFileName = "/daemon.sock"

    static func resolveConfigDirectory() throws -> Resolution {
        guard let dir = ProcessInfo.processInfo.environment["TRACE_COMMONS_CONTRIBUTOR_DIR"],
              !dir.isEmpty
        else { throw Refusal.noDirectory }

        let socketBytes = (dir + socketFileName).utf8.count
        guard socketBytes <= maxSocketPathBytes else { throw Refusal.pathTooLong(socketBytes) }

        var isDirectory: ObjCBool = false
        guard FileManager.default.fileExists(atPath: dir, isDirectory: &isDirectory),
              isDirectory.boolValue
        else { throw Refusal.notADirectory }

        let settingsPath = dir + "/daemon-settings.json"
        guard let data = FileManager.default.contents(atPath: settingsPath) else {
            throw Refusal.settingsMissing
        }
        guard let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw Refusal.settingsMissing
        }
        let claudeDeclared = object["claude_root"] is String
        let codexDeclared = object["codex_root"] is String
        guard claudeDeclared || codexDeclared else { throw Refusal.rootsNotDeclared }

        return Resolution(path: dir)
    }
}
