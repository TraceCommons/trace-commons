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

    /// The section's name in the list: the heading the section itself
    /// shows, from the core's copy where the section takes it from there.
    /// Nil while that copy has not loaded; the list then draws the row as a
    /// disabled placeholder (`ListRow`) rather than inventing a name or
    /// hiding the section.
    func title(_ sources: TitleSources) -> String? {
        switch self {
        case .connection: SettingsWords.connection
        case .startup: SettingsWords.startup
        case .notifications: sources.notifications
        case .updates: SettingsWords.updates
        case .watching: SettingsWords.watching
        case .consent: SettingsContent.consentHeading
        case .publicProfile: PublicProfileCopy.heading
        case .watchedFolders: sources.watchedFolders
        case .tools: sources.tools
        case .privateAI: sources.privateAI
        case .witness: sources.witness
        case .projects: SettingsWords.projects
        case .changes: SettingsContent.auditHeading
        case .compute: sources.compute ?? SettingsWords.compute
        }
    }

    /// The row the list draws for this section.
    func listRow(_ sources: TitleSources) -> ListRow {
        ListRow.row(title: title(sources))
    }

    /// The headings that come from loaded copy, each nil until its copy
    /// has loaded. Held apart from `AppModel` so the list's rows can be
    /// checked with any of them missing.
    struct TitleSources {
        var notifications: String?
        var watchedFolders: String?
        var tools: String?
        var privateAI: String?
        var witness: String?
        var compute: String?

        @MainActor
        init(model: AppModel, compute: String?) {
            notifications = Notifier.copy?.notificationHeading
            watchedFolders = TCSourceChecks.settingsCopy()?.heading
            tools = model.routingCopy?.toolsHeading
            privateAI = model.privateInferenceCopy?.settingsTitle
            witness = model.witnessCopy?.heading
            self.compute = compute
        }

        init(
            notifications: String? = nil, watchedFolders: String? = nil, tools: String? = nil,
            privateAI: String? = nil, witness: String? = nil, compute: String? = nil
        ) {
            self.notifications = notifications
            self.watchedFolders = watchedFolders
            self.tools = tools
            self.privateAI = privateAI
            self.witness = witness
            self.compute = compute
        }
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
    static let compute = "Compute"
}
