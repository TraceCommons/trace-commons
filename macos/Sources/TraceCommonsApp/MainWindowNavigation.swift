import Foundation
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

    /// Settings, asked for: the Monitor draws it as Ron's #1146 modal over
    /// its panes while this is set, and closing the modal clears it (#1241
    /// Task 10). Cmd-comma, the Monitor's gear and the menu-bar popover each
    /// set it; nil is closed.
    var settingsRequest: SettingsRequest?

    /// Ask for Settings, opened at `section` (scrolled straight to it), or
    /// at the top for nil. Each request is new, so asking again for the
    /// same section while the modal is open scrolls to it again.
    func requestSettings(at section: SettingsSection? = nil) {
        settingsRequest = SettingsRequest(section: section)
    }
}

/// One ask for Settings: the section to open at, if any.
struct SettingsRequest: Equatable, Identifiable {
    let id = UUID()
    let section: SettingsSection?
}
