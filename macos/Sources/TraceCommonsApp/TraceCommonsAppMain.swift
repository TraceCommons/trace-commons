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

    init() {
        // Earlier builds wrote the preview sheet's recent-search list to
        // UserDefaults. That list is the contributor's own record of what
        // they were checking for -- client names, employers, unreleased
        // products -- so it is removed here rather than only when a preview
        // sheet next happens to open. An upgraded install where nobody opens
        // a preview again would otherwise keep those terms forever, with no
        // surface left in the app that could clear them.
        RecentSearches.purgeLegacyStore()
    }

    var body: some Scene {
        MenuBarExtra {
            MenuBarContent()
                .environmentObject(model)
                .tint(TC.green)
        } label: {
            Launcher(model: model)
        }

        Window("Trace Commons", id: WindowID.main) {
            MainWindowView()
                .environmentObject(model)
                .frame(minWidth: 760, minHeight: 520)
                // The community site's primary is green, not the platform
                // blue. Overriding the user's chosen accent colour is a real
                // departure from macOS convention and it is made on purpose:
                // this app and `community/public` are one product, the
                // accent is the single strongest cue that they are, and the
                // green carries a meaning here (good standing) that the
                // system blue does not. Everything else about the controls
                // -- shape, focus ring, keyboard behaviour -- stays stock.
                .tint(TC.green)
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

        // Development hook, same family as TRACE_COMMONS_SHOW_WINDOW below:
        // pins the app to one appearance so a capture run can produce light
        // and dark images without touching the machine's system setting.
        // Unset (the normal case) leaves the app following the system, which
        // is the only correct behaviour for a shipping build.
        switch ProcessInfo.processInfo.environment["TRACE_COMMONS_APPEARANCE"] {
        case "dark": NSApp.appearance = NSAppearance(named: .darkAqua)
        case "light": NSApp.appearance = NSAppearance(named: .aqua)
        default: break
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
