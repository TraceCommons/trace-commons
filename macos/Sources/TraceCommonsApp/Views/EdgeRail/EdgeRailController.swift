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
    private lazy var opening = EdgeRailOpening(state: state) { [weak self] open in self?.place(open: open) }
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
        let root = EdgeRailView(state: state, request: { [weak self] in self?.opening.set($0, animation: $1) })
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
        opening.reset()
        place(open: false)
        panel.orderFrontRegardless()
        // A display arriving, leaving or changing resolution moves the edge.
        screenObserver = NotificationCenter.default.addObserver(
            forName: NSApplication.didChangeScreenParametersNotification, object: nil, queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.place(open: self?.state.open ?? false) }
        }
    }

    private func remove() {
        if let screenObserver { NotificationCenter.default.removeObserver(screenObserver) }
        screenObserver = nil
        panel?.orderOut(nil)
        panel = nil
        opening.reset()
    }

    /// The rail's hosting view. The panel's frame is `place(open:)`'s
    /// alone: a hosting view on its default sizing options also sizes the
    /// window to its SwiftUI content, and the two fought inside one layout
    /// pass until AppKit threw from `_postWindowNeedsUpdateConstraints` and
    /// the app aborted (#1402). The rail is borderless at a screen edge, so
    /// it takes no safe-area insets either, which keeps the insets'
    /// invalidation out of that pass as well. Empty sizing options do not
    /// stop the hosting view's animated resize of its window; the panel
    /// drops that (`EdgeRailPanel`, #1517).
    static func hostingView<Content: View>(_ root: Content) -> NSHostingView<Content> {
        let hosting = NSHostingView(rootView: root)
        hosting.sizingOptions = []
        hosting.safeAreaRegions = []
        return hosting
    }

    /// Moves the panel to the main display's right edge, at the size the
    /// rail's state needs.
    private func place(open: Bool) {
        guard let panel, let screen = NSScreen.screens.first else { return }
        panel.place(EdgeRailGeometry.frame(open: open, visible: screen.visibleFrame))
    }
}

/// Opening and closing the rail, in the order that keeps the rail inside
/// its panel: the panel grows before the rail draws open into it, and
/// shrinks only once the rail has finished sliding out. The rail used to
/// draw open into the four-point closed panel for a turn, and the panel
/// shrank under it while it was still going (#1517).
///
/// Both are asked for from inside SwiftUI's hover handling, so the panel
/// moves on the next turn of the run loop, never inside that update
/// (#1402).
@MainActor
final class EdgeRailOpening {
    typealias Animate = @MainActor (Animation?, () -> Void, @escaping @MainActor () -> Void) -> Void

    private let state: EdgeRailState
    private let nextTurn: @MainActor (@escaping @MainActor () -> Void) -> Void
    private let animate: Animate
    private let place: @MainActor (Bool) -> Void
    /// What was asked for last. A step queued for the other answer finds
    /// it changed and does nothing.
    private var wanted = false

    init(
        state: EdgeRailState,
        nextTurn: @escaping @MainActor (@escaping @MainActor () -> Void) -> Void = { GlassWindowFrame.onNextTurn($0) },
        animate: @escaping Animate = EdgeRailOpening.animate,
        place: @escaping @MainActor (Bool) -> Void
    ) {
        self.state = state
        self.nextTurn = nextTurn
        self.animate = animate
        self.place = place
    }

    /// Opens or closes the rail, with `animation` for the rail itself.
    func set(_ open: Bool, animation: Animation?) {
        wanted = open
        if open {
            nextTurn { [weak self] in
                guard let self, wanted, !state.open else { return }
                place(true)
                animate(animation, { self.state.open = true }, {})
            }
        } else if state.open {
            animate(animation, { self.state.open = false }) { [weak self] in
                self?.nextTurn { [weak self] in
                    guard let self, !wanted, !state.open else { return }
                    place(false)
                }
            }
        }
    }

    /// Closed at once, as a new or removed panel starts.
    func reset() {
        wanted = false
        state.open = false
    }

    /// `body` under `animation`, then `completion` once the rail it
    /// removes has gone.
    static func animate(_ animation: Animation?, _ body: () -> Void, completion: @escaping @MainActor () -> Void) {
        withAnimation(animation, completionCriteria: .removed, body) {
            MainActor.assumeIsolated { completion() }
        }
    }
}

/// What the rail shows: open or closed, and which icon's peek.
@Observable
@MainActor
final class EdgeRailState {
    var open = false
    var peek: EdgeRailPeek = .privateAI
}

/// The rail's icons, top to bottom (owner, 2026-10-11): one per tab of
/// the app, each peek a compact summary of that tab. Home's is gone (owner,
/// 2026-10-22). The app's own tile closes the rail.
enum EdgeRailPeek: CaseIterable, Identifiable {
    case privateAI, traces

    var id: Self { self }

    /// The icon: a shield for Private AI (owner, 2026-10-11, in place of
    /// the key), and four connected nodes in a T for Traces (owner,
    /// 2026-10-22, in place of the Trace mark).
    var icon: EdgeRailIcon {
        switch self {
        case .privateAI: .symbol("shield")
        case .traces: .traceNodes
        }
    }

    /// The icon's name and its peek's title: the app's tab names.
    func name(nav: MonitorSettingsNavCopy?, shell: MonitorShellCopy?) -> String {
        switch self {
        case .privateAI: nav?.privateAI ?? ""
        case .traces: shell?.tabTraces ?? ""
        }
    }
}

/// What a rail icon draws: an SF Symbol, or Traces' connected nodes.
enum EdgeRailIcon: Equatable {
    case symbol(String)
    /// Four nodes in a T (`GlassTraceNodesGlyph`, owner, 2026-10-22).
    case traceNodes
}

/// A borderless panel that floats over every app on every Space and never
/// becomes key: hovering and clicking the rail leaves the app in front in
/// front. Its frame is the one its controller places it at.
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

    /// Set only inside `place(_:)`.
    private var placing = false

    /// Moves and sizes the panel: the one way its frame changes.
    func place(_ frame: CGRect) {
        placing = true
        defer { placing = false }
        setFrame(frame, display: true)
    }

    // The panel keeps only the frames `place(_:)` gives it. The rail's
    // hosting view sizes its window to its SwiftUI content even with empty
    // `sizingOptions`, in the app as built against the macOS 26 SDK: every
    // time the rail opened, `NSHostingView.updateAnimatedWindowSize` called
    // `setFrame(_:display:)` from inside `windowDidLayout`, animating the
    // panel from the closed rail's size to the open rail's own (464 x 282
    // where the panel is 484 x 680), each resize asking for another layout
    // pass. Once the passes outnumbered the window's views, AppKit threw
    // and the app aborted (#1517). Dropped, those resizes change nothing
    // and ask for nothing.
    override func setFrame(_ frameRect: NSRect, display flag: Bool) {
        guard placing else { return }
        super.setFrame(frameRect, display: flag)
    }

    override func setFrame(_ frameRect: NSRect, display displayFlag: Bool, animate animateFlag: Bool) {
        guard placing else { return }
        super.setFrame(frameRect, display: displayFlag, animate: animateFlag)
    }

    override func setContentSize(_ size: NSSize) {
        guard placing else { return }
        super.setContentSize(size)
    }
}
