import SwiftUI
import TCDesign

/// One Settings section, on the glass system (R11 of #1173). The window's
/// list picks the section; this draws it. Compute has its own view.
struct GlassSettingsContent: View {
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
            case .privateAI: PrivateAISection()
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
