import AppKit
import SwiftUI

/// A continuous animation that runs at its own rate only while its window
/// is focused, and not at all while it cannot be seen.
///
/// A `TimelineView` on the `.animation` schedule asks for a new frame on
/// every display refresh it is allowed, whether or not its window is on
/// screen. Each frame is a SwiftUI graph update on the main thread. This
/// wrapper applies the window's presence (`glassWindowPresence`, kept by
/// `glassAnimationVisibility()` at the window's root) to the caller's own
/// schedule (`GlassAnimationGate.schedule`):
/// - focused: the caller's rate;
/// - seen but not focused: at most 15 frames a second (owner, 2026-11-20);
/// - fully covered, minimised, on another Space or ordered out, or any
///   window of a hidden app: holds its last frame and asks for no more.
///
/// Every continuous animation in the app draws through this one clock
/// (`AnimationFocusTests` checks the sources), so the rule holds for all
/// of them without code of their own.
///
/// The animations in this app take their phase from the absolute time
/// (`date`), not from a count of frames, so a paused one picks up where
/// it would have been, and nothing it draws while it can be seen changes.
public struct GlassAnimationTimeline<Content: View>: View {
    private let minimumInterval: Double?
    private let paused: Bool
    private let content: (Date) -> Content
    @Environment(\.glassWindowPresence) private var presence

    /// `minimumInterval` nil runs at the display's rate.
    public init(minimumInterval: Double? = nil, paused: Bool = false, @ViewBuilder content: @escaping (Date) -> Content) {
        self.minimumInterval = minimumInterval
        self.paused = paused
        self.content = content
    }

    public var body: some View {
        let schedule = GlassAnimationGate.schedule(minimumInterval: minimumInterval, paused: paused, presence: presence)
        TimelineView(.animation(minimumInterval: schedule.minimumInterval, paused: schedule.paused)) { timeline in
            content(timeline.date)
        }
    }
}

/// Where a view's window stands, for its animations.
public enum GlassWindowPresence: Equatable, Sendable {
    /// The key window, or the main window of the active app.
    case focused
    /// At least partly on screen, but not focused.
    case unfocused
    /// Cannot be seen at all.
    case hidden
}

/// What a `TimelineView` is asked to do: its minimum interval between
/// frames (nil: the display's rate) and whether it holds still.
public struct GlassAnimationSchedule: Equatable, Sendable {
    public let minimumInterval: Double?
    public let paused: Bool

    public init(minimumInterval: Double?, paused: Bool) {
        self.minimumInterval = minimumInterval
        self.paused = paused
    }
}

extension EnvironmentValues {
    /// This view's window's presence. Focused unless a root above it has
    /// `glassAnimationVisibility()` and its window is not: a view with no
    /// such root (a preview, a test, an `ImageRenderer` image) animates
    /// as it always did.
    @Entry public var glassWindowPresence: GlassWindowPresence = .focused
}

extension View {
    /// Keeps `glassWindowPresence` for everything under this view with
    /// its window's presence. Applied once at each window's root.
    /// The probe is an AppKit view, so it belongs on a root in a real
    /// window, never inside a component (`ImageRenderer` draws an AppKit
    /// view as a placeholder).
    public func glassAnimationVisibility() -> some View {
        modifier(GlassAnimationVisibility())
    }
}

private struct GlassAnimationVisibility: ViewModifier {
    @State private var presence = GlassWindowPresence.focused

    func body(content: Content) -> some View {
        content
            .environment(\.glassWindowPresence, presence)
            .background(
                GlassWindowVisibilityProbe { presence = $0 }
                    .frame(width: 0, height: 0)
                    .allowsHitTesting(false)
                    .accessibilityHidden(true))
    }
}

/// The decisions behind `GlassAnimationTimeline`, as plain logic.
public enum GlassAnimationGate {
    /// The longest a seen but unfocused window's animations go between
    /// frames is no shorter than this: 15 frames a second.
    public static let unfocusedInterval: Double = 1.0 / 15

    /// The schedule an animation runs on: the caller's own while its
    /// window is focused; at most 15 frames a second while the window can
    /// be seen but is not focused (a slower request is kept, never sped
    /// up); stopped while it cannot be seen. It keeps no history, so the
    /// first frame after focus returns is at the caller's rate.
    public static func schedule(minimumInterval: Double?, paused: Bool, presence: GlassWindowPresence) -> GlassAnimationSchedule {
        switch presence {
        case .focused:
            GlassAnimationSchedule(minimumInterval: minimumInterval, paused: paused)
        case .unfocused:
            GlassAnimationSchedule(minimumInterval: max(minimumInterval ?? 0, unfocusedInterval), paused: paused)
        case .hidden:
            GlassAnimationSchedule(minimumInterval: minimumInterval, paused: true)
        }
    }

    /// A window's presence from its state. It cannot be seen: hidden.
    /// Otherwise it is focused when it is the key window (a panel can be
    /// key without activating the app), or the main window of the active
    /// app (its own sheet is key then, and draws with its environment).
    public static func presence(visible: Bool, isKey: Bool, isMain: Bool, appActive: Bool) -> GlassWindowPresence {
        guard visible else { return .hidden }
        return isKey || (isMain && appActive) ? .focused : .unfocused
    }

    /// The same, read from a window.
    @MainActor
    public static func presence(_ window: NSWindow?) -> GlassWindowPresence {
        presence(
            visible: isVisible(window),
            isKey: window?.isKeyWindow ?? false,
            isMain: window?.isMainWindow ?? false,
            appActive: NSApplication.shared.isActive)
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

/// A zero-size view that reports its window's presence, now and whenever
/// that changes.
struct GlassWindowVisibilityProbe: NSViewRepresentable {
    let onChange: @MainActor (GlassWindowPresence) -> Void

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
        var onChange: (@MainActor (GlassWindowPresence) -> Void)?
        private var observers: [NSObjectProtocol] = []
        /// The last value reported, so an unchanged one is not reported
        /// again (each report is a SwiftUI state write).
        private(set) var reported: GlassWindowPresence?

        /// The window's own changes that can move its presence: seen or
        /// not, and focus arriving or leaving.
        static let windowNotifications: [Notification.Name] = [
            NSWindow.didChangeOcclusionStateNotification,
            NSWindow.didMiniaturizeNotification,
            NSWindow.didDeminiaturizeNotification,
            NSWindow.didBecomeKeyNotification,
            NSWindow.didResignKeyNotification,
            NSWindow.didBecomeMainNotification,
            NSWindow.didResignMainNotification,
        ]
        /// The app's: hidden or shown, active or not.
        static let appNotifications: [Notification.Name] = [
            NSApplication.didHideNotification,
            NSApplication.didUnhideNotification,
            NSApplication.didBecomeActiveNotification,
            NSApplication.didResignActiveNotification,
        ]

        override var isFlipped: Bool { true }
        override func hitTest(_ point: NSPoint) -> NSView? { nil }

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            stopObserving()
            if let window {
                let centre = NotificationCenter.default
                // A window brought forward is also asked again whether it
                // can be seen, in case an occlusion change was not posted.
                let names: [(Notification.Name, AnyObject?)] =
                    Self.windowNotifications.map { ($0, window) }
                    + Self.appNotifications.map { ($0, NSApplication.shared) }
                for (name, object) in names {
                    observers.append(centre.addObserver(forName: name, object: object, queue: .main) { [weak self] _ in
                        MainActor.assumeIsolated { self?.refresh() }
                    })
                }
            }
            refresh()
        }

        func refresh() {
            let now = GlassAnimationGate.presence(window)
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
