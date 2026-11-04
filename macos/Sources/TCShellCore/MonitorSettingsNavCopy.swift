import Foundation

/// Ron's #1146 Settings section names (`features/settings/sections.ts`;
/// `preview_copy::MonitorSettingsNavCopy`): the Settings modal's section
/// list, and the rule that opens each section in its body. Plain labels,
/// in his order.
public struct MonitorSettingsNavCopy: MonitorWordTable {
    public let connection: String
    public let startup: String
    public let watching: String
    public let uses: String
    public let profile: String
    public let folders: String
    public let tools: String
    public let privateAI: String
    public let witness: String
    public let projects: String
    public let log: String
    public let compute: String

    enum CodingKeys: String, CodingKey, CaseIterable {
        case connection
        case startup
        case watching
        case uses
        case profile
        case folders
        case tools
        case privateAI = "private_ai"
        case witness
        case projects
        case log
        case compute
    }

    public static var consumedFields: [String] { CodingKeys.allCases.map(\.rawValue) }
}
