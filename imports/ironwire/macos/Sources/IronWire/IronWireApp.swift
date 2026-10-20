//! The app.
//
// A `MenuBarExtra` and nothing else — no window, no dock icon (`LSUIElement` in
// `Resources/Info.plist`). The whole program is: poll the daemon, draw what it
// said, and post a pin when the user picks one.
//
// `.window` rather than `.menu` for the dropdown style, for two reasons that are
// not cosmetic. A capacity bar has to be *absent* for an unobserved backend, and
// a real menu cannot express that as clearly as a view can. And `.window` gives
// `onAppear`/`onDisappear` on the content, which is the signal the poll rate
// switches on — `/_ironwire/status` costs a credential check per backend, so
// polling every second in the background would not be free.

import AppKit
import IronWireKit
import SwiftUI

@main
struct IronWireApp: App {
    // Created once, when the scene is first evaluated — which is at launch,
    // because the label has to be drawn before anyone has clicked anything.
    @StateObject private var model = AppModel()

    var body: some Scene {
        MenuBarExtra {
            MenuContent(client: model.client, notifier: model.notifier, loginItem: model.loginItem)
        } label: {
            MenuBarLabel(client: model.client)
        }
        .menuBarExtraStyle(.window)
    }
}

/// The icon, redrawn whenever the status changes.
private struct MenuBarLabel: View {
    @ObservedObject var client: ControlClient

    var body: some View {
        Image(nsImage: MenuBarIcon.image(for: IconState.from(client.status)))
            // The attention icon carries its own colour; without this SwiftUI
            // would flatten it back to a template and the dot would vanish.
            .renderingMode(.original)
    }
}

/// Owns the client, and starts it.
///
/// Startup hangs off the scene's own lifetime rather than an
/// `NSApplicationDelegate`, because the label has to be drawn for there to be an
/// app at all — so this cannot silently fail to run. Polling that only began
/// when somebody opened the menu would have missed the point of the app: the
/// events worth announcing are the ones nobody is looking for.
@MainActor
final class AppModel: ObservableObject {
    let client = ControlClient()
    let notifier = Notifier()
    let loginItem = LoginItem()

    init() {
        client.onEvent = { [weak self] event in
            self?.notifier.deliver(event)
        }
        client.start()

        // Quitting must not leave a task polling somebody's daemon. Never
        // unregistered, because this object lives exactly as long as the process
        // does — there is no point at which removing it would be right.
        NotificationCenter.default.addObserver(
            forName: NSApplication.willTerminateNotification, object: nil, queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.client.stop() }
        }
    }
}
