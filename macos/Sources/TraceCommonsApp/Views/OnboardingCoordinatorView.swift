import AppKit
import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// Hosts Ron's first run (#1030): Quick setup (Join, Folders, Uses) and
/// Custom setup (Join, Tools, Rules, Uses), one screen per step.
///
/// Every decision is in `FirstRunState`, every daemon call in
/// `FirstRunPlan`, run in order by one `FirstRunRunner` this view holds for
/// the whole first run; the screens draw the state and commit it. This view
/// owns only what spans the steps:
///
/// - the copy table, read once from the core (`tc_first_run_copy_json`). A
///   table that will not decode shows no step at all: every word a step
///   shows is the core's, and none is written here to fall back on. That
///   is logged, label only, so a broken table is diagnosable;
/// - the passkey sheets, mounted here exactly once, so a request the
///   Folders or Tools commit raises presents on whichever step follows;
/// - invite links (`PendingInvite`), which fill Join and bring it up, in a
///   host that takes them (`takesInvites`).
///
/// The daemon is not running on a fresh install: the core refuses to start
/// it until the session roots are declared, and Folders or Tools is where
/// they are. So the invite pasted on Join is looked up and joined, and the
/// account chosen there signed in or created, only once that commit has
/// started the daemon (`CommitPoint.leaveRoots`).
///
/// ## Resuming
///
/// A host starts the first run at `startAt` (Join, unless it names another
/// step). What an earlier first run left on the daemon is recorded
/// (`OnboardingNavigation.initialState`): a running daemon is not started
/// again, and an enrollment is the account (`recordEnrolment`), so its
/// invite is not joined again, Join reads Continue, and Automatic is
/// offered. The daemon's first status can arrive after this view is up, so
/// the enrollment is recorded then too. Start's marker (the tenant's, or
/// watching only's) is what tells "enrolled" from "set up".
struct OnboardingCoordinatorView: View {
    @EnvironmentObject private var model: AppModel
    var startAt: Step
    /// Told the step each time it changes, and once on appearing.
    var onStep: ((Step) -> Void)?
    /// Called once Start has finished the first run. The caller decides
    /// what replaces this view.
    var onComplete: () -> Void
    /// Whether this host takes invite links. One whose runner does not last
    /// the whole first run leaves them parked for one that does.
    var takesInvites: Bool

    typealias Step = OnboardingNavigation.Step

    @State private var copy = TCCoreCopy.firstRunCopyJSON().flatMap(FirstRunCopy.decode)
    /// Made on first appearing, since it runs its calls through the
    /// environment's `AppModel`, and kept for the whole first run.
    @State private var runner: FirstRunRunner?

    init(
        startAt: Step = .join, onStep: ((Step) -> Void)? = nil, takesInvites: Bool = true,
        onComplete: @escaping () -> Void
    ) {
        self.startAt = startAt
        self.onStep = onStep
        self.takesInvites = takesInvites
        self.onComplete = onComplete
    }

    var body: some View {
        Group {
            if let copy, let runner {
                FirstRunSteps(
                    copy: copy, runner: runner, onStep: onStep, takesInvites: takesInvites,
                    onComplete: onComplete)
            } else {
                GlassSpinner(standalone: true)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .onAppear {
            if copy == nil { NSLog("trace-commons: first-run-copy-undecodable") }
            guard runner == nil else { return }
            let uses = copy?.uses
            runner = FirstRunRunner(
                state: OnboardingNavigation.initialState(
                    startAt: startAt, daemonRunning: model.startup == .running, enrolled: model.status.loggedIn),
                daemon: model,
                finishedNotice: { refusal in uses.flatMap { UsesScreenLayout.finishedNotice(refusal, uses: $0) } })
        }
    }
}

/// The current step's screen, and what spans the steps.
private struct FirstRunSteps: View {
    @EnvironmentObject private var model: AppModel
    let copy: FirstRunCopy
    @ObservedObject var runner: FirstRunRunner
    let onStep: ((OnboardingNavigation.Step) -> Void)?
    let takesInvites: Bool
    let onComplete: () -> Void

    @ObservedObject private var pendingInvite = PendingInvite.shared
    /// The account the passkey sheets complete with: the running daemon's.
    /// Nil until it runs, which Join reads as "record the choice for later".
    @State private var passkeyAccount: LivePasskeyAccount?
    @State private var passkeyClient: DaemonClient?

    var body: some View {
        screen
            .firstRunPasskeySheets(copy: copy, runner: runner, account: passkeyAccount)
            .onChange(of: runner.state.step, initial: true) { _, step in onStep?(step) }
            .onChange(of: runner.completed) { _, completed in
                if completed { onComplete() }
            }
            .onChange(of: model.startup, initial: true) { _, _ in refreshPasskeyAccount() }
            .onChange(of: model.status.loggedIn, initial: true) { _, _ in recordEnrolment() }
            // Not `onOpenURL`: a link can arrive before any window exists,
            // so `AppDelegate` parks it and the first run collects it when
            // it next appears, or at once while it is up.
            .onAppear(perform: receivePendingInvite)
            .onChange(of: pendingInvite.value) { _, _ in receivePendingInvite() }
            .onChange(of: runner.isCommitting) { _, _ in
                recordEnrolment()
                receivePendingInvite()
            }
    }

    @ViewBuilder private var screen: some View {
        switch runner.state.step {
        case .join:
            JoinScreen(copy: copy, runner: runner, passkeyAccount: passkeyAccount)
        case .folders:
            FoldersScreen(copy: copy, runner: runner, installURL: copy.folders.installURL(for:))
        case .tools:
            ToolsScreen(copy: copy, runner: runner, installURL: copy.folders.installURL(for:))
        case .rules:
            RulesScreen(copy: copy, runner: runner, source: model)
        case .uses:
            UsesScreen(copy: copy, runner: runner)
        }
    }

    /// One account per daemon client: a restarted daemon gets a new one.
    /// Once there is one, a returning person is offered Welcome back (P-7);
    /// the runner decides, and offers it at most once.
    private func refreshPasskeyAccount() {
        let client = model.passkeyClient
        guard client !== passkeyClient else { return }
        passkeyClient = client
        passkeyAccount = client.map {
            LivePasskeyAccount(client: $0, presentationAnchor: { NSApp.keyWindow ?? NSApp.mainWindow })
        }
        if let passkeyAccount {
            Task { await runner.offerWelcomeBack(from: passkeyAccount) }
        }
    }

    /// An enrollment the daemon reported after the runner was made. Not
    /// during a commit: an enroll that commit runs records itself.
    private func recordEnrolment() {
        guard model.status.loggedIn, !runner.isCommitting else { return }
        let recorded = OnboardingNavigation.recordEnrolment(runner.state)
        if recorded != runner.state { runner.state = recorded }
    }

    private func receivePendingInvite() {
        guard let invite = pendingInvite.value else { return }
        switch OnboardingNavigation.receive(
            invite: invite, in: runner.state, failure: runner.failure, isCommitting: runner.isCommitting,
            hostTakesInvites: takesInvites, host: JoinScreen.defaultIssuerHost)
        {
        case .leaveParked:
            return
        case .discard:
            _ = pendingInvite.take()
        case .apply(let state, let failure):
            _ = pendingInvite.take()
            runner.state = state
            runner.failure = failure
        }
    }
}
