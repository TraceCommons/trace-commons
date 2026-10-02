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
    /// The start work, registered once at launch, for a window that needs
    /// live services whatever the main window shows; see
    /// `activateServicesForWindow`.
    private var serviceStart: (() -> Void)?

    func registerServiceStart(_ start: @escaping () -> Void) {
        serviceStart = start
    }

    /// The Settings window and the monitor read and write the daemon, so
    /// opening one starts services even while the main window rests on
    /// Insights, which defers them. Once, shared with
    /// `activateServicesIfNeeded`.
    func activateServicesForWindow() {
        guard !servicesActivated, let serviceStart else { return }
        servicesActivated = true
        serviceStart()
    }

    var displaysInsights: Bool { section == .insights }
    var displaysCompute: Bool { section == .compute }
}
