import Observation

/// Window selection is independent of watcher startup and trace onboarding.
/// Owning it above the window also preserves selection when the window closes.
@Observable @MainActor
final class MainWindowNavigation {
    var section: MainWindowView.Section = .insights
    private(set) var servicesActivated = false
    /// Local Insights and Mission drafts never activate discovery, enrollment, or network work.
    func activateServicesIfNeeded(_ start: () -> Void) {
        guard section != .insights, section != .missionDrafts, !servicesActivated else { return }
        servicesActivated = true
        start()
    }
    var displaysInsights: Bool { section == .insights }
    var displaysCompute: Bool { section == .compute }
}
