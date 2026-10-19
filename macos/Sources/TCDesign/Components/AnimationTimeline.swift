import AppKit
import SwiftUI

/// A continuous animation that runs only while it can be seen.
///
/// A `TimelineView` on the `.animation` schedule asks for a new frame on
/// every display refresh it is allowed, whether or not its window is on
/// screen. Each frame is a SwiftUI graph update on the main thread. This
/// wrapper adds one condition to the caller's own `paused`: the window it
/// is in must be at least partly visible (`glassAnimationsVisible`, kept by
/// `glassAnimationVisibility()` at the window's root). A window that is
/// fully covered, minimised, on another Space or ordered out, or any window
/// of a hidden app, holds its last frame and asks for no more.
///
/// The animations in this app take their phase from the absolute time
/// (`date`), not from a count of frames, so a paused one picks up where
/// it would have been, and nothing it draws while it can be seen changes.
public struct GlassAnimationTimeline<Content: View>: View {
    private let minimumInterval: Double?
    private let paused: Bool
    private let content: (Date) -> Content
    @Environment(\.glassAnimationsVisible) private var visible

    /// `minimumInterval` nil runs at the display's rate.
    public init(minimumInterval: Double? = nil, paused: Bool = false, @ViewBuilder content: @escaping (Date) -> Content) {
        self.minimumInterval = minimumInterval
        self.paused = paused
        self.content = content
    }

    public var body: some View {
        TimelineView(.animation(minimumInterval: minimumInterval, paused: GlassAnimationGate.paused(paused, visible: visible))) { timeline in
            content(timeline.date)
        }
    }
}

extension EnvironmentValues {
    /// Whether this view's window can be seen. True unless a root above it
    /// has `glassAnimationVisibility()` and its window cannot be: a view
    /// with no such root (a preview, a test, an `ImageRenderer` image)
    /// animates as it always did.
    @Entry public var glassAnimationsVisible: Bool = true
}

extension View {
    /// Keeps `glassAnimationsVisible` for everything under this view with
    /// whether its window can be seen. Applied once at each window's root.
    /// The probe is an AppKit view, so it belongs on a root in a real
    /// window, never inside a component (`ImageRenderer` draws an AppKit
    /// view as a placeholder).
    public func glassAnimationVisibility() -> some View {
        modifier(GlassAnimationVisibility())
    }
}

private struct GlassAnimationVisibility: ViewModifier {
    @State private var visible = true

    func body(content: Content) -> some View {
        content
            .environment(\.glassAnimationsVisible, visible)
            .background(
                GlassWindowVisibilityProbe { visible = $0 }
                    .frame(width: 0, height: 0)
                    .allowsHitTesting(false)
                    .accessibilityHidden(true))
    }
}

/// The decisions behind `GlassAnimationTimeline`, as plain logic.
public enum GlassAnimationGate {
    /// Whether an animation that its caller would run (`paused` false)
    /// holds still: it does whenever it cannot be seen.
    public static func paused(_ paused: Bool, visible: Bool) -> Bool {
        paused || !visible
    }

    /// Whether a view can be seen, from its window's state. A view in no
    /// window cannot be; a window AppKit reports as occluded (fully
    /// covered, minimised, on another Space, ordered out) cannot be; nor
    /// can any window of a hidden app.
    public static func isVisible(inWindow: Bool, occlusionVisible: Bool, miniaturized: Bool, appHidden: Bool) -> Bool {
        inWindow && occlusionVisible && !miniaturized && !appHidden
    }

    /// The same, read from a window.
    @MainActor
    public static func isVisible(_ window: NSWindow?) -> Bool {
        guard let window else { return false }
        return isVisible(
            inWindow: true,
            occlusionVisible: window.occlusionState.contains(.visible),
            miniaturized: window.isMiniaturized,
            appHidden: NSApplication.shared.isHidden)
    }
}

/// A zero-size view that reports whether its window can be seen, now and
/// whenever that changes.
struct GlassWindowVisibilityProbe: NSViewRepresentable {
    let onChange: @MainActor (Bool) -> Void

    func makeNSView(context: Context) -> ProbeView {
        let view = ProbeView()
        view.onChange = onChange
        return view
    }

    func updateNSView(_ view: ProbeView, context: Context) {
        view.onChange = onChange
    }

    static func dismantleNSView(_ view: ProbeView, coordinator: ()) {
        view.stopObserving()
    }

    final class ProbeView: NSView {
        var onChange: (@MainActor (Bool) -> Void)?
        private var observers: [NSObjectProtocol] = []
        /// The last value reported, so an unchanged one is not reported
        /// again (each report is a SwiftUI state write).
        private(set) var reported: Bool?

        override var isFlipped: Bool { true }
        override func hitTest(_ point: NSPoint) -> NSView? { nil }

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            stopObserving()
            if let window {
                let centre = NotificationCenter.default
                let names: [(Notification.Name, AnyObject?)] = [
                    (NSWindow.didChangeOcclusionStateNotification, window),
                    (NSWindow.didMiniaturizeNotification, window),
                    (NSWindow.didDeminiaturizeNotification, window),
                    // A window brought forward is visible: asked again in
                    // case an occlusion change was not posted.
                    (NSWindow.didBecomeKeyNotification, window),
                    (NSWindow.didBecomeMainNotification, window),
                    (NSApplication.didHideNotification, NSApplication.shared),
                    (NSApplication.didUnhideNotification, NSApplication.shared),
                    (NSApplication.didBecomeActiveNotification, NSApplication.shared),
                ]
                for (name, object) in names {
                    observers.append(centre.addObserver(forName: name, object: object, queue: .main) { [weak self] _ in
                        MainActor.assumeIsolated { self?.refresh() }
                    })
                }
            }
            refresh()
        }

        func refresh() {
            let now = GlassAnimationGate.isVisible(window)
            guard now != reported else { return }
            reported = now
            // Reported after the current update: AppKit can move a view
            // between windows while SwiftUI is updating, and a state write
            // then is not allowed.
            let onChange = onChange
            DispatchQueue.main.async { onChange?(now) }
        }

        func stopObserving() {
            for observer in observers { NotificationCenter.default.removeObserver(observer) }
            observers.removeAll()
        }
    }
}
