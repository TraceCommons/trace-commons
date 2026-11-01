import AppKit
import os
import SwiftUI

/// A fixed-label line in the unified log naming the window whose size just
/// changed, so the next report of the window-layout crash can be matched to
/// a window: the crash report itself shows only AppKit and SwiftUI frames.
///
/// The line carries the window's label and its new size in whole points,
/// nothing else: no title, no path, no content. It goes to the unified log
/// at the default level, which is kept on disk, and is read with
/// `log show --predicate 'subsystem == "<the app's bundle id>" AND category == "window"'`
/// around the crash's time. It is not in the `.ips` report.
enum WindowBreadcrumb {
    /// The app's windows, each by a fixed label.
    enum Scene: CaseIterable {
        case monitor, menuBarPanel, settings, firstRun, edgeRail

        var label: String {
            switch self {
            case .monitor: "monitor"
            case .menuBarPanel: "menu-bar panel"
            case .settings: "settings"
            case .firstRun: "first run"
            case .edgeRail: "edge rail"
            }
        }
    }

    /// The bundle id, so a staging app logs under its own subsystem
    /// (staging design, section 4.2). Outside a bundle, the process name.
    static let logger = Logger(
        subsystem: Bundle.main.bundleIdentifier ?? ProcessInfo.processInfo.processName,
        category: "window")

    /// Which resizes are logged: the first of a burst, then one more line if
    /// the burst runs past `burst` resizes inside `interval` seconds. A
    /// window dragged by hand logs once a second at most; the loop AppKit
    /// aborts on is hundreds of resizes in one turn, and logs twice.
    struct Throttle {
        static let interval: Double = 1
        static let burst = 32

        enum Line: Equatable { case resized, runaway }

        private var start: Double?
        private var count = 0

        mutating func note(at now: Double) -> Line? {
            if let start, now - start < Self.interval {
                count += 1
                return count == Self.burst ? .runaway : nil
            }
            start = now
            count = 1
            return .resized
        }
    }

    static func log(_ line: Throttle.Line, scene: Scene, size: CGSize) {
        let width = Int(size.width.rounded())
        let height = Int(size.height.rounded())
        switch line {
        case .resized:
            logger.notice("window resized: \(scene.label, privacy: .public) \(width)x\(height)")
        case .runaway:
            logger.error("window resizing repeatedly: \(scene.label, privacy: .public) \(width)x\(height)")
        }
    }
}

extension View {
    /// Logs this view's window by `scene`'s label when the window's size
    /// changes (`WindowBreadcrumb`).
    func windowBreadcrumb(_ scene: WindowBreadcrumb.Scene) -> some View {
        background(WindowBreadcrumbProbe(scene: scene).frame(width: 0, height: 0).accessibilityHidden(true))
    }
}

/// Watches its window's resize notification. Logging only: it never
/// changes a frame, so it can run inside AppKit's resize.
private struct WindowBreadcrumbProbe: NSViewRepresentable {
    let scene: WindowBreadcrumb.Scene

    func makeNSView(context: Context) -> NSView { Probe(scene: scene) }

    func updateNSView(_ view: NSView, context: Context) {}

    private final class Probe: NSView {
        private let scene: WindowBreadcrumb.Scene
        private var throttle = WindowBreadcrumb.Throttle()
        private var observer: WindowNotificationObservers?

        init(scene: WindowBreadcrumb.Scene) {
            self.scene = scene
            super.init(frame: .zero)
        }

        @available(*, unavailable)
        required init?(coder: NSCoder) { fatalError("init(coder:) is not used") }

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            observer = nil
            guard let window else { return }
            let token = NotificationCenter.default.addObserver(
                forName: NSWindow.didResizeNotification, object: window, queue: nil
            ) { [weak self] note in
                guard let window = note.object as? NSWindow else { return }
                MainActor.assumeIsolated { self?.noteResize(window.frame.size) }
            }
            observer = WindowNotificationObservers(center: .default, tokens: [token])
        }

        private func noteResize(_ size: CGSize) {
            if let line = throttle.note(at: ProcessInfo.processInfo.systemUptime) {
                WindowBreadcrumb.log(line, scene: scene, size: size)
            }
        }
    }
}
