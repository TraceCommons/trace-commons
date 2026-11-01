import Foundation

/// `economizer_status`, `economizer_set_enabled` and `economizer_restore`:
/// what the economizer (`near-ai economizer on`) learned in each folder a
/// managed session ran in. Decode only: every figure is the daemon's.
extension DaemonData {
    public struct EconomizerStatus: Codable, Equatable, Sendable {
        public let enabled: Bool
        /// Most sessions first; a folder with nothing learned is absent.
        public let folders: [EconomizerFolder]
        /// Folders left out past the daemon's limit.
        public let foldersTruncated: UInt32

        public init(enabled: Bool, folders: [EconomizerFolder] = [], foldersTruncated: UInt32 = 0) {
            self.enabled = enabled; self.folders = folders; self.foldersTruncated = foldersTruncated
        }

        public enum CodingKeys: String, CodingKey {
            case enabled, folders
            case foldersTruncated = "folders_truncated"
        }
    }

    /// One tool's sessions in one folder.
    public struct EconomizerFolder: Codable, Equatable, Sendable, Identifiable {
        /// The folder's last path component, or its last few when two
        /// folders for one tool share a name (home is `~`); never the path.
        public let folder: String
        /// `claude`, `codex` or `opencode`.
        public let harness: String
        /// The daemon's reference for `economizer_restore`.
        public let key: String
        public let sessions: UInt32
        /// Built-in tools left out of the next sessions here.
        public let leftOut: [String]
        /// Tools given back here; never left out again.
        public let givenBack: [String]
        /// Commands told to the agent at a session's start.
        public let told: [EconomizerCommand]
        /// Commands still being confirmed.
        public let forming: UInt32
        public var id: String { key }

        public init(folder: String, harness: String, key: String, sessions: UInt32, leftOut: [String] = [],
                    givenBack: [String] = [], told: [EconomizerCommand] = [], forming: UInt32 = 0) {
            self.folder = folder; self.harness = harness; self.key = key; self.sessions = sessions
            self.leftOut = leftOut; self.givenBack = givenBack; self.told = told; self.forming = forming
        }

        public enum CodingKeys: String, CodingKey {
            case folder, harness, key, sessions, told, forming
            case leftOut = "left_out"
            case givenBack = "given_back"
        }
    }

    /// Run `program` as `prefix program` in this folder.
    public struct EconomizerCommand: Codable, Equatable, Sendable {
        public let program: String
        public let prefix: String

        public init(program: String, prefix: String) {
            self.program = program; self.prefix = prefix
        }
    }

    /// `economizer_set_enabled`'s reply: the switch as it now stands.
    public struct EconomizerSwitch: Codable, Equatable, Sendable {
        public let enabled: Bool

        public init(enabled: Bool) { self.enabled = enabled }
    }

    /// `economizer_restore`'s reply: the tools given back.
    public struct EconomizerRestored: Codable, Equatable, Sendable {
        public let restored: [String]

        public init(restored: [String]) { self.restored = restored }
    }
}
