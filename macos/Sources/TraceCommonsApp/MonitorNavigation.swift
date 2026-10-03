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
    /// Opens a destination; the flag says whether to activate the app.
    @MainActor static var handler: ((MonitorDestination?, Bool) -> Void)? {
        didSet { replayIfPending() }
    }

    /// Whether a request is held, its destination, and whether it
    /// activates. A held request with no destination (a Dock click) is
    /// still a request.
    @MainActor private static var pending: (held: Bool, destination: MonitorDestination?, activate: Bool) = (false, nil, true)

    /// Every opener activates the app, as a click does; only the launch's
    /// own request may stay quiet (R-44).
    @MainActor
    static func request(_ destination: MonitorDestination? = nil, activate: Bool = true) {
        guard let handler else {
            pending = (true, destination, activate)
            return
        }
        handler(destination, activate)
    }

    /// Tests only: no handler and nothing held.
    @MainActor
    static func reset() {
        pending = (false, nil, true)
        handler = nil
    }

    @MainActor
    private static func replayIfPending() {
        guard let handler, pending.held else { return }
        let held = pending
        pending = (false, nil, true)
        handler(held.destination, held.activate)
    }
}

/// Which window a request opens: first run while onboarding is required,
/// the Monitor otherwise -- except Inference, which opens the Monitor even
/// while onboarding is required, so Private AI sign-in is reachable before
/// Commons enrollment (R-38). Pure.
enum LaunchRouting {
    enum Window: Equatable { case firstRun, monitor }

    /// Before the core has said whether onboarding is required, the
    /// Monitor: it waits on the core and then applies its own gate, so a
    /// cold-start request is never routed from the placeholder status.
    static func window(for destination: MonitorDestination?, requiresOnboarding: Bool, onboardingKnown: Bool) -> Window {
        guard onboardingKnown, requiresOnboarding else { return .monitor }
        return destination == .inference ? .monitor : .firstRun
    }

    /// Where finishing first run goes: the destination an earlier opener
    /// left waiting, else Home.
    static func handOff(pending: MonitorDestination?) -> MonitorDestination {
        pending ?? .home(.overview)
    }

    /// Whether the launch's own request activates the app: only for first
    /// run (R-44). Every other opener activates.
    static func launchActivates(requiresOnboarding: Bool) -> Bool {
        requiresOnboarding
    }

    /// What one request opens: its window, and the Settings section it
    /// also opens. The section does not depend on onboarding, so a quit
    /// refusal lands on Compute while first run is still showing.
    struct Opening: Equatable {
        let window: Window
        let settings: SettingsSection?
    }

    static func opening(_ destination: MonitorDestination?, requiresOnboarding: Bool, onboardingKnown: Bool) -> Opening {
        Opening(window: window(for: destination, requiresOnboarding: requiresOnboarding, onboardingKnown: onboardingKnown),
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
