import Foundation

/// The command-line tool's words (`preview_copy::MonitorCliToolCopy`): its
/// row in Settings and its app-menu item. `{path}` is the link path.
public struct MonitorCliToolCopy: MonitorWordTable {
    public let title: String
    public let notInstalled: String
    public let installed: String
    public let stale: String
    public let foreign: String
    public let install: String
    public let uninstall: String
    public let updateLink: String
    public let installLabel: String
    public let uninstallLabel: String
    public let updateLinkLabel: String
    public let menuInstall: String
    public let menuUninstall: String
    public let menuUpdateLink: String
    public let failed: String

    enum CodingKeys: String, CodingKey, CaseIterable {
        case title
        case notInstalled = "not_installed"
        case installed
        case stale
        case foreign
        case install
        case uninstall
        case updateLink = "update_link"
        case installLabel = "install_label"
        case uninstallLabel = "uninstall_label"
        case updateLinkLabel = "update_link_label"
        case menuInstall = "menu_install"
        case menuUninstall = "menu_uninstall"
        case menuUpdateLink = "menu_update_link"
        case failed
    }

    public static var consumedFields: [String] { CodingKeys.allCases.map(\.rawValue) }

    private static func fill(_ template: String, path: String) -> String {
        template.replacingOccurrences(of: "{path}", with: path)
    }

    /// The row's status line for `status`, or nil when the row is not shown.
    public func statusLine(_ status: CliToolStatus) -> String? {
        switch status.state {
        case .notInstalled: notInstalled
        case .installed: Self.fill(installed, path: status.linkPath)
        case .stale: Self.fill(stale, path: status.linkPath)
        case .foreign: Self.fill(foreign, path: status.linkPath)
        case .unavailable: nil
        }
    }

    /// The line shown when a change did not happen (never for a cancelled
    /// password prompt).
    public func failedLine(path: String) -> String { Self.fill(failed, path: path) }

    /// The row's button for `action`.
    public func button(_ action: CliToolStatus.Action) -> String {
        switch action {
        case .install: install
        case .updateLink: updateLink
        case .uninstall: uninstall
        }
    }

    /// The button's accessible name.
    public func label(_ action: CliToolStatus.Action) -> String {
        switch action {
        case .install: installLabel
        case .updateLink: updateLinkLabel
        case .uninstall: uninstallLabel
        }
    }

    /// The app menu's item for `action`.
    public func menuItem(_ action: CliToolStatus.Action) -> String {
        switch action {
        case .install: menuInstall
        case .updateLink: menuUpdateLink
        case .uninstall: menuUninstall
        }
    }
}
