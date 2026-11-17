import Foundation

/// The words for declining a whole project from the Waiting screen, decoded
/// from `tc_project_ignore_copy_json` (`project_copy::ignore_project_copy`).
///
/// Every property comes from the payload. This text used to be written here,
/// in each of three shells, and plural agreement was the first thing to
/// drift; the core now assembles the title and the body for the project and
/// the count, and this shell renders them.
///
/// Decoding is here rather than in `TCBridge` so it can be tested without
/// linking the dylib; `TCBridgeTests` checks it against the real export.
public struct ProjectIgnoreCopy: Decodable, Equatable, Sendable {
    /// "Ignore <project>?", naming the project.
    public let title: String
    /// What ignoring removes, that nothing submitted is affected, and the way
    /// back. The removal clause is dropped when nothing is waiting.
    public let body: String
    /// The button, on the row and in the confirmation.
    public let button: String
    /// The button's help text.
    public let tooltip: String
    /// The confirmation's cancel, which keeps the project (#1146).
    public let keep: String
    /// The confirmation's Never button's help text: the way back.
    /// Approved by the owner 2026-11-04 (Settings copy pass 2, P121).
    public let ignoreProjectTooltip: String

    enum CodingKeys: String, CodingKey {
        case title, body, button, tooltip, keep
        case ignoreProjectTooltip = "ignore_project_tooltip"
    }

    /// The payload fields this shell decodes, by wire name.
    public static let consumedFields = ["title", "body", "button", "tooltip", "keep", "ignore_project_tooltip"]

    /// Decode the payload, or nil if it will not parse or a field is empty.
    /// Nil, never a partly-filled value: the confirmation is not offered
    /// without the words that say what it does.
    public static func decode(fromJSON json: String?) -> ProjectIgnoreCopy? {
        guard let data = json?.data(using: .utf8),
            let copy = try? JSONDecoder().decode(ProjectIgnoreCopy.self, from: data)
        else {
            return nil
        }
        return [copy.title, copy.body, copy.button, copy.tooltip, copy.keep, copy.ignoreProjectTooltip]
            .contains(where: \.isEmpty)
            ? nil : copy
    }
}
