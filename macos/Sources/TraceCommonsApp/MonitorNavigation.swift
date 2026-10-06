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
/// Commons enrollment (R-38), and a daemon that refused to start, which
/// opens the Monitor at the core's refusal: a refused daemon has no status,
/// so the placeholder's "onboarding required" says nothing about the
/// person, and first run cannot be finished without the daemon anyway.
/// Pure.
enum LaunchRouting {
    enum Window: Equatable { case firstRun, monitor }

    /// Before the core has said whether onboarding is required, the
    /// Monitor: it waits on the core and then applies its own gate, so a
    /// cold-start request is never routed from the placeholder status.
    static func window(
        for destination: MonitorDestination?, startup: AppModel.Startup, requiresOnboarding: Bool, onboardingKnown: Bool
    ) -> Window {
        if case .refused = startup { return .monitor }
        guard onboardingKnown, requiresOnboarding else { return .monitor }
        return destination == .inference ? .monitor : .firstRun
    }

    /// Where finishing first run goes: the destination an earlier opener
    /// left waiting, else Home.
    static func handOff(pending: MonitorDestination?) -> MonitorDestination {
        pending ?? .home(.overview)
    }

    /// What the launch's own request opens (R-44). A launch is uniform
    /// (`AppDelegate`: a login launch cannot be told from any other), so
    /// it is quiet: an onboarded install opens nothing, and the menu-bar
    /// item and the Dock are the way in. First run opens, activated, while
    /// onboarding is required; the Monitor opens, without taking focus,
    /// over a daemon that refused to start, so a watcher that is not
    /// running is not silent. Until a running daemon's status has
    /// answered, the launch waits: a failed read is retried by the next
    /// status event, and is not taken as "onboarding required" for an
    /// install that may have finished it long ago.
    enum LaunchOpening: Equatable { case wait, nothing, firstRun, monitor }

    static func launchOpening(startup: AppModel.Startup, statusAnswered: Bool, requiresOnboarding: Bool) -> LaunchOpening {
        switch startup {
        case .starting: .wait
        case .refused: .monitor
        case .needsRoots: .firstRun
        case .running: !statusAnswered ? .wait : requiresOnboarding ? .firstRun : .nothing
        }
    }

    /// What one request opens: its window, and the Settings section it
    /// opens. A Settings destination (a quit refusal's Compute, "Manage
    /// rules") opens Settings alone, whatever onboarding says: no window
    /// behind it, which a Settings destination has nothing to show in.
    struct Opening: Equatable {
        let window: Window?
        let settings: SettingsSection?
    }

    static func opening(
        _ destination: MonitorDestination?, startup: AppModel.Startup, requiresOnboarding: Bool, onboardingKnown: Bool
    ) -> Opening {
        if let section = destination?.settingsSection { return Opening(window: nil, settings: section) }
        return Opening(
            window: window(for: destination, startup: startup, requiresOnboarding: requiresOnboarding,
                           onboardingKnown: onboardingKnown),
            settings: nil)
    }

    /// Whether the Monitor takes a pending destination now. Only one it can
    /// show: before the core has said, or while onboarding is required, a
    /// Home or Traces destination stays pending for first run to hand off
    /// (`handOff`), rather than being consumed by a Monitor that draws
    /// Inference alone. Inference is shown either way; a Settings
    /// destination is the Settings window's and never waits here.
    static func monitorConsumes(_ destination: MonitorDestination, requiresOnboarding: Bool, onboardingKnown: Bool) -> Bool {
        if destination.settingsSection != nil || destination == .inference { return true }
        return onboardingKnown && !requiresOnboarding
    }

    /// Whether the core has said enough to know if onboarding is required:
    /// once its startup is known and, over a running daemon, once its status
    /// has answered, so the Monitor's gate is never chosen from the
    /// placeholder status. A daemon that needs its folders has no status
    /// to wait for, and onboarding is required. A daemon that refused has
    /// none either; the Monitor draws the refusal (`MonitorGate`), not
    /// "onboarding required", and a request goes to the Monitor (`window`).
    /// A status read that failed is known too, for the gates: the status is
    /// still the placeholder, which requires onboarding, so a write surface
    /// stays closed (fail closed). The launch does not open first run on it
    /// (`launchOpening`).
    static func onboardingKnown(startup: AppModel.Startup, statusAnswered: Bool, statusFailed: Bool) -> Bool {
        switch startup {
        case .starting: false
        case .running: statusAnswered || statusFailed
        case .needsRoots, .refused: true
        }
    }
}

/// What the Monitor's main pane, and a Settings section that writes what
/// first run asks, draw in place of their content. Pure.
enum MonitorGate: Equatable {
    /// The core has not said whether onboarding is required.
    case awaiting
    /// The daemon refused to start: the core's down title over the
    /// refusal's sentence, for anyone, onboarded or not.
    case down(String)
    /// Onboarding is required: the onboarding notice, whose button opens
    /// first run.
    case signedOut
    /// Nothing in the way.
    case open

    static func of(startup: AppModel.Startup, onboardingKnown: Bool, requiresOnboarding: Bool) -> MonitorGate {
        if case .refused(let sentence) = startup { return .down(sentence) }
        guard onboardingKnown else { return .awaiting }
        return requiresOnboarding ? .signedOut : .open
    }

    /// The gate for one Settings section: a section available before
    /// onboarding (R-43) is never gated.
    func forSettings(availableBeforeOnboarding: Bool) -> MonitorGate {
        availableBeforeOnboarding ? .open : self
    }
}
