/// Home's pages. Top-level rather than nested in `HomeTabView` (which names
/// it `HomeTabView.Page`), so `MonitorDestination` can name one without
/// reaching into a view.
enum HomePage: String {
    case overview
    case history
    case missions
    case insights
    case missionDrafts
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
/// the Monitor otherwise -- except Inference, which opens the Monitor even
/// while onboarding is required, so Private AI sign-in is reachable before
/// Commons enrollment (R-38). Pure.
enum LaunchRouting {
    enum Window: Equatable { case firstRun, monitor }

    static func window(for destination: MonitorDestination?, requiresOnboarding: Bool) -> Window {
        guard requiresOnboarding else { return .monitor }
        return destination == .inference ? .monitor : .firstRun
    }

    /// What one request opens: its window, and the Settings section it
    /// also opens. The section does not depend on onboarding, so a quit
    /// refusal lands on Compute while first run is still showing.
    struct Opening: Equatable {
        let window: Window
        let settings: SettingsSection?
    }

    static func opening(_ destination: MonitorDestination?, requiresOnboarding: Bool) -> Opening {
        Opening(window: window(for: destination, requiresOnboarding: requiresOnboarding),
                settings: destination?.settingsSection)
    }

    /// Whether the core has said enough to know if onboarding is required:
    /// once its startup is known and, over a running daemon, once its status
    /// has answered, so neither the launch's window nor the Monitor's gate is
    /// chosen from the placeholder status. A daemon that needs its folders,
    /// or refused, has no status to wait for, and onboarding is required
    /// either way. A status read that failed is known too: the status is
    /// still the placeholder, which requires onboarding, so first run opens
    /// rather than nothing (fail closed).
    static func onboardingKnown(startup: AppModel.Startup, statusAnswered: Bool, statusFailed: Bool) -> Bool {
        switch startup {
        case .starting: false
        case .running: statusAnswered || statusFailed
        case .needsRoots, .refused: true
        }
    }
}
