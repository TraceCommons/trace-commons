import SwiftUI
import TCDesign

/// One Settings section, on the glass system (R11 of #1173). The Monitor's
/// Settings modal draws every section, one after another. Compute has its
/// own view.
struct GlassSettingsContent: View {
    var navigation: MainWindowNavigation?
    let section: SettingsSection
    /// Payouts tells the window whether anything in it applies (D2).
    var onPayoutsPresence: (Bool) -> Void = { _ in }

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
            switch section {
            case .account: AccountSection()
            case .payouts: PayoutsSection(onPresence: onPayoutsPresence)
            case .referrals: ReferralsSection()
            case .watching: WatchingSection()
            case .watchedFolders: WatchedFoldersSection()
            case .tools: ToolsSection()
            // The Insights switches, their own section since the Settings IA
            // review (D13); it waits for first run (R-43), as Tools did.
            case .insights:
                InsightsSettingsCard()
                SubscriptionUsageSettingsSection()
            case .projects: ProjectsSection()
            case .consent: ConsentSection()
            case .publicProfile: PublicProfileSection()
            case .changes: ChangesSection()
            case .privateAI: PrivateAISection()
            case .general: GeneralSection()
            case .startup: StartupSection()
            case .notifications: NotificationsSection()
            case .updates: UpdatesSection()
            case .witness: WitnessSection()
            case .compute: EmptyView()
            }
        }
        // The modal body's insets (#1146 `px-5`), under the section's rule;
        // the cards fill the body, as #1146's `.tc-page` has no max width.
        .padding(.horizontal, GlassTokens.Space.s9)
        .padding(.vertical, GlassTokens.Space.s6)
        .frame(maxWidth: .infinity, alignment: .topLeading)
    }
}
