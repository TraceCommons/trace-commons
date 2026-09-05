import Foundation

/// Where the contributor's state directory is, and the only reasons this
/// shell will refuse to name one.
///
/// Carved out of the app target for the same reason `TCUpdates` was: so it
/// can be tested without a framework, a bundle, or a running app. The defect
/// that prompted it is exactly the kind one unit test catches -- the shipped
/// app read `TRACE_COMMONS_CONTRIBUTOR_DIR` and nothing else, so a Finder
/// launch, which carries no shell environment, always refused. Every launch
/// that ever worked was a shell launch, which inherits the variable and hides
/// it.
///
/// The precedence here is not new: it is the one
/// `crates/trace-commons-contributor/src/config.rs` and
/// `windows/src/TraceCommons.App/DaemonHost.cs` already implement. Swift was
/// the odd one out.
///
/// What this deliberately does NOT decide is whether the session roots have
/// been declared. That rule lives once, in Rust
/// (`daemon::settings::roots_declared`), and is enforced at the C ABI's start
/// functions. A fourth transcription of it here is what this slice removed.
public enum StateDirectory {
    public struct Resolution: Equatable {
        public let path: String

        public init(path: String) {
            self.path = path
        }
    }

    public enum Refusal: Error, CustomStringConvertible {
        case pathTooLong(Int)
        case notADirectory

        public var description: String {
            switch self {
            case .pathTooLong(let bytes):
                return """
                    The state directory path is too long (\(bytes) bytes). The control \
                    socket lives inside it and cannot exceed the system limit. Nothing \
                    is being watched.
                    """
            case .notADirectory:
                return """
                    The state directory path is a file, not a folder. Nothing is being \
                    watched.
                    """
            }
        }
    }

    /// What the filesystem says about a path. Injected so the precedence
    /// rules can be tested without touching a real disk.
    public struct Probe: Sendable {
        public enum Verdict {
            case absent
            case directory
            case file
        }

        let verdict: @Sendable (String) -> Verdict

        public init(_ verdict: @escaping @Sendable (String) -> Verdict) {
            self.verdict = verdict
        }

        public static let filesystem = Probe { path in
            var isDirectory: ObjCBool = false
            guard FileManager.default.fileExists(atPath: path, isDirectory: &isDirectory) else {
                return .absent
            }
            return isDirectory.boolValue ? .directory : .file
        }
    }

    /// The daemon's socket is `<dir>/daemon.sock`, and the daemon refuses a
    /// socket path over 104 bytes. Checked here so an over-long path is a
    /// sentence rather than an opaque start failure.
    static let maxSocketPathBytes = 104
    static let socketFileName = "/daemon.sock"

    public static func resolve(
        explicit: String? = nil,
        environment: [String: String] = ProcessInfo.processInfo.environment,
        homeDirectory: String = NSHomeDirectory(),
        probe: Probe = .filesystem
    ) throws -> Resolution {
        let dir = chooseDirectory(
            explicit: explicit, environment: environment, homeDirectory: homeDirectory)

        let socketBytes = (dir + socketFileName).utf8.count
        guard socketBytes <= maxSocketPathBytes else {
            throw Refusal.pathTooLong(socketBytes)
        }

        // An absent directory is not a refusal: `ConfigStore::open` creates
        // it, 0700 on unix. Refusing here would put the fresh-install case
        // back into the dead end this slice exists to remove. A path that
        // exists and is not a directory is a different matter -- nothing
        // downstream can recover from that, and it is never what anyone
        // meant.
        if case .file = probe.verdict(dir) {
            throw Refusal.notADirectory
        }

        return Resolution(path: dir)
    }

    /// Compute uses no daemon socket, so a long directory must not inherit
    /// the watcher's Unix socket-path refusal. Folder validity still applies.
    public static func resolveCompute(
        explicit: String? = nil,
        environment: [String: String] = ProcessInfo.processInfo.environment,
        homeDirectory: String = NSHomeDirectory(),
        probe: Probe = .filesystem
    ) throws -> Resolution {
        let dir = chooseDirectory(explicit: explicit, environment: environment, homeDirectory: homeDirectory)
        if case .file = probe.verdict(dir) { throw Refusal.notADirectory }
        return Resolution(path: dir)
    }

    private static func chooseDirectory(
        explicit: String?,
        environment: [String: String],
        homeDirectory: String
    ) -> String {
        if let explicit, !explicit.isEmpty {
            return explicit
        }
        if let fromEnvironment = environment["TRACE_COMMONS_CONTRIBUTOR_DIR"],
            !fromEnvironment.isEmpty
        {
            return fromEnvironment
        }
        // The Homebrew cask's `zap` stanza already treats this as the state
        // directory, and it is where a contributor's identity already lives.
        return homeDirectory + "/Library/Application Support/trace-commons"
    }
}
