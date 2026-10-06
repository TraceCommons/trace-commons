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
    /// The Settings section a request asked for, until the Settings window
    /// selects it. `@SceneStorage` is per scene and cannot be written from
    /// outside, so the request is parked here.
    var settingsSection: SettingsSection?
    /// Bumped by every request that leaves a destination, and observed by
    /// the Monitor rather than `pending` itself: first run's hand-off asks
    /// for the destination already waiting, which an open Monitor would
    /// otherwise never see, because the value did not change.
    private(set) var requests = 0

    /// Leaves one request's destination for the Monitor. A request with
    /// none (a Dock click, an invite link) leaves a waiting destination in
    /// place rather than clearing it, and a Settings destination is the
    /// Settings window's (`settingsSection`), never the Monitor's.
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

    /// The Settings window and the monitor read and write the daemon, so
    /// opening one starts services. Once, shared with
    /// `activateServicesIfNeeded`.
    func activateServicesForWindow() {
        guard !servicesActivated, let serviceStart else { return }
        servicesActivated = true
        serviceStart()
    }
}
