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
/// These paths need answering here rather than in individual views:
///
/// - **Initial activation** opens the window on a foreground launch, while
///   background launches stay quiet until the person activates the app.
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
    /// Read at quit time for one fact: whether quitting stops Private AI.
    var model: AppModel?
    private let quitCoordinator = QuitCoordinator()
    private let terminationSignal = TerminationSignal()
    /// Set by SIGTERM just before it asks to terminate, and consumed by the
    /// next `applicationShouldTerminate`. Whoever sent the signal has
    /// already decided, so there is no one to ask.
    private var quitRequestedBySignal = false
    private var openedForInitialActivation = false
    func applicationDidFinishLaunching(_ notification: Notification) {
        // SIGTERM takes the same path as Quit, minus the confirmation:
        // compute stop, then willTerminate's daemon stop. See
        // `TerminationSignal`.
        terminationSignal.install { [weak self] in
            self?.quitRequestedBySignal = true
            NSApp.terminate(nil)
        }

        // The launch's open-application event is current here (not yet in
        // willFinishLaunching): a login-item launch says so in it, and
        // stays in the menu bar (`LaunchRouting.launchKind`).
        let event = NSAppleEventManager.shared().currentAppleEvent
        LaunchRouting.launchedAsLoginItem = LaunchRouting.isLoginItemLaunch(
            eventClass: event?.eventClass, eventID: event?.eventID,
            propData: event?.paramDescriptor(forKeyword: LaunchRouting.keyAEPropDataCode)?.enumCodeValue)

        // Explicit rather than inherited. Removing LSUIElement already makes
        // this the default, but the default is invisible in the source: a
        // reader of this file cannot see the plist, and the app's shape is
        // too load-bearing to leave stated in only one place.
        NSApp.setActivationPolicy(.regular)
        // The person's Light, Dark or System choice (Settings > General).
        GlassAppearance.applyStored()

        // AppKit can activate the app before didFinishLaunching, or send
        // didBecomeActive afterwards. Both paths open once. A background
        // launch (including open -g) waits until the person activates it.
        if NSApp.isActive { openForInitialActivation() }
    }

    func applicationDidBecomeActive(_ notification: Notification) {
        openForInitialActivation()
    }

    private func openForInitialActivation() {
        guard !openedForInitialActivation else { return }
        openedForInitialActivation = true
        OpenMonitor.requestForInitialActivation()
    }

    /// Every quit path, funnelled through the one confirmation.
    ///
    /// It asks only when quitting would stop compute or Private AI, in one
    /// sentence saying which; any other quit proceeds at once (owner,
    /// 2026-11-12). The watcher stopping is not a reason to ask.
    ///
    /// A quit asked for by SIGTERM skips the confirmation and nothing else:
    /// a compute stop that does not confirm still keeps the app running.
    ///
    /// Confirmation is synchronous; compute stop runs on its background queue.
    /// Every pending request gets a reply, including the outer deadline, which
    /// keeps the app running if worker shutdown has not returned safe evidence.
    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        let bySignal = quitRequestedBySignal
        quitRequestedBySignal = false
        let confirmed =
            bySignal
            || quitCoordinator.isStopping
            || QuitConfirmation.granted(
                stopsCompute: QuitPrompt.stopsCompute(compute?.snapshot),
                stopsPrivateAI: model?.privateInferenceQuitStops ?? false
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
            guard let invite = DeepLink.inviteURL(
                from: url, scheme: BuildChannelGate.agreedChannel?.urlScheme)
            else { continue }
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
    /// Answers whether to proceed, asking first only when quitting would
    /// stop something live.
    ///
    /// The heading, the one-sentence body and both buttons are the core's
    /// (`quit_copy::quit_prompt`). A prompt that cannot be read while
    /// something is live refuses the quit.
    @MainActor
    static func granted(stopsCompute: Bool, stopsPrivateAI: Bool) -> Bool {
        let prompt: QuitPrompt
        switch QuitPrompt.decision(
            stopsCompute: stopsCompute, stopsPrivateAI: stopsPrivateAI,
            promptJSON: TCCoreCopy.quitReasonPromptJSON
        ) {
        case .quitNow: return true
        case .refuse: return false
        case .ask(let asked): prompt = asked
        }
        NSApp.activate(ignoringOtherApps: true)
        let alert = NSAlert()
        alert.messageText = prompt.title
        alert.informativeText = prompt.body
        alert.addButton(withTitle: prompt.confirm)
        alert.addButton(withTitle: prompt.cancel)
        return alert.runModal() == .alertFirstButtonReturn
    }
}
