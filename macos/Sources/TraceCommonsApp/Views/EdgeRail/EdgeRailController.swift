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
    private var compute: ComputeModel?
    private var screenObserver: NSObjectProtocol?

    /// Hands the rail the app's models, and shows it if the switch is on.
    func attach(model: AppModel, compute: ComputeModel) {
        self.model = model
        self.compute = compute
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
        guard panel == nil, let model, let compute else { return }
        let panel = EdgeRailPanel()
        let root = EdgeRailView(state: state, onOpenChange: { [weak self] open in self?.place(open: open) })
            .environmentObject(model)
            .environment(compute)
            .tint(GlassTokens.Color.purpleSoft.color)
        panel.contentView = NSHostingView(rootView: root)
        self.panel = panel
        state.open = false
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
        state.open = false
    }

    /// Moves the panel to the main display's right edge, at the size the
    /// rail's state needs.
    private func place(open: Bool) {
        guard let panel, let screen = NSScreen.screens.first else { return }
        let frame = EdgeRailGeometry.frame(open: open, visible: screen.visibleFrame)
        panel.setFrame(frame, display: true)
    }
}

/// What the rail shows: open or closed, and which icon's peek.
@Observable
@MainActor
final class EdgeRailState {
    var open = false
    var peek: EdgeRailPeek = .waiting
}

/// The rail's icons, top to bottom.
enum EdgeRailPeek: CaseIterable, Identifiable {
    case waiting, tools, balance, privateAI, compute, privacy

    var id: Self { self }

    var symbol: String {
        switch self {
        case .waiting: "tray"
        case .tools: "terminal"
        case .balance: "creditcard"
        case .privateAI: "key"
        case .compute: "cpu"
        case .privacy: "lock"
        }
    }

    /// The icon's name: Settings' name for a section Settings has, the
    /// rail's own for the rest.
    func name(_ rail: MonitorEdgeRailCopy?, nav: MonitorSettingsNavCopy?) -> String {
        switch self {
        case .waiting: rail?.waiting ?? ""
        case .tools: nav?.tools ?? ""
        case .balance: rail?.balance ?? ""
        case .privateAI: nav?.privateAI ?? ""
        case .compute: nav?.compute ?? ""
        case .privacy: rail?.privacy ?? ""
        }
    }

    /// Where the peek's link goes in the app.
    var destination: MonitorDestination {
        switch self {
        case .waiting: .traces(entryId: nil)
        case .tools: .settings(.tools)
        case .balance, .privateAI: .inference
        case .compute: .settings(.compute)
        case .privacy: .settings(.watching)
        }
    }
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
