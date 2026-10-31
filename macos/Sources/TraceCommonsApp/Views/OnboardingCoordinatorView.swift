import AppKit
import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// Hosts the animated first run (Ron's 2026-10-21 canvas, built on the
/// owner's request of 2026-10-22): Welcome, Tools, Uses, Traces, Private AI
/// and Join, one screen per step over one scene, with the step thread above
/// them and the NEAR mark and the tool tiles moving from step to step.
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
/// - the scene, the step thread and what the stage remembers
///   (`FirstRunStageMemory`), so going Back does not replay the scan;
/// - invite links (`PendingInvite`), which fill Join's invite, in a host
///   that takes them (`takesInvites`).
///
/// The daemon is not running on a fresh install: the core refuses to start
/// it until the session roots are declared, and Tools is where they are
/// (`CommitPoint.leaveTools`, or Customize's `declareRoots`). Signing in is
/// the last step, Join, where the daemon is already running, so the invite
/// is looked up and joined, and near.ai signed in, at once
/// (`CommitPoint.signIn`).
///
/// ## Resuming
///
/// A host starts the first run at `startAt` (Welcome, unless it names
/// another step). What an earlier first run left on the daemon is recorded
/// (`OnboardingNavigation.initialState`): a running daemon is not started
/// again, and an enrollment is the account (`recordEnrolment`), so its
/// invite is not joined again. The daemon's first status can arrive after
/// this view is up, so the enrollment is recorded then too. Start's marker
/// (the tenant's, or watching only's) is what tells "enrolled" from "set
/// up".
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
        startAt: Step = .welcome, onStep: ((Step) -> Void)? = nil, takesInvites: Bool = true,
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
    @StateObject private var memory = FirstRunStageMemory()
    @Namespace private var actors
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        FirstRunStageFit {
            ZStack(alignment: .top) {
                FirstRunBackdrop()
                screen
                if let index = FirstRunStageLayout.threadIndex(of: runner.state.step) {
                    FirstRunStepThread(labels: copy.threadLabels, current: index, enabled: !runner.isCommitting) { picked in
                        runner.state = FirstRunNavigation.back(
                            to: FirstRunNavigation.threadSteps[picked], from: runner.state)
                    }
                    .padding(.top, 14)
                    .transition(.opacity.combined(with: .offset(y: -10)))
                }
            }
            .environment(\.firstRunActors, actors)
            .animation(GlassMotion.spatial(reduceMotion), value: runner.state.step)
        }
        .onChange(of: runner.state.step, initial: true) { _, step in
            onStep?(step)
        }
        .onChange(of: runner.completed) { _, completed in
            if completed { onComplete() }
        }
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
        case .welcome:
            WelcomeScreen(copy: copy, runner: runner)
                .transition(.opacity)
        case .tools:
            ToolsScreen(copy: copy, runner: runner, memory: memory, source: model)
                .transition(.opacity)
        case .uses:
            UsesScreen(copy: copy, runner: runner)
                .transition(.opacity)
        case .traces:
            TracesScreen(copy: copy, runner: runner, memory: memory)
                .transition(.opacity)
        case .privateAI:
            PrivateAIScreen(copy: copy, runner: runner, memory: memory)
                .transition(.opacity)
        case .join:
            JoinScreen(copy: copy, runner: runner)
                .transition(.opacity)
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

/// The stage at its design size, scaled down to fit a smaller host (the
/// Private AI tab's Tools step), and at the host's size otherwise.
private struct FirstRunStageFit<Content: View>: View {
    @ViewBuilder let content: Content

    var body: some View {
        GeometryReader { geometry in
            let canvas = FirstRunStageLayout.canvas
            let scale = min(1, geometry.size.width / canvas.width, geometry.size.height / canvas.height)
            if scale < 1 {
                content
                    .frame(width: canvas.width, height: canvas.height)
                    .scaleEffect(scale)
                    .frame(width: geometry.size.width, height: geometry.size.height)
            } else {
                content
                    .frame(width: geometry.size.width, height: geometry.size.height)
            }
        }
    }
}
