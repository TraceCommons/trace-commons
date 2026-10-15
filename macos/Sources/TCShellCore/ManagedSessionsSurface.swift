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
    public let model: String?
    public let collection: String?
    /// The daemon can bring this session's window to the front (#1407).
    /// False from an older daemon.
    public let canFocus: Bool
    /// The daemon can ask this session's launcher to stop it. False from an
    /// older daemon and for a launch from the command line.
    public let canStop: Bool
    /// The ended session can be launched again as it was.
    public let canStartAgain: Bool
    /// When the session started; nil when absent or unreadable.
    public let startedAt: Date?
    public var holdsAccount: Bool { state != "exited" && state != "failed" }
    enum CodingKeys: String, CodingKey {
        case id, tool, connection, cwd, state, purpose, model, collection
        case accountID = "account_id", accountLabel = "account_label", projectLabel = "project_label", exitCode = "exit_code"
        case canFocus = "can_focus", canStop = "can_stop", canStartAgain = "can_start_again", startedAt = "started_at"
    }

    /// Decodes its own timestamp, as `PastSession` does: chrono writes
    /// nanoseconds, which `ISO8601DateFormatter` refuses.
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        tool = try c.decode(String.self, forKey: .tool)
        connection = try c.decode(String.self, forKey: .connection)
        accountID = try c.decode(String.self, forKey: .accountID)
        accountLabel = try c.decode(String.self, forKey: .accountLabel)
        projectLabel = try c.decode(String.self, forKey: .projectLabel)
        cwd = try c.decode(String.self, forKey: .cwd)
        state = try c.decode(String.self, forKey: .state)
        purpose = try c.decode(String.self, forKey: .purpose)
        exitCode = try c.decodeIfPresent(Int.self, forKey: .exitCode)
        model = try c.decodeIfPresent(String.self, forKey: .model)
        collection = try c.decodeIfPresent(String.self, forKey: .collection)
        canFocus = try c.decodeIfPresent(Bool.self, forKey: .canFocus) ?? false
        canStop = try c.decodeIfPresent(Bool.self, forKey: .canStop) ?? false
        canStartAgain = try c.decodeIfPresent(Bool.self, forKey: .canStartAgain) ?? false
        startedAt = try c.decodeIfPresent(String.self, forKey: .startedAt).flatMap(SourceCandidate.Wire.parseTimestamp)
    }
}

/// What a session row offers (#1407), in the order drawn.
public enum ManagedSessionAction: Equatable, Sendable {
    /// Bring the session's window to the front.
    case show
    /// Stop the session, after a confirmation.
    case stop
    /// Launch the ended session again, same tool, account and folder.
    case startAgain
    /// Take the ended row off the list.
    case dismiss
}

public struct ManagedToolAvailability: Decodable, Equatable, Identifiable, Sendable {
    public let tool: String
    public let installed: Bool
    public let installURL: String
    public var id: String { tool }
    public var url: URL? {
        guard let url = URL(string: installURL), url.scheme == "https" else { return nil }
        return url
    }
    enum CodingKeys: String, CodingKey { case tool, installed; case installURL = "install_url" }
}

public struct ManagedSnapshot: Decodable, Equatable, Sendable {
    public struct Capabilities: Decodable, Equatable, Sendable {
        public let managedLaunch: Bool
        public let terminalLaunch: Bool
        public let terminalDestination: String?
        /// Whether the daemon holds a NEAR AI sign-in. A bool, never the
        /// credential; absent from an older daemon, which reads as no.
        public let nearAiSignin: Bool?
        enum CodingKeys: String, CodingKey {
            case managedLaunch = "managed_launch", terminalLaunch = "terminal_launch", terminalDestination = "terminal_destination"
            case nearAiSignin = "near_ai_signin"
        }
    }
    public let tools: [ManagedToolAvailability]?
    public let copy: [String: String]?
    public let revision: UInt64
    public let accounts: [ManagedAccount]
    /// One entry per tool, derived from the NEAR AI sign-in and never
    /// saved. Empty, or absent from an older daemon, without one.
    public let signInAccounts: [ManagedAccount]?
    public let defaults: [ManagedSelection]
    public let generations: [String: UInt64]
    public let sessions: [ManagedSession]
    public let capabilities: Capabilities
    enum CodingKeys: String, CodingKey {
        case tools, copy, revision, accounts, defaults, generations, sessions, capabilities
        case signInAccounts = "sign_in_accounts"
    }
}

/// Copy keys for the managed-session values the daemon sends, so no shell
/// draws a raw wire value or a daemon error code. Every word is in the
/// shared table (`managed/copy.rs`); this only picks the key.
public enum ManagedSurface {
    public static func visibleHarnesses(_ rows: [HarnessRow], tools: [ManagedToolAvailability]?) -> [HarnessRow] {
        rows.filter { row in
            // Saved routing and previous activity remain manageable after uninstall.
            if row.connected || row.lastCallAt != nil { return true }
            return tools?.first(where: { $0.tool == row.id })?.installed ?? row.installed
        }
    }

    /// The managed tools, in the order the Add and Launch sheets offer them.
    /// OpenCode is one since #1417.
    public static let supportedTools = ["claude", "codex", "opencode"]
    public static func setupTools(_ tools: [ManagedToolAvailability]?) -> [String] {
        guard let tools else { return supportedTools }
        return supportedTools.filter { tool in tools.first(where: { $0.tool == tool })?.installed == true }
    }

    /// The lifecycle values the daemon sends today.
    public static let sessionStates: Set<String> = ["starting", "running", "exited", "failed", "unknown"]
    /// The sign-in values the daemon sends today.
    public static let authStates: Set<String> = ["sign_in_required", "checking", "ready", "unavailable"]

    /// A lifecycle a later daemon grows reads as unknown, never as its raw value.
    public static func stateKey(_ state: String) -> String {
        "state_" + (sessionStates.contains(state) ? state : "unknown")
    }

    /// Nil for a sign-in value this shell does not know: no label rather than the raw value.
    public static func authKey(_ authState: String) -> String? {
        authStates.contains(authState) ? "auth_" + authState : nil
    }

    /// The tools a NEAR AI sign-in session can start, in the order shown.
    public static let signInTools = ["claude", "codex"]

    /// The Start button's key for a tool, or nil for a tool this shell
    /// does not know.
    public static func startKey(_ tool: String) -> String? {
        signInTools.contains(tool) ? "start_" + tool : nil
    }

    /// The Add sheet's sign-in types. Signed in, NEAR AI is not offered: the
    /// sign-in entries already run on it, with no key. Signed out, a pasted
    /// key stays the fallback.
    public static func connectionOptions(signedIn: Bool, tool: String = "claude") -> [String] {
        // OpenCode has no sign-in entry or subscription flow. Its NEAR AI
        // connection always uses a pasted key, even while signed in.
        if tool == "opencode" { return ["api_key", "near_ai"] }
        return signedIn ? ["subscription", "api_key"] : ["subscription", "api_key", "near_ai"]
    }

    /// The sign-in card's line with `{destination}` filled from the
    /// terminal the daemon reports, as the launch line does; `fallback`
    /// (the core's "your terminal") when it reports none.
    public static func signInScope(_ template: String, destination: String?, fallback: String) -> String {
        template.replacingOccurrences(of: "{destination}", with: destination ?? fallback)
    }

    /// `tool_unavailable` ("{tool} wasn't found on this computer.") with the
    /// tool's display name filled in.
    public static func toolUnavailable(_ tool: String, copy: (String) -> String) -> String {
        copy("tool_unavailable").replacingOccurrences(of: "{tool}", with: copy(toolKey(tool) ?? tool))
    }

    /// The tool's name key, or nil for a tool this shell does not know.
    public static func toolKey(_ tool: String) -> String? {
        ["claude", "codex", "opencode"].contains(tool) ? tool : nil
    }

    /// A failed action is said in the shared copy, never as the daemon's
    /// code: a launch whose outcome is unknown asks for a refresh, anything
    /// else is the general failure. A missing tool is said on its account
    /// row after the refresh that follows, where the tool can be named.
    public static func errorKey(code: String?, message: String?) -> String {
        let launchUnknown = ["launch-unknown", "managed-launch-unknown"]
        if launchUnknown.contains(code ?? "") || launchUnknown.contains(message ?? "") { return "launch_unknown" }
        // No NEAR AI sign-in: the line that asks for one.
        let signInNeeded = "managed-near-ai-sign-in-required"
        if code == signInNeeded || message == signInNeeded { return "sign_in_needed" }
        return "action_failed"
    }
}

/// Session row actions (Ron, #1407, 2026-10-14). An action the daemon cannot
/// do is left out, never drawn disabled.
public extension ManagedSurface {
    /// The row's actions. A running row: Show and Stop where the daemon can
    /// do them. An ended row: Start again where it can be, with a terminal
    /// to start in, then Dismiss. Anything else: none.
    static func sessionActions(_ session: ManagedSession, terminalLaunch: Bool) -> [ManagedSessionAction] {
        if session.state == "running" { return menuActions(session) }
        guard !session.holdsAccount else { return [] }
        return (session.canStartAgain && terminalLaunch ? [.startAgain] : []) + [.dismiss]
    }

    /// The row menu's items: Show and Stop, for a running row only.
    static func menuActions(_ session: ManagedSession) -> [ManagedSessionAction] {
        guard session.state == "running" else { return [] }
        return (session.canFocus ? [.show] : []) + (session.canStop ? [.stop] : [])
    }

    /// Whether clicking the row brings its window to the front.
    static func clickShows(_ session: ManagedSession) -> Bool {
        session.state == "running" && session.canFocus
    }

    /// The stop confirmation's line with `{tool}` and `{folder}` filled.
    static func stopDescription(_ template: String, tool: String, folder: String) -> String {
        template.replacingOccurrences(of: "{tool}", with: tool).replacingOccurrences(of: "{folder}", with: folder)
    }

    /// The `managed_session_start_again` request: the row and a fresh
    /// request id, nothing the shell could change.
    static func startAgainParams(sessionID: String, requestID: String) -> [String: Any] {
        ["session_id": sessionID, "request_id": requestID]
    }

    /// The row's caption: its parts, then the start time in the app's
    /// relative format when the daemon sent one.
    static func caption(_ parts: [String], startedAt: Date?, relative: (Date) -> String = Format.when) -> String {
        (parts + [startedAt.map(relative)].compactMap { $0 }).filter { !$0.isEmpty }.joined(separator: " · ")
    }
}

public extension ManagedSnapshot {
    var setupTools: [String] { ManagedSurface.setupTools(tools) }
    var needsToolInstallation: Bool {
        guard let tools else { return false }
        return !tools.contains { ManagedSurface.supportedTools.contains($0.tool) && $0.installed }
    }
    func canLaunch(tool: String) -> Bool {
        guard ManagedSurface.supportedTools.contains(tool) else { return false }
        guard let tools else { return true }
        return tools.first { $0.tool == tool }?.installed == true
    }

    /// A running, starting or unknown session holds its account: no sign-in,
    /// key change or removal while it does.
    func isHeld(_ account: ManagedAccount) -> Bool {
        sessions.contains { $0.accountID == account.id && $0.holdsAccount }
    }

    func isDefault(_ account: ManagedAccount) -> Bool {
        defaults.contains { $0.accountID == account.id }
    }

    /// Whether the daemon holds a NEAR AI sign-in.
    var nearAiSignedIn: Bool { capabilities.nearAiSignin == true }

    /// The tool's NEAR AI sign-in entry, only while signed in.
    func signInEntry(_ tool: String) -> ManagedAccount? {
        guard nearAiSignedIn else { return nil }
        return signInAccounts?.first { $0.tool == tool && $0.connection == "near_ai_signin" }
    }

    /// The `managed_launch_prepare` request that starts `tool` on the NEAR
    /// AI sign-in in `cwd`; nil without a sign-in entry for it.
    func signInLaunchParams(tool: String, cwd: String, requestID: String) -> [String: Any]? {
        guard canLaunch(tool: tool), let entry = signInEntry(tool) else { return nil }
        return [
            "request_id": requestID, "purpose": "coding", "tool": entry.tool,
            "connection": entry.connection, "account_id": entry.id, "cwd": cwd,
            "expected_generation": generations[entry.tool] ?? 0, "save_default": false,
        ]
    }
}
