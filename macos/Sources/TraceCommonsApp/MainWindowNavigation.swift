import Observation

/// The services gate and the destination an `OpenMonitor` request left for
/// the Monitor or Settings. Owned above the windows, so a request survives
/// the window it is for being closed.
@Observable @MainActor
final class MainWindowNavigation {
    /// Where an `OpenMonitor` request asked the Monitor to go, until the
    /// Monitor consumes it (`MonitorWindowView`, `.onChange(of:initial:)`).
    var pending: MonitorDestination?
    /// The Settings section a request asked for, until the Settings window
    /// selects it. `@SceneStorage` is per scene and cannot be written from
    /// outside, so the request is parked here.
    var settingsSection: SettingsSection?
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
