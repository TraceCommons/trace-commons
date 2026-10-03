import Observation

/// Window selection is independent of watcher startup and trace onboarding.
/// Owning it above the window also preserves selection when the window closes.
@Observable @MainActor
final class MainWindowNavigation {
    /// Where an `OpenMonitor` request asked the Monitor to go, until the
    /// Monitor consumes it (`MonitorWindowView`, `.onChange(of:initial:)`).
    var pending: MonitorDestination?
    /// The Settings section a request asked for, until the Settings window
    /// selects it. `@SceneStorage` is per scene and cannot be written from
    /// outside, so the request is parked here.
    var settingsSection: SettingsSection?

    /// The legacy main window's sidebar. A remnant until R15: T11 deletes
    /// it, `displaysInsights`, `displaysCompute` and `legacySection(for:)`
    /// with `MainWindowView` (ruling R-33).
    var section: MainWindowView.Section = .insights
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

    var displaysInsights: Bool { section == .insights }
    var displaysCompute: Bool { section == .compute }

    /// The legacy window's section for a destination, for a release build
    /// until R15 (T11 deletes this): nil leaves the window where it was.
    static func legacySection(for destination: MonitorDestination?) -> MainWindowView.Section? {
        switch destination {
        case nil: nil
        case .traces: .queue
        case .home(.history): .history
        case .home: nil
        case .inference: .privateInference
        case .settings(.compute): .compute
        case .settings: .settings
        }
    }
}
