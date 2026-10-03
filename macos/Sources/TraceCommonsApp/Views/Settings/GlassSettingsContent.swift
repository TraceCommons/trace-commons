import SwiftUI
import TCDesign

/// One Settings section, on the glass system (R11 of #1173). The window's
/// list picks the section; this draws it. Compute has its own view.
struct GlassSettingsContent: View {
    var navigation: MainWindowNavigation?
    let section: SettingsSection

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
            switch section {
            case .connection: ConnectionSection()
            case .startup: StartupSection()
            case .notifications: NotificationsSection()
            case .updates: UpdatesSection()
            case .consent: ConsentSection()
            case .publicProfile: PublicProfileSection()
            case .watching: WatchingSection()
            case .watchedFolders: WatchedFoldersSection()
            case .tools: ToolsSection()
            case .privateAI: PrivateAISection(navigation: navigation)
            case .witness: WitnessSection()
            case .projects: ProjectsSection()
            case .changes: ChangesSection()
            case .compute: EmptyView()
            }
        }
        .padding(GlassTokens.Space.panePadding)
        .frame(maxWidth: 560, alignment: .leading)
        .frame(maxWidth: .infinity, alignment: .topLeading)
    }
}

// Stubs: each later task replaces its stub with the real section file and
// deletes the stub here. Until then the legacy body draws the section.

struct WatchedFoldersSection: View {
    var body: some View { SettingsContent(section: .watchedFolders) }
}

struct ToolsSection: View {
    var body: some View { SettingsContent(section: .tools) }
}

struct PrivateAISection: View {
    var navigation: MainWindowNavigation?

    var body: some View { SettingsContent(navigation: navigation, section: .privateAI) }
}

struct WitnessSection: View {
    var body: some View { SettingsContent(section: .witness) }
}

struct ProjectsSection: View {
    var body: some View { SettingsContent(section: .projects) }
}
