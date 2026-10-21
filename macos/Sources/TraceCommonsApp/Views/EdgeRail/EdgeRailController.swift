import AppKit
import Observation
import SwiftUI
import TCDesign
import TCShellCore

/// The edge rail ("Run 1 OS-level Explorations", 1b): rest the pointer on
/// the right edge of the screen and a rail of glances slides in, each icon
/// peeking a summary of one part of the app with a way into it.
///
/// Optional, and off until the contributor turns it on in Settings
/// (`EdgeRailPreference`). It lives in a panel of its own that floats over
/// every app on every Space, never takes focus from the app in front, and
/// shrinks back to a thin hover zone when the pointer leaves it -- so while
/// closed it covers four points of screen edge and nothing else.
///
/// The rail reads. Its one write is the watcher's pause and resume, the
/// same one the menu-bar panel offers; everything else opens the app.
@MainActor
final class EdgeRailController {
    static let shared = EdgeRailController()

    private var panel: EdgeRailPanel?
    private let state = EdgeRailState()
    private weak var model: AppModel?
    private var screenObserver: NSObjectProtocol?

    /// Hands the rail the app's model, and shows it if the switch is on.
    func attach(model: AppModel) {
        self.model = model
        apply(EdgeRailPreference.isEnabled())
    }

    /// The Settings switch: remembers the choice and shows or removes the
    /// rail at once.
    func setEnabled(_ enabled: Bool) {
        EdgeRailPreference.set(enabled)
        apply(enabled)
    }

    private func apply(_ enabled: Bool) {
        if enabled { show() } else { remove() }
    }

    private func show() {
        guard panel == nil, let model else { return }
        let panel = EdgeRailPanel()
        let state = state
        let root = EdgeRailView(state: state) { [weak self] open, animation in
            Self.transition(to: open, state: state, animation: animation) { self?.place(open: $0) }
        }
            .environmentObject(model)
            .tint(GlassTokens.Color.purpleSoft.color)
            // No focus ring drawn round controls (owner, 2026-10-11).
            .focusEffectDisabled()
            // The panel never becomes key, but the rail is what the person
            // is using while it is open: it draws in focus, without the
            // flat theme's out-of-focus dim (`inactiveDim`).
            .environment(\.controlActiveState, .key)
            // Names this window in the log when it resizes.
            .windowBreadcrumb(.edgeRail)
        panel.contentView = Self.hostingView(root)
        self.panel = panel
        state.open = false
        state.wanted = false
        place(open: false)
        panel.orderFrontRegardless()
        // A display arriving, leaving or changing resolution moves the edge.
        screenObserver = NotificationCenter.default.addObserver(
            forName: NSApplication.didChangeScreenParametersNotification, object: nil, queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.place(open: self?.placedOpen ?? false) }
        }
    }

    private func remove() {
        if let screenObserver { NotificationCenter.default.removeObserver(screenObserver) }
        screenObserver = nil
        panel?.orderOut(nil)
        panel = nil
        state.open = false
        state.wanted = false
        state.closing = 0
    }

    /// The rail's hosting view, which never sizes its window. The panel's
    /// frame is `place(open:)`'s alone: a hosting view on its default
    /// sizing options also sizes the window to its SwiftUI content, and the
    /// two fought inside one layout pass until AppKit threw from
    /// `_postWindowNeedsUpdateConstraints` and the app aborted. The rail is
    /// borderless at a screen edge, so it takes no safe-area insets either,
    /// which keeps the insets' invalidation out of that pass as well.
    static func hostingView<Content: View>(_ root: Content) -> NSHostingView<Content> {
        let hosting = NSHostingView(rootView: root)
        hosting.sizingOptions = []
        hosting.safeAreaRegions = []
        return hosting
    }

    /// Runs a placement on the next turn of the run loop. Opening and
    /// closing are asked for from inside SwiftUI's hover handling and
    /// animation, and a window frame set there re-enters the layout pass
    /// that is already running.
    static func deferred(_ place: @escaping @MainActor (Bool) -> Void) -> (Bool) -> Void {
        { open in DispatchQueue.main.async { MainActor.assumeIsolated { place(open) } } }
    }

    /// Opens or closes the rail so that the panel's size never changes
    /// while the rail's content is changing (#1517). The panel is the
    /// controller's to size (`hostingView`), but a size change inside the
    /// open or close animation still reached the window through
    /// `NSHostingView.updateAnimatedWindowSize`, from `windowDidLayout`,
    /// until AppKit's layout-loop guard threw and the app aborted -- with
    /// the panel at its closed 4 x 240 pt.
    ///
    /// - Opening: the panel grows on the next turn (`deferred`), with the
    ///   closed handle in it, which fills any size; the rail opens, under
    ///   `animation`, on the turn after. The open rail never lays out in
    ///   the hover zone.
    /// - Closing: the rail closes now, under `animation`, at the open
    ///   panel's size, and the panel shrinks on the turn after the close
    ///   has finished. The closing rail never lays out in the hover zone.
    ///
    /// The last request wins: a step queued for a request since overtaken
    /// (hovering back in while the rail closes) does nothing.
    static func transition(
        to open: Bool, state: EdgeRailState, animation: Animation?, place: @escaping @MainActor (Bool) -> Void
    ) {
        state.wanted = open
        if open {
            deferred { _ in
                guard state.wanted else { return }
                place(true)
                deferred { _ in
                    guard state.wanted else { return }
                    withAnimation(animation) { state.open = true }
                    // A close asked for from inside that write (the change
                    // runs SwiftUI's update, and its hover handling, before
                    // the write lands) was overwritten by it: close again.
                    if !state.wanted { transition(to: false, state: state, animation: animation, place: place) }
                }(true)
            }(true)
        } else {
            // Only a close that takes the rail away has a transition to
            // wait for; one asked for while the rail is already closed
            // (or still opening) waits for any close still running.
            let closes = state.open
            if closes { state.closing += 1 }
            withAnimation(animation, completionCriteria: .removed) {
                state.open = false
            } completion: {
                if closes { state.closing = max(0, state.closing - 1) }
                deferred { _ in
                    guard !state.wanted, !state.open, state.closing == 0 else { return }
                    place(false)
                }(false)
            }
        }
    }

    /// The size the panel was last placed at, which a screen change keeps:
    /// mid-close the panel is still open-sized while `state.open` is not.
    private var placedOpen = false

    /// Moves the panel to the main display's right edge, at the size the
    /// rail's state needs.
    private func place(open: Bool) {
        placedOpen = open
        guard let panel, let screen = NSScreen.screens.first else { return }
        let frame = EdgeRailGeometry.frame(open: open, visible: screen.visibleFrame)
        panel.setFrame(frame, display: true)
    }
}

/// What the rail shows: open or closed, and which icon's peek.
@Observable
@MainActor
final class EdgeRailState {
    /// Whether the rail's content is open. Set only by
    /// `EdgeRailController.transition(to:)`, after the panel has grown or
    /// before it shrinks.
    var open = false
    /// The open or close last asked for, which a queued step checks before
    /// it runs.
    @ObservationIgnored var wanted = false
    /// Closes whose transition is still running: the panel shrinks only
    /// once there are none.
    @ObservationIgnored var closing = 0
    var peek: EdgeRailPeek = .home
}

/// The rail's icons, top to bottom (owner, 2026-10-11): one per tab of
/// the app, each peek a compact summary of that tab. The app's own tile
/// closes the rail.
enum EdgeRailPeek: CaseIterable, Identifiable {
    case home, privateAI, traces

    var id: Self { self }

    /// The icon: a house, a shield for Private AI (owner, 2026-10-11, in
    /// place of the key), and the Trace mark for Traces.
    var icon: EdgeRailIcon {
        switch self {
        case .home: .symbol("house")
        case .privateAI: .symbol("shield")
        case .traces: .traceMark
        }
    }

    /// The icon's name and its peek's title: the app's tab names.
    func name(nav: MonitorSettingsNavCopy?, shell: MonitorShellCopy?) -> String {
        switch self {
        case .home: shell?.tabHome ?? ""
        case .privateAI: nav?.privateAI ?? ""
        case .traces: shell?.tabTraces ?? ""
        }
    }

    /// The tab the peek's link opens.
    var destination: MonitorDestination {
        switch self {
        case .home: .home(.overview)
        case .privateAI: .inference
        case .traces: .traces(entryId: nil)
        }
    }
}

/// What a rail icon draws: an SF Symbol, or the Trace mark.
enum EdgeRailIcon: Equatable {
    case symbol(String)
    /// The Trace logo's live-trace mark (`GlassTraceMark`, Claude Design
    /// 1c, owner 2026-10-12).
    case traceMark
}

/// A borderless panel that floats over every app on every Space and never
/// becomes key: hovering and clicking the rail leaves the app in front in
/// front.
final class EdgeRailPanel: NSPanel {
    init() {
        super.init(contentRect: .zero, styleMask: [.borderless, .nonactivatingPanel],
                   backing: .buffered, defer: false)
        level = .statusBar
        collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .stationary, .ignoresCycle]
        isOpaque = false
        backgroundColor = .clear
        hasShadow = false
        hidesOnDeactivate = false
        isReleasedWhenClosed = false
        becomesKeyOnlyIfNeeded = true
        isMovable = false
    }

    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }
}
