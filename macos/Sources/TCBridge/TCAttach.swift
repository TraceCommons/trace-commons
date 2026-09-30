import CTraceCommons
import Foundation

/// The words for a watcher another process is already running. Nothing here
/// authors contributor-facing wording: every sentence comes from
/// `attach_copy` in the Rust contributor crate, so this shell, the GTK one
/// and the Windows one say the same thing and a rewording reaches all three.
public enum TCAttach {
    /// Every fixed word on this surface, as the ABI returns it.
    public struct Copy: Decodable, Equatable, Sendable {
        public let attachedTitle: String
        public let attachedDetail: String
        public let alreadyRunning: String
        public let noDaemonListening: String
        public let stateDirectoryNotWritable: String
        public let settingsUnreadable: String
        public let ipcBindFailed: String

        enum CodingKeys: String, CodingKey {
            case attachedTitle = "attached_title"
            case attachedDetail = "attached_detail"
            case alreadyRunning = "already_running"
            case noDaemonListening = "no_daemon_listening"
            case stateDirectoryNotWritable = "state_directory_not_writable"
            case settingsUnreadable = "settings_unreadable"
            case ipcBindFailed = "ipc_bind_failed"
        }
    }

    public static func copyJSON() -> String? {
        guard let raw = tc_attach_copy() else { return nil }
        defer { tc_string_free(raw) }
        return String(cString: raw)
    }

    /// The decoded copy, or nil if the ABI could not answer.
    ///
    /// Deliberately not cached behind a `static let`: a decode that failed
    /// once at launch would be a surface permanently wordless, and this is
    /// cheap enough to ask for again.
    public static func copy() -> Copy? {
        guard let json = copyJSON(), let data = json.data(using: .utf8) else { return nil }
        return try? JSONDecoder().decode(Copy.self, from: data)
    }

    /// The sentence for a fixed start-failure label, or nil for a label this
    /// surface does not own.
    ///
    /// nil rather than a fallback: `trace_commons.h` says to treat an
    /// unrecognized label as `daemon-start-failed` rather than assuming it is
    /// safe to display, and inventing a sentence here would be doing exactly
    /// that.
    public static func line(forLabel label: String) -> String? {
        guard let copy = copy() else { return nil }
        switch label {
        case "already-running": return copy.alreadyRunning
        case "no-daemon-listening": return copy.noDaemonListening
        case "state-directory-not-writable": return copy.stateDirectoryNotWritable
        case "settings-unreadable": return copy.settingsUnreadable
        case "ipc-bind-failed": return copy.ipcBindFailed
        default: return nil
        }
    }
}
