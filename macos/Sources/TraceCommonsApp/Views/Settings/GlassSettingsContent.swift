import SwiftUI
import TCDesign

/// One Settings section, on the glass system (R11 of #1173). The Monitor's
/// Settings modal draws every section, one after another. Compute has its
/// own view.
struct GlassSettingsContent: View {
    var navigation: MainWindowNavigation?
    let section: SettingsSection
    /// The Private AI pointer's way out of the Monitor's Settings modal;
    /// nil elsewhere (`PrivateAISection.onPointer`).
    var onPrivateAI: (() -> Void)? = nil

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
            case .privateAI: PrivateAISection(onPointer: onPrivateAI)
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
