/// Home's pages. Top-level rather than nested in `HomeTabView` (which names
/// it `HomeTabView.Page`) so a release build, where the Monitor's views are
/// still debug-only until R15, can name a destination (ruling R-33).
enum HomePage: String {
    case overview
    case history
    case missions
}

/// Where an outside caller wants the Monitor to go. Nothing here sends.
enum MonitorDestination: Equatable, Sendable {
    case home(HomePage)
    case inference
    case traces(entryId: String?)
    case settings(SettingsSection)

    /// The Settings section this destination opens, nil for a Monitor tab.
    var settingsSection: SettingsSection? {
        if case .settings(let section) = self { return section }
        return nil
    }
}

/// Opening the Monitor from outside a SwiftUI view: a notification action,
/// a Dock click, an invite link, a quit refusal, `TRACE_COMMONS_SHOW_WINDOW`.
///
/// Requests that arrive before the handler exists are held rather than
/// dropped, and replayed once when it is installed; the last destination
/// wins. That is not defensive coding: the handler is installed from a
/// `.task` on the menu-bar label, and `applicationDidFinishLaunching` and a
/// `tracecommons://` link that launches the app can both beat it. Dropping
/// those would make exactly the cold-start cases fail while every warm test
/// passed.
enum OpenMonitor {
    @MainActor static var handler: ((MonitorDestination?) -> Void)? {
        didSet { replayIfPending() }
    }

    /// Whether a request is held, and its destination. A held request with
    /// no destination (a Dock click) is still a request.
    @MainActor private static var pending: (held: Bool, destination: MonitorDestination?) = (false, nil)

    @MainActor
    static func request(_ destination: MonitorDestination? = nil) {
        guard let handler else {
            pending = (true, destination)
            return
        }
        handler(destination)
    }

    /// Tests only: no handler and nothing held.
    @MainActor
    static func reset() {
        pending = (false, nil)
        handler = nil
    }

    @MainActor
    private static func replayIfPending() {
        guard let handler, pending.held else { return }
        let destination = pending.destination
        pending = (false, nil)
        handler(destination)
    }
}

/// Which window a request opens: first run while onboarding is required,
/// the Monitor otherwise. Pure.
enum LaunchRouting {
    enum Window: Equatable { case firstRun, monitor }

    static func window(requiresOnboarding: Bool) -> Window {
        requiresOnboarding ? .firstRun : .monitor
    }
}
