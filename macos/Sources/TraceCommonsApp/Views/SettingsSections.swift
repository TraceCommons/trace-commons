import TCBridge
import TCShellCore

/// The Settings window's sections (spec, "Settings navigation"; R11 of
/// #1173), in the order #1146's settings modal lists them, plus Compute.
/// Each names the section `GlassSettingsContent` draws for it.
enum SettingsSection: String, CaseIterable, Identifiable {
    case connection
    case startup
    case notifications
    case updates
    case watching
    case consent
    case publicProfile
    case watchedFolders
    case tools
    case privateAI
    case witness
    case projects
    case changes
    case compute

    var id: String { rawValue }

    /// Whether the Settings window draws this section while onboarding is
    /// required (R-43): only the sections that ask nothing first run asks.
    /// Consent, the public profile, the watched folders, the tools, the
    /// witness, the projects -- and the pause and change log beside them --
    /// wait for first run, so no write surface exists outside it.
    var availableBeforeOnboarding: Bool {
        switch self {
        case .connection, .startup, .notifications, .updates, .privateAI, .compute: true
        case .watching, .consent, .publicProfile, .watchedFolders, .tools, .witness, .projects, .changes: false
        }
    }

    /// Whether the list names this section (#1146's twelve,
    /// `sections.ts`). Notifications and Updates are drawn in the body
    /// under Startup's name, as #1146 draws them under "Startup &
    /// notifications"; a request for either still scrolls to it.
    var isListed: Bool { self != .notifications && self != .updates }

    /// The sections the list names, in its order.
    static let listed: [SettingsSection] = allCases.filter(\.isListed)

    /// The name #1146 lists this section by, from the core's table: the
    /// list's row and the rule that opens the section in the body. Nil for
    /// a section drawn under the one before it, and while the table has not
    /// loaded; the list then draws a disabled placeholder (`ListRow`)
    /// rather than inventing a name or hiding the section.
    func navName(_ nav: MonitorSettingsNavCopy?) -> String? {
        guard let nav else { return nil }
        switch self {
        case .connection: return nav.connection
        case .startup: return nav.startup
        case .notifications, .updates: return nil
        case .watching: return nav.watching
        case .consent: return nav.uses
        case .publicProfile: return nav.profile
        case .watchedFolders: return nav.folders
        case .tools: return nav.tools
        case .privateAI: return nav.privateAI
        case .witness: return nav.witness
        case .projects: return nav.projects
        case .changes: return nav.log
        case .compute: return nav.compute
        }
    }

    /// The row the list draws for this section.
    func listRow(_ nav: MonitorSettingsNavCopy?) -> ListRow {
        ListRow.row(title: navName(nav))
    }

    /// What the list draws for a section: its title, or, while the copy
    /// that names it has not loaded, a disabled placeholder. A row never
    /// vanishes: a section whose copy never loads must still be visible,
    /// and a restored selection must still point at a row.
    struct ListRow: Equatable {
        let text: String
        let enabled: Bool

        static func row(title: String?) -> ListRow {
            guard let title, !title.isEmpty else { return ListRow(text: "—", enabled: false) }
            return ListRow(text: title, enabled: true)
        }
    }

    /// An SF Symbol for the row.
    var symbol: String {
        switch self {
        case .connection: "link"
        case .startup: "power"
        case .notifications: "bell"
        case .updates: "arrow.down.circle"
        case .watching: "eye"
        case .consent: "checkmark.shield"
        case .publicProfile: "person.crop.circle"
        case .watchedFolders: "folder"
        case .tools: "wrench.and.screwdriver"
        case .privateAI: "key"
        case .witness: "seal"
        case .projects: "square.stack"
        case .changes: "clock.arrow.circlepath"
        case .compute: "cpu"
        }
    }
}

/// The single words the section list shows for sections whose heading is
/// a single word on the section itself too.
enum SettingsWords {
    static let connection = "Connection"
    static let startup = "Startup"
    static let updates = "Updates"
    static let watching = "Watching"
    static let projects = "Projects"
}
