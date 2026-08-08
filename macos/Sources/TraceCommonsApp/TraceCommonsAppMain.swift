import AppKit
import SwiftUI

/// The macOS contributor shell.
///
/// One application bundle, no second binary: the app links the C ABI and
/// hosts the watch/upload/digest loops in-process (see the macOS design
/// spec). `LSUIElement` is set in the bundle's Info.plist, so there is a
/// menu-bar item and no Dock icon.
@main
struct TraceCommonsShell: App {
    @StateObject private var model = AppModel()

    var body: some Scene {
        MenuBarExtra {
            MenuBarContent()
                .environmentObject(model)
        } label: {
            Launcher(model: model)
        }

        Window("Trace Commons", id: WindowID.main) {
            MainWindowView()
                .environmentObject(model)
                .frame(minWidth: 760, minHeight: 520)
        }
        .defaultSize(width: 940, height: 660)
    }
}

/// The menu-bar label, plus the one-time launch work. It lives in a view
/// rather than in `App` so it can reach `openWindow`, which the notification
/// `Review` action and the queue-full banner both need.
private struct Launcher: View {
    @ObservedObject var model: AppModel
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        MenuBarLabel(model: model)
            .task { launch() }
    }

    @MainActor
    private func launch() {
        OpenMainWindow.handler = {
            NSApp.activate(ignoringOtherApps: true)
            openWindow(id: WindowID.main)
        }
        model.start()
        Notifier.shared.configure()
        // The only thing a notification action may do is open this window.
        Notifier.shared.onReview = { OpenMainWindow.request() }

        NotificationCenter.default.addObserver(
            forName: NSApplication.willTerminateNotification,
            object: nil,
            queue: .main
        ) { _ in
            MainActor.assumeIsolated { model.shutdown() }
        }

        // Used by scripts/run-demo.sh to bring the window up for a
        // screenshot. A menu-bar app otherwise shows nothing until asked.
        if ProcessInfo.processInfo.environment["TRACE_COMMONS_SHOW_WINDOW"] == "1" {
            DispatchQueue.main.asyncAfter(deadline: .now() + 1.0) {
                OpenMainWindow.request()
            }
        }
        DebugScreenshot.scheduleIfRequested(model: model)
        SelfTest.runIfRequested(model: model)
    }
}

enum WindowID {
    static let main = "trace-commons-main"
}

/// Opening the window from outside a SwiftUI view (a notification action)
/// needs a hook that is not `@Environment(\.openWindow)`.
enum OpenMainWindow {
    @MainActor static var handler: (() -> Void)?

    @MainActor
    static func request() {
        handler?()
    }
}
