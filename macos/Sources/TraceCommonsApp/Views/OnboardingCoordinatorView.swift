import AppKit
import SwiftUI
import TCBridge
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
///   shows is the core's, and none is written here to fall back on;
/// - the passkey sheets, mounted here exactly once, so a request the
///   Folders or Tools commit raises presents on whichever step follows;
/// - invite links (`PendingInvite`), which fill Join and bring it up.
///
/// The daemon is not running on a fresh install: the core refuses to start
/// it until the session roots are declared, and Folders or Tools is where
/// they are. So the invite pasted on Join is looked up and joined, and the
/// account chosen there signed in or created, only once that commit has
/// started the daemon (`CommitPoint.leaveRoots`).
///
/// ## Resuming
///
/// The first run is not resumed mid-way: a host starts it at `startAt`
/// (Join, unless it names another step). A person whose earlier first run
/// started the daemon or enrolled finds the daemon recorded as started
/// (`OnboardingNavigation.initialState`), so it is not started again, and
/// the completion marker (`AppModel.isOnboardingComplete`, written by Start)
/// is what tells "enrolled" from "set up": the main window shows its
/// content only once both hold.
struct OnboardingCoordinatorView: View {
    @EnvironmentObject private var model: AppModel
    var startAt: Step
    /// Told the step each time it changes, and once on appearing.
    var onStep: ((Step) -> Void)?
    /// Called once Start has finished the first run. The caller decides
    /// what replaces this view.
    var onComplete: () -> Void

    typealias Step = OnboardingNavigation.Step

    @State private var copy = TCCoreCopy.firstRunCopyJSON().flatMap(FirstRunCopy.decode)
    /// Made on first appearing, since it runs its calls through the
    /// environment's `AppModel`, and kept for the whole first run.
    @State private var runner: FirstRunRunner?

    init(startAt: Step = .join, onStep: ((Step) -> Void)? = nil, onComplete: @escaping () -> Void) {
        self.startAt = startAt
        self.onStep = onStep
        self.onComplete = onComplete
    }

    var body: some View {
        Group {
            if let copy, let runner {
                FirstRunSteps(copy: copy, runner: runner, onStep: onStep, onComplete: onComplete)
            } else {
                ProgressView().controlSize(.small)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .onAppear {
            guard runner == nil else { return }
            runner = FirstRunRunner(
                state: OnboardingNavigation.initialState(startAt: startAt, daemonRunning: model.startup == .running),
                daemon: model)
        }
    }
}

/// The current step's screen, and what spans the steps.
private struct FirstRunSteps: View {
    @EnvironmentObject private var model: AppModel
    let copy: FirstRunCopy
    @ObservedObject var runner: FirstRunRunner
    let onStep: ((OnboardingNavigation.Step) -> Void)?
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
            // Not `onOpenURL`: a link can arrive before any window exists,
            // so `AppDelegate` parks it and the first run collects it when
            // it next appears, or at once while it is up.
            .onAppear(perform: receivePendingInvite)
            .onChange(of: pendingInvite.value) { _, _ in receivePendingInvite() }
            .onChange(of: runner.isCommitting) { _, _ in receivePendingInvite() }
    }

    @ViewBuilder private var screen: some View {
        switch runner.state.step {
        case .join:
            JoinScreen(copy: copy, runner: runner, passkeyAccount: passkeyAccount)
        case .folders:
            FoldersScreen(copy: copy, runner: runner)
        case .tools:
            ToolsScreen(copy: copy, runner: runner)
        case .rules:
            RulesScreen(
                copy: copy,
                state: $runner.state,
                source: model,
                onBack: { runner.state = FirstRunNavigation.back(runner.state) },
                onContinue: { runner.state = FirstRunNavigation.next(runner.state) })
        case .uses:
            UsesScreen(runner: runner, copy: copy)
        }
    }

    /// One account per daemon client: a restarted daemon gets a new one.
    private func refreshPasskeyAccount() {
        let client = model.passkeyClient
        guard client !== passkeyClient else { return }
        passkeyClient = client
        passkeyAccount = client.map {
            LivePasskeyAccount(client: $0, presentationAnchor: { NSApp.keyWindow ?? NSApp.mainWindow })
        }
    }

    private func receivePendingInvite() {
        guard !runner.isCommitting, let invite = pendingInvite.take() else { return }
        guard
            let received = OnboardingNavigation.receive(
                invite: invite, in: runner.state, failure: runner.failure, isCommitting: false,
                host: JoinScreen.defaultIssuerHost)
        else { return }
        runner.state = received.state
        runner.failure = received.failure
    }
}
