import AppKit
import SwiftUI

/// The pieces of app behaviour that SwiftUI does not own.
///
/// There was no delegate here at all while the app was `LSUIElement`, and it
/// did not need one: a menu-bar-only app has no App menu, no Cmd-Tab entry,
/// no Dock menu and no reopen event, so the only way into anything was the
/// menu the app drew itself. Becoming a regular app opens all of those at
/// once, and each is a path into behaviour that previously had exactly one
/// entrance.
///
/// Three of those paths need answering, and they are the reason this type
/// exists rather than three separate accommodations:
///
/// - **Quit** now arrives from the App menu, Cmd-Q and the Dock icon's
///   context menu, none of which SwiftUI routes through the menu-bar item's
///   "Quit…" command. `applicationShouldTerminate` is the one funnel every
///   one of them passes through.
/// - **Reopen** (clicking the Dock icon with no window open) does nothing at
///   all without a delegate, which reads as a hang.
/// - **URL events** are delivered here, above any view. `onOpenURL` fires
///   only on a mounted view, and the app's resting state is running with no
///   window, so a view-level handler drops every link that arrives in the
///   state contributors are actually in.
@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    func applicationDidFinishLaunching(_ notification: Notification) {
        // Explicit rather than inherited. Removing LSUIElement already makes
        // this the default, but the default is invisible in the source: a
        // reader of this file cannot see the plist, and the app's shape is
        // too load-bearing to leave stated in only one place.
        NSApp.setActivationPolicy(.regular)

        // No window is opened here, and no attempt is made to detect a login
        // launch.
        //
        // The obvious heuristic does not work. A login item is started by
        // launchd, so `getppid() == 1` looks like it identifies one -- but
        // every GUI launch is reparented to launchd, including a Finder
        // double-click and `open`. Measured: launching this bundle with
        // `open` gives ppid 1 and is indistinguishable from a login start.
        // An earlier draft of this file used that test and hid the app on
        // every launch.
        //
        // SMAppService.mainApp offers no launch-hidden flag and no "you were
        // started at login" signal, so rather than guess, launch behaviour is
        // uniform: come up quietly, exactly as this app always has. That is
        // the correct answer at login -- the contributor agreed to "Start
        // Trace Commons when you log in?", which is a promise to be running,
        // not a request to be greeted -- and a recoverable one everywhere
        // else, because there is now a Dock icon, and clicking it opens the
        // window through applicationShouldHandleReopen below.
        //
        // Which is the point of the whole slice: what was missing was not a
        // window on launch, it was any reliable way to reach the app at all.
    }

    /// Every quit path, funnelled through the one confirmation.
    ///
    /// The alert is not decoration. This app *is* the daemon -- the watcher
    /// runs in-process -- so quitting it stops the thing the contributor
    /// installed it for, which is not what "close the window" means anywhere
    /// else on this platform.
    ///
    /// `runModal` is synchronous, so this answers immediately rather than
    /// going through `.terminateLater`. That matters for cleanup:
    /// `TraceCommonsAppMain` runs `model.shutdown()` off
    /// `willTerminateNotification`, and a `.terminateLater` design that never
    /// gets its reply leaves the daemon un-stopped.
    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        QuitConfirmation.granted() ? .terminateNow : .terminateCancel
    }

    /// Clicking the Dock icon with no window open. Without this the click is
    /// swallowed and the app appears wedged.
    func applicationShouldHandleReopen(
        _ sender: NSApplication,
        hasVisibleWindows: Bool
    ) -> Bool {
        if !hasVisibleWindows { OpenMainWindow.request() }
        return true
    }

    /// Invite links, delivered above the view layer.
    ///
    /// This deliberately does not enrol. It fills the field and brings the
    /// screen up; pressing the button stays a person's decision, because
    /// which commons to join is the question that screen exists to ask. The
    /// other two clients say the same thing at their own registration sites.
    ///
    /// The invite reaches `PendingInvite` and nothing else. It is a
    /// credential, so it is not logged, not put in a window title, and not
    /// echoed in an error.
    func application(_ application: NSApplication, open urls: [URL]) {
        for url in urls {
            guard let invite = DeepLink.inviteURL(from: url) else { continue }
            PendingInvite.shared.set(invite)
            NSApp.activate(ignoringOtherApps: true)
            OpenMainWindow.request()
            return
        }
    }
}

/// The one invite waiting for the Connect screen to come and get it.
///
/// A URL can arrive before the screen that consumes it exists -- at launch,
/// or with the app running and no window open -- so the value is parked here
/// rather than pushed at a view. Linux holds the same shape in a
/// `PENDING_INVITE` thread-local set by `set_pending_invite`
/// (`crates/trace-commons-contributor-gtk/src/ui/onboarding.rs`), and this
/// mirrors it rather than inventing a second pattern.
@MainActor
final class PendingInvite: ObservableObject {
    static let shared = PendingInvite()

    /// Published so a Connect screen that is *already* on show notices, which
    /// `onAppear` alone would miss.
    @Published private(set) var value: String?

    private init() {}

    func set(_ invite: String) {
        value = invite
    }

    /// Reads and clears. Taking rather than peeking is what stops the same
    /// invite being re-applied over whatever the contributor has since typed.
    func take() -> String? {
        defer { value = nil }
        return value
    }
}

/// The quit confirmation, shared by every path that can terminate the app.
enum QuitConfirmation {
    /// Shows the alert and answers whether to proceed.
    ///
    /// The copy is unchanged from when it lived in the menu-bar item: it was
    /// written specifically because the watcher stops with the app, and
    /// nothing about gaining a Dock icon makes that less true.
    @MainActor
    static func granted() -> Bool {
        NSApp.activate(ignoringOtherApps: true)
        let alert = NSAlert()
        alert.messageText = "Quit Trace Commons?"
        alert.informativeText = """
        The watcher runs inside this app, so quitting stops it. Nothing will be \
        noticed or sent while it is closed.

        Sessions already waiting stay on this machine and will be here when you \
        come back. Nothing is sent while nobody's approving.
        """
        alert.addButton(withTitle: "Quit")
        alert.addButton(withTitle: "Keep running")
        return alert.runModal() == .alertFirstButtonReturn
    }
}
