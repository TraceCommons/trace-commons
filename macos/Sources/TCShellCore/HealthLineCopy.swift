import Foundation

/// The health banner's words, decoded from `tc_health_copy_json`
/// (`health_copy::HealthLineCopy`). The core words every label, the core-down
/// banner and the on-hold line for a label it does not know; this type only
/// carries them.
public struct HealthLineCopy: Decodable, Equatable, Sendable {
    public enum Severity: String, Decodable, Sendable {
        /// Something the contributor can act on.
        case actionable
        /// Ambient; it clears on its own.
        case waiting
    }

    /// What the action button does, so a shell switches on a kind and never
    /// on the button's words or the label.
    public enum ActionKind: String, Decodable, Sendable {
        case reconnect
        case privacyScanNotice = "privacy_scan_notice"
        case reviewQueue = "review_queue"
    }

    public let title: String
    public let detail: String
    public let action: String?
    public let actionKind: ActionKind?
    public let severity: Severity

    enum CodingKeys: String, CodingKey {
        case title, detail, action, severity
        case actionKind = "action_kind"
    }

    /// Decode the payload, or nil if it will not parse or a sentence is
    /// empty. Nil is never a healthy daemon: callers fall back to the core's
    /// on-hold line.
    public static func decode(fromJSON json: String?) -> HealthLineCopy? {
        guard let data = json?.data(using: .utf8),
            let copy = try? JSONDecoder().decode(HealthLineCopy.self, from: data)
        else {
            return nil
        }
        return copy.title.isEmpty || copy.detail.isEmpty || copy.action?.isEmpty == true ? nil : copy
    }
}
