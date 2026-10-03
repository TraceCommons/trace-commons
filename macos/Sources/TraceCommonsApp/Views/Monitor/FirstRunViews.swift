#if DEBUG
import SwiftUI
import TCDesign
import TCShellCore

/// First run in the glass system (R12 of #1173): a single pane over the
/// painted scene, with step progress above the step (spec, "Screens").
///
/// The steps are the existing onboarding screens, sequenced by
/// `OnboardingCoordinatorView` exactly as the shipping window sequences
/// them: the same daemon calls, the same consent order, the same resume
/// rules. This view draws the frame and the progress, and nothing else.
///
/// It is gated as the main window gates its own onboarding
/// (`MainWindowView`): the coordinator is drawn only while
/// `model.requiresOnboarding`, and the window closes itself as soon as
/// that is false, so an onboarded person never lands in the flow.
///
/// The passkey popups from #1030 are not here: #1030 hides the passkey card
/// in the first release (its rule 12), and the passkey client (Z11) does
/// not exist yet.
struct FirstRunWindowView: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.dismissWindow) private var dismissWindow
    @State private var step: OnboardingNavigation.Step = .welcome
    /// Whether this run asks for the session folders: read once, when the
    /// core's startup is first known (not at appearing, before
    /// `refreshAll` has filled it), so the steps do not change shape
    /// partway through (the roots step itself clears the condition).
    @State private var asksForFolders: Bool?

    var body: some View {
        ZStack {
            // #1146's painted scene: only behind the first-run pane.
            RadialGradient(
                colors: [GlassTokens.Color.sceneWarm.color, GlassTokens.Color.sceneBase.color],
                center: .top, startRadius: 40, endRadius: 900)
                .ignoresSafeArea()
            if model.requiresOnboarding {
                GlassPane(padding: 0) {
                    VStack(spacing: 0) {
                        if let progress = FirstRunProgress(
                            step: step, folders: asksForFolders ?? false,
                            scan: model.daemonSettings?.nearAIConfigured == true
                        ) {
                            GlassStepProgress(labels: progress.labels, current: progress.current)
                                .padding(.top, GlassTokens.Space.s10)
                                .padding(.bottom, GlassTokens.Space.s6)
                                .padding(.horizontal, GlassTokens.Space.s10)
                                .frame(maxWidth: .infinity)
                        }
                        OnboardingCoordinatorView(
                            startAt: model.status.loggedIn ? .consent : .welcome,
                            onStep: { step = $0 },
                            onComplete: {
                                // As the shipping window does. The window
                                // closes when the marker took
                                // (`requiresOnboarding` turns false, below);
                                // with no tenant yet the marker is not written
                                // and the flow stays on screen to press again.
                                model.markOnboardingComplete()
                            })
                    }
                }
                .frame(width: FirstRunProgress.paneWidth)
                // As tall as the step, not the window: the pane sits on the
                // scene rather than filling it.
                .fixedSize(horizontal: false, vertical: true)
                .padding(.vertical, GlassTokens.Space.windowPadding * 3)
            }
        }
        .frame(minWidth: FirstRunProgress.paneWidth + 80, minHeight: 640)
        .onAppear { model.refreshAll() }
        .onChange(of: model.startup, initial: true) { _, startup in
            if asksForFolders == nil { asksForFolders = FirstRunProgress.asksForFolders(startup) }
        }
        .onChange(of: model.requiresOnboarding, initial: true) { _, requires in
            if !requires { dismissWindow(id: WindowID.firstRun) }
        }
    }
}

/// The step progress for one onboarding step: which steps it shows and
/// where the person is. Pure, so the mapping is tested.
///
/// The order is Folders, Join, Uses, Scan, Projects on a fresh install: the
/// folders come before Join because the daemon cannot start without them and
/// Join needs the daemon (D-3). It is not the concept's Join-first order.
struct FirstRunProgress: Equatable {
    let labels: [String]
    let current: Int

    /// Wide enough for the onboarding screens, which are laid out at 660.
    static let paneWidth: CGFloat = 700

    /// Whether this run asks for the session folders, once the core's
    /// startup says: nil while it is still starting, so the answer is never
    /// read before `refreshAll` has filled it in.
    static func asksForFolders(_ startup: AppModel.Startup) -> Bool? {
        switch startup {
        case .starting: nil
        case .needsRoots: true
        default: false
        }
    }

    /// Nil on Welcome, which is before the steps, and on Done, which is
    /// after them. The folders step shows only when this run asks for the
    /// session folders, and the scan step only when the scan will run: a
    /// step the person never takes is never drawn as done.
    init?(step: OnboardingNavigation.Step, folders: Bool, scan: Bool) {
        var steps: [(OnboardingNavigation.Step, String)] = []
        if folders { steps.append((.roots, FirstRunWords.folders)) }
        steps += [
            (.connect, FirstRunWords.join),
            (.consent, FirstRunWords.uses),
        ]
        if scan { steps.append((.privacyScan, FirstRunWords.scan)) }
        steps.append((.projects, FirstRunWords.projects))
        guard let index = steps.firstIndex(where: { $0.0 == step }) else { return nil }
        labels = steps.map(\.1)
        current = index
    }
}

/// The step names: single words, as #1030's stepper has them.
enum FirstRunWords {
    static let folders = "Folders"
    static let join = "Join"
    static let uses = "Uses"
    static let scan = "Scan"
    static let projects = "Projects"
}
#endif
