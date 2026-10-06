import Foundation
import Observation

/// The services gate and the destination an `OpenMonitor` request left for
/// the Monitor or Settings. Owned above the windows, so a request survives
/// the window it is for being closed.
@Observable @MainActor
final class MainWindowNavigation {
    /// Where an `OpenMonitor` request asked the Monitor to go, until the
    /// Monitor consumes it (`MonitorWindowView`), which it does only once it
    /// can show it (`LaunchRouting.monitorConsumes`). Until then it waits,
    /// and first run hands it off (`LaunchRouting.handOff`).
    var pending: MonitorDestination?
    /// Bumped by every request that leaves a destination, and observed by
    /// the Monitor rather than `pending` itself: first run's hand-off asks
    /// for the destination already waiting, which an open Monitor would
    /// otherwise never see, because the value did not change.
    private(set) var requests = 0

    /// Leaves one request's destination for the Monitor. A request with
    /// none (a Dock click, an invite link) leaves a waiting destination in
    /// place rather than clearing it, and a Settings destination is the
    /// Settings modal's (`requestSettings(at:)`), never the Monitor's.
    func leave(_ destination: MonitorDestination?) {
        guard let destination, destination.settingsSection == nil else { return }
        pending = destination
        requests += 1
    }
    private(set) var servicesActivated = false
    /// Starts services once. D-11: no section defers them any more; the
    /// Monitor starts them on appear, and launch starts them here. Insights
    /// and Mission drafts are Home pages, and an Insights-only launch no
    /// longer avoids starting services.
    func activateServicesIfNeeded(_ start: () -> Void) {
        guard !servicesActivated else { return }
        servicesActivated = true
        start()
    }
    /// The start work, registered once at launch, for a window that needs
    /// live services; see `activateServicesForWindow`.
    private var serviceStart: (() -> Void)?

    func registerServiceStart(_ start: @escaping () -> Void) {
        serviceStart = start
    }

    /// The monitor, and the Settings modal over it, read and write the
    /// daemon, so opening the monitor starts services. Once, shared with
    /// `activateServicesIfNeeded`.
    func activateServicesForWindow() {
        guard !servicesActivated, let serviceStart else { return }
        servicesActivated = true
        serviceStart()
    }

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
