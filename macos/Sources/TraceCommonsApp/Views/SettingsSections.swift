import TCBridge
import TCShellCore

/// The Settings window's sections (spec, "Settings navigation"; R11 of
/// #1173), in the order #1146's settings modal lists them, plus Compute.
/// Each names the part of `SettingsContent` it shows.
enum SettingsSection: String, CaseIterable, Identifiable {
    case connection
    case startup
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

    /// The section's name in the list: the heading the section itself
    /// shows, from the core's copy where the section takes it from there.
    /// Nil while that copy has not loaded, and the list then skips the row
    /// rather than inventing a name.
    @MainActor
    func title(model: AppModel, compute: String?) -> String? {
        switch self {
        case .connection: SettingsWords.connection
        case .startup: SettingsWords.startup
        case .watching: SettingsWords.watching
        case .consent: SettingsContent.consentHeading
        case .publicProfile: PublicProfileCopy.heading
        case .watchedFolders: TCSourceChecks.settingsCopy()?.heading
        case .tools: model.routingCopy?.toolsHeading
        case .privateAI: model.privateInferenceCopy?.settingsTitle
        case .witness: model.witnessCopy?.heading
        case .projects: SettingsWords.projects
        case .changes: SettingsContent.auditHeading
        case .compute: compute ?? SettingsWords.compute
        }
    }

    /// An SF Symbol for the row.
    var symbol: String {
        switch self {
        case .connection: "link"
        case .startup: "power"
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
/// a single word in `SettingsContent` too.
enum SettingsWords {
    static let connection = "Connection"
    static let startup = "Startup"
    static let watching = "Watching"
    static let projects = "Projects"
    static let compute = "Compute"
}
