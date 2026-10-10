import AppKit
import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

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
    var compute: ComputeModel?
    /// Read at quit time for one sentence, and for nothing else.
    ///
    /// With the listener inside this process, quitting stops answering
    /// model calls as well as stopping the watcher, and the confirmation's
    /// existing sentence does not cover that. The extra sentence is the
    /// Rust's, not this file's -- the rest of that dialog is Swift-authored
    /// and this line deliberately is not.
    var model: AppModel?
    private let quitCoordinator = QuitCoordinator()
    func applicationDidFinishLaunching(_ notification: Notification) {
        // Explicit rather than inherited. Removing LSUIElement already makes
        // this the default, but the default is invisible in the source: a
        // reader of this file cannot see the plist, and the app's shape is
        // too load-bearing to leave stated in only one place.
        NSApp.setActivationPolicy(.regular)
        // The person's Light, Dark or System choice (Settings > General).
        GlassAppearance.applyStored()

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
    /// Confirmation is synchronous; compute stop runs on its background queue.
    /// Every pending request gets a reply, including the outer deadline, which
    /// keeps the app running if worker shutdown has not returned safe evidence.
    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        let confirmed =
            quitCoordinator.isStopping
            || QuitConfirmation.granted(
                prompt: model?.quitPrompt
                    ?? QuitPrompt.decode(fromJSON: TCCoreCopy.quitPromptWithoutWatcherJSON()),
                computeDetail: compute?.copy?.quitDetail,
                privateInferenceDetail: model?.privateInferenceQuitDetail
            )
        let decision = quitCoordinator.request(confirmed: confirmed, deadlineSeconds: 17, stop: { [weak self] in
            guard let compute = self?.compute else { return true }
            return await compute.shutdown(timeoutMilliseconds: 15_000)
        }, reply: { [weak self] stopped in
            if !stopped {
                self?.compute?.noteQuitRefused()
                OpenMonitor.request(.settings(.compute))
            }
            sender.reply(toApplicationShouldTerminate: stopped)
        })
        return decision == .later ? .terminateLater : .terminateCancel
    }

    /// Clicking the Dock icon with no window open. Without this the click is
    /// swallowed and the app appears wedged.
    func applicationShouldHandleReopen(
        _ sender: NSApplication,
        hasVisibleWindows: Bool
    ) -> Bool {
        if !hasVisibleWindows { OpenMonitor.request() }
        return true
    }

    /// Invite links, delivered above the view layer.
    ///
    /// This deliberately does not enroll. It fills the field and brings the
    /// screen up; pressing the button stays a person's decision, because
    /// which commons to join is the question that screen exists to ask. The
    /// other two clients say the same thing at their own registration sites.
    ///
    /// The invite reaches `PendingInvite` and nothing else. It is a
    /// credential, so it is not logged, not put in a window title, and not
    /// echoed in an error.
    ///
    /// D-14 (default taken): a link that arrives while already onboarded
    /// only opens the Monitor. The request carries no destination; while
    /// onboarding is required it routes to the first-run window, where the
    /// Connect screen picks the invite up.
    func application(_ application: NSApplication, open urls: [URL]) {
        for url in urls {
            guard let invite = DeepLink.inviteURL(from: url) else { continue }
            PendingInvite.shared.set(invite)
            NSApp.activate(ignoringOtherApps: true)
            OpenMonitor.request()
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
    /// The heading, the body and both buttons are the core's
    /// (`quit_copy::quit_prompt`), chosen for whether this process hosts the
    /// watcher or is attached to one: the hosting sentence this alert used to
    /// hard-code is false for an attached app, whose watcher keeps sending.
    /// Without a prompt the quit is not confirmed, because it is not
    /// confirmable before the true sentence for this process is shown.
    @MainActor
    static func granted(
        prompt: QuitPrompt?,
        computeDetail: String? = nil,
        privateInferenceDetail: String? = nil
    ) -> Bool {
        guard let prompt else { return false }
        NSApp.activate(ignoringOtherApps: true)
        let alert = NSAlert()
        alert.messageText = prompt.title
        alert.informativeText = prompt.body
        if let computeDetail { alert.informativeText += "\n\n" + computeDetail }
        // Appended only when the switch is on. A contributor who never
        // turned it on should not be warned about losing it.
        if let privateInferenceDetail {
            alert.informativeText += "\n\n" + privateInferenceDetail
        }
        alert.addButton(withTitle: prompt.confirm)
        alert.addButton(withTitle: prompt.cancel)
        return alert.runModal() == .alertFirstButtonReturn
    }
}
