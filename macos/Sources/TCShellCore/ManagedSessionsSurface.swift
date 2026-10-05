import Foundation

public struct ManagedAccount: Decodable, Equatable, Identifiable, Sendable {
    public let id: String
    public let tool: String
    public let connection: String
    public let label: String
    public let authState: String
    enum CodingKeys: String, CodingKey { case id, tool, connection, label; case authState = "auth_state" }
}

public struct ManagedSelection: Decodable, Equatable, Sendable {
    public let tool: String
    public let accountID: String
    public let generation: UInt64
    enum CodingKeys: String, CodingKey { case tool, generation; case accountID = "account_id" }
}

public struct ManagedSession: Decodable, Equatable, Identifiable, Sendable {
    public let id: String
    public let tool: String
    public let connection: String
    public let accountID: String
    public let accountLabel: String
    public let projectLabel: String
    public let cwd: String
    public let state: String
    public let purpose: String
    public let exitCode: Int?
    public var holdsAccount: Bool { state != "exited" && state != "failed" }
    enum CodingKeys: String, CodingKey {
        case id, tool, connection, cwd, state, purpose
        case accountID = "account_id", accountLabel = "account_label", projectLabel = "project_label", exitCode = "exit_code"
    }
}

public struct ManagedSnapshot: Decodable, Equatable, Sendable {
    public struct Capabilities: Decodable, Equatable, Sendable {
        public let managedLaunch: Bool
        public let terminalLaunch: Bool
        public let terminalDestination: String?
        enum CodingKeys: String, CodingKey {
            case managedLaunch = "managed_launch", terminalLaunch = "terminal_launch", terminalDestination = "terminal_destination"
        }
    }
    public let copy: [String: String]?
    public let revision: UInt64
    public let accounts: [ManagedAccount]
    public let defaults: [ManagedSelection]
    public let generations: [String: UInt64]
    public let sessions: [ManagedSession]
    public let capabilities: Capabilities
}
