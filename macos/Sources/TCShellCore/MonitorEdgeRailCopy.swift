import Foundation

/// The edge rail's words (`preview_copy::MonitorEdgeRailCopy`): the
/// optional rail on the right edge of the screen, its switch in Settings,
/// its icons' names and its peeks. Tools, Private AI and Compute are named
/// by `MonitorSettingsNavCopy`, as Settings names them. A singular is its
/// own line; numbers are `{count}` holes and a section's name `{section}`.
public struct MonitorEdgeRailCopy: MonitorWordTable {
    public let settingToggle: String
    public let settingCaption: String
    public let railLabel: String
    public let handleLabel: String
    public let waiting: String
    public let balance: String
    public let privacy: String
    public let openApp: String
    public let openSection: String
    public let openSectionLabel: String
    public let review: String
    public let toolsEmpty: String
    public let computeUnreported: String
    public let privacyTitle: String
    public let onThisMac: String
    public let onThisMacLineOne: String
    public let onThisMacLine: String
    public let inTheLibrary: String
    public let inTheLibraryLineOne: String
    public let inTheLibraryLine: String
    public let nothingSent: String

    enum CodingKeys: String, CodingKey, CaseIterable {
        case settingToggle = "setting_toggle"
        case settingCaption = "setting_caption"
        case railLabel = "rail_label"
        case handleLabel = "handle_label"
        case waiting
        case balance
        case privacy
        case openApp = "open_app"
        case openSection = "open_section"
        case openSectionLabel = "open_section_label"
        case review
        case toolsEmpty = "tools_empty"
        case computeUnreported = "compute_unreported"
        case privacyTitle = "privacy_title"
        case onThisMac = "on_this_mac"
        case onThisMacLineOne = "on_this_mac_line_one"
        case onThisMacLine = "on_this_mac_line"
        case inTheLibrary = "in_the_library"
        case inTheLibraryLineOne = "in_the_library_line_one"
        case inTheLibraryLine = "in_the_library_line"
        case nothingSent = "nothing_sent"
    }

    public static var consumedFields: [String] { CodingKeys.allCases.map(\.rawValue) }

    /// A peek's link into the app: its visible words and its accessible name.
    public func open(section: String) -> (text: String, label: String) {
        (openSection.replacingOccurrences(of: "{section}", with: section),
         openSectionLabel.replacingOccurrences(of: "{section}", with: section))
    }

    /// The Privacy peek's line for sessions waiting on this Mac.
    public func onThisMac(count: Int) -> String {
        count == 1 ? onThisMacLineOne : onThisMacLine.replacingOccurrences(of: "{count}", with: String(count))
    }

    /// The Privacy peek's line for contributions sent from this Mac.
    public func inTheLibrary(count: Int) -> String {
        count == 1 ? inTheLibraryLineOne : inTheLibraryLine.replacingOccurrences(of: "{count}", with: String(count))
    }
}
