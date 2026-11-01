import Foundation

/// The command-line tool's link, as the core reads it (`tc_cli_tool_json`,
/// `cli_tool::probe`): the link path for this build's channel, what is
/// there, and the AppleScript for each action the core offers.
///
/// Swift composes no path, name or script: the core validated and quoted
/// every one of them, and this shell only runs what it was given.
public struct CliToolStatus: Decodable, Equatable, Sendable {
    public enum State: String, Decodable, Sendable {
        case notInstalled = "not_installed"
        case installed
        case stale
        case foreign
        case unavailable
    }

    /// What the row and the menu can do.
    public enum Action: String, CaseIterable, Identifiable, Sendable {
        case install
        case updateLink
        case uninstall

        public var id: String { rawValue }
    }

    public let commandName: String
    public let linkPath: String
    public let state: State
    public let installScript: String?
    public let uninstallScript: String?

    enum CodingKeys: String, CodingKey {
        case commandName = "command_name"
        case linkPath = "link_path"
        case state
        case installScript = "install_script"
        case uninstallScript = "uninstall_script"
    }

    /// The core's answer, or nil when it will not parse (an unknown state
    /// included): the row then shows nothing rather than guessing.
    public static func decode(fromJSON json: String?) -> CliToolStatus? {
        guard let data = json?.data(using: .utf8) else { return nil }
        return try? JSONDecoder().decode(CliToolStatus.self, from: data)
    }

    /// The actions offered, in the row's order. Each needs its script: an
    /// action the core gave no script for is not offered.
    public var actions: [Action] {
        let wanted: [Action] =
            switch state {
            case .notInstalled: [.install]
            case .installed: [.uninstall]
            case .stale: [.updateLink, .uninstall]
            case .foreign, .unavailable: []
            }
        return wanted.filter { script(for: $0) != nil }
    }

    /// The menu's one item: the first action offered.
    public var primaryAction: Action? { actions.first }

    /// The AppleScript `action` runs, or nil when it is not offered.
    public func script(for action: Action) -> String? {
        switch (action, state) {
        case (.install, .notInstalled), (.updateLink, .stale): installScript
        case (.uninstall, .installed), (.uninstall, .stale): uninstallScript
        default: nil
        }
    }
}

/// How a privileged change ended, from `osascript`'s exit status and error
/// output.
public enum CliToolOutcome: Equatable, Sendable {
    case done
    /// The password prompt was cancelled: AppleScript's error -128. Nothing
    /// changed, and nothing is shown.
    case cancelled
    case failed

    public static func classify(exitStatus: Int32, standardError: String) -> CliToolOutcome {
        if exitStatus == 0 { return .done }
        return standardError.contains("(-128)") ? .cancelled : .failed
    }
}
