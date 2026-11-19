import Foundation

/// Claude Desktop's saved Local environment is independent of managed CLI
/// sessions. A configured route is only observed after the daemon sees a
/// successful request carrying that route's credential.
public struct ClaudeDesktopStatus: Decodable, Equatable, Sendable {
    public enum State: String, Sendable {
        case disconnected, ready, observed, unavailable
        case signInRequired = "sign_in_required"
        case disabled
    }

    public let state: State
    public let model: String?
    public let backgroundModel: String?
    public let observed: Bool
    public let copy: [String: String]
    public let environment: String?
    /// Keep a known recoverable refusal distinct from an unfamiliar state
    /// even though both render the shared Unavailable status.
    private let recognizedState: Bool

    private enum CodingKeys: String, CodingKey {
        case state, model, observed, copy, environment
        case backgroundModel = "background_model"
    }

    public static let copyKeys = [
        "title", "setup", "save", "copy", "disconnect", "main_model", "background_model",
        "status_disconnected", "status_ready", "status_observed", "status_unavailable",
        "status_sign_in_required", "status_disabled", "instructions", "error",
    ]
    /// The Private AI tab's setup notice (`ClaudeDesktopPrompt`). Optional:
    /// a daemon without them draws no notice, and the row still works.
    public static let promptCopyKeys = ["prompt_title", "prompt_line", "prompt_dismiss"]

    public init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        let reported = try values.decode(String.self, forKey: .state)
        let successfulCall = try values.decode(Bool.self, forKey: .observed)
        let known = State(rawValue: reported)
        recognizedState = known != nil
        // Neither an unfamiliar state nor an inconsistent observed flag
        // earns the claim that Desktop has successfully used this route.
        state = known == .observed && !successfulCall ? .ready : (known ?? .unavailable)
        observed = state == .observed && successfulCall
        model = try values.decodeIfPresent(String.self, forKey: .model)
        backgroundModel = try values.decodeIfPresent(String.self, forKey: .backgroundModel)
        let table = try values.decode([String: String].self, forKey: .copy)
        guard Self.copyKeys.allSatisfy({ !(table[$0] ?? "").isEmpty }) else {
            throw DecodingError.dataCorruptedError(forKey: .copy, in: values,
                                                   debugDescription: "claude-desktop-copy-incomplete")
        }
        copy = table
        let text = try values.decodeIfPresent(String.self, forKey: .environment)
        environment = state == .ready || state == .observed ? text : nil
    }

    public var statusLine: String { text("status_" + state.rawValue) }
    public func text(_ key: String) -> String { copy[key] ?? "" }
    public var canConfigure: Bool {
        recognizedState && [.disconnected, .ready, .observed, .unavailable].contains(state)
    }
    public var canDisconnect: Bool {
        // An unavailable connection can have no readable binding after a
        // rejected model or corrupt metadata. Disconnect asks the daemon
        // to revoke/reset that state; it does not require a model id.
        recognizedState && state != .disconnected && (model != nil || state == .unavailable)
    }

    /// Only the sheet keeps an explicitly requested environment. Status
    /// snapshots and the settings row never retain the credential text.
    public func withoutEnvironment() -> Self {
        Self(state: state, model: model, backgroundModel: backgroundModel,
             observed: observed, copy: copy, environment: nil, recognizedState: recognizedState)
    }

    private init(state: State, model: String?, backgroundModel: String?, observed: Bool,
                 copy: [String: String], environment: String?, recognizedState: Bool) {
        self.state = state
        self.model = model
        self.backgroundModel = backgroundModel
        self.observed = observed
        self.copy = copy
        self.environment = environment
        self.recognizedState = recognizedState
    }
}

/// What the Private AI tab draws for Claude Desktop.
///
/// - `offer`: the flag is on, Claude Desktop is installed on this Mac, the
///   person has not dismissed the offer, the daemon says it is not set up
///   (`disconnected`), and the daemon sends the offer's words.
/// - `connected`: the flag is on and it is set up (`ready` or `observed`):
///   its status with Disconnect, whatever became of the offer or the app,
///   so a live connection can always be turned off from the tab.
/// - `hidden`: anything else. Other states either cannot be set up from
///   here or say why in Settings' row.
public enum ClaudeDesktopPrompt {
    public enum Mode: Equatable, Sendable { case hidden, offer, connected }

    /// Claude Desktop's bundle identifier.
    public static let desktopBundleID = "com.anthropic.claudefordesktop"
    /// The `UserDefaults` key holding the offer's dismissal.
    public static let dismissedKey = "claudeDesktopPromptDismissed"

    public static func mode(featureOn: Bool, desktopInstalled: Bool, dismissed: Bool,
                            status: ClaudeDesktopStatus?) -> Mode {
        guard featureOn, let status else { return .hidden }
        if [.ready, .observed].contains(status.state), status.canDisconnect { return .connected }
        guard status.state == .disconnected, desktopInstalled, !dismissed,
              ClaudeDesktopStatus.promptCopyKeys.allSatisfy({ !status.text($0).isEmpty }) else { return .hidden }
        return .offer
    }
}
