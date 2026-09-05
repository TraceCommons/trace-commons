import SwiftUI

/// Chains the existing onboarding screens into one first-run flow.
///
/// Each screen already exists (`OnboardingWelcomeView`, `OnboardingRootsView`,
/// `OnboardingConnectView`, `ConsentScopesView`, `OnboardingPrivacyScanView`,
/// `OnboardingProjectsView`, `OnboardingDoneView`) and drives its own daemon
/// call. This view owns only the sequencing between them and the one piece
/// of state that must survive a screen change: the consent scopes chosen on
/// the consent screen, which the screens after it must not discard if the
/// contributor goes back to revisit them.
///
/// ## The roots screen comes second, not first
///
/// On a fresh install the daemon refuses to start until the session roots
/// are declared (`AppModel.Startup.needsRoots`), and the roots screen is
/// what clears that. It used to be rendered directly on that state, so the
/// first thing a new contributor ever saw was "Which folders may this app
/// watch?" -- a consent question asked before the screen that says what the
/// app is and why it wants anything. Welcome now comes first on that path
/// too; `Get started` goes to the roots screen while the daemon is still
/// refusing and straight to Connect once it is running. The roots screen
/// needs no daemon, so nothing about it changes by moving it.
///
/// ## Call ordering (why `enroll` carries no scopes)
///
/// The contract's `enroll` entry (`docs/contributor-daemon-ipc-v1_1.md`)
/// accepts an optional `scopes` array at enroll time. That would be the
/// simpler wire sequence -- one call instead of two -- but it does not fit
/// this product's fixed screen order: screen 2 (Connect, which fires
/// `enroll`) comes *before* screen 3 (the actual consent decision). Asking
/// `OnboardingConnectView` to somehow carry screen 3's answer backward in
/// time is not an option, so this coordinator does the opposite: `enroll`
/// runs with no `scopes` (the daemon then applies the floor scope,
/// `debugging_evaluation`, only), and screen 3's `Continue` calls
/// `set_consent_scopes` -- a separate, local-only, already-enrolled-only
/// call built for exactly this -- to apply what the contributor actually
/// chose. `set_consent_scopes` requires an existing enrollment, which
/// `enroll` on screen 2 has, by this point, already established.
///
/// ## Atomicity: what is atomic, what is resumable
///
/// Each daemon call in this flow (`enroll`, `set_consent_scopes`,
/// `acknowledge_near_ai_notice`, `set_project_mode`) is individually
/// atomic: it either lands on the daemon or it visibly fails, and this
/// coordinator does not advance past a failed one silently (see
/// `advanceFromConsent`).
///
/// The onboarding *sequence* is deliberately NOT atomic -- there is no
/// wire-level transaction spanning `enroll` through the Done screen, and
/// there could not be one without a contract change this task is not
/// permitted to make. What makes that safe is that the two states an
/// interrupted contributor can be found in are told apart, not conflated:
///
/// - Not yet enrolled (`status.logged_in == false`): show onboarding from
///   the top (`Welcome`).
/// - Enrolled but onboarding not finished (`status.logged_in == true` and
///   `AppModel.isOnboardingComplete == false`, a local marker this
///   coordinator sets only when the Done screen's button is pressed): show
///   onboarding resumed at the Consent screen -- `enroll` already ran, so
///   there is nothing to redo there, but nothing past it is trustworthy
///   yet.
/// - Enrolled and onboarding finished: show the main window.
///
/// This is what keeps a crash or quit between `enroll` and Done from ever
/// landing a contributor in the main window with an unset (floor-only)
/// consent choice they never actually confirmed -- the forbidden outcome.
/// `TraceCommonsAppMain`/`MainWindowView` is what reads `isOnboardingComplete`
/// to make that branch; this view only needs `startAt` to know where in the
/// sequence to resume.
struct OnboardingCoordinatorView: View {
    @EnvironmentObject private var model: AppModel
    var startAt: Step = .welcome
    /// Called once the Done screen's button is pressed. The caller (not
    /// this view) is responsible for calling `AppModel.markOnboardingComplete()`
    /// and for whatever transition follows -- this view has no opinion on
    /// what replaces it.
    var onComplete: () -> Void

    enum Step: Equatable {
        case welcome
        /// Only on a fresh install: the daemon is refusing to start until
        /// the session roots are declared. Never resumed to -- a daemon that
        /// is running already has its roots.
        case roots
        case connect
        case consent
        case privacyScan
        case projects
        case done

        static func afterWelcome(needsRoots: Bool) -> Self {
            needsRoots ? .roots : .connect
        }

        func previous(privacyScanConfigured: Bool) -> Self? {
            switch self {
            case .welcome: return nil
            case .roots, .connect: return .welcome
            case .consent: return .connect
            case .privacyScan: return .consent
            case .projects: return privacyScanConfigured ? .privacyScan : .consent
            case .done: return .projects
            }
        }
    }

    @State private var step: Step
    /// Names of optional scopes ticked on screen 3, kept here (not just
    /// inside `ConsentScopesContent`) so a trip to screen 4 or 5 and back
    /// does not lose the choice.
    @State private var selectedScopes: Set<String> = []
    @State private var consentSaveFailed = false
    @State private var consentSaveInProgress = false
    /// Reference material for the welcome screen, presented as a sheet
    /// rather than a step of its own -- it asks for no decision, and this
    /// flow is one decision per screen.
    @State private var showingWhatGetsRemoved = false

    init(startAt: Step = .welcome, onComplete: @escaping () -> Void) {
        self.startAt = startAt
        self.onComplete = onComplete
        _step = State(initialValue: startAt)
    }

    var body: some View {
        VStack(spacing: 0) {
            if let previous = previousStep {
                backBar(to: previous)
            }
            content
        }
        .disabled(consentSaveInProgress)
        .sheet(isPresented: $showingWhatGetsRemoved) {
            WhatGetsRemovedSheet()
        }
    }

    /// Where Back goes from the current step, or nil on Welcome, which has
    /// nothing before it.
    ///
    /// Every step after Welcome has one. Back used to exist on only two of
    /// them (the scan and projects screens, both returning to consent), so
    /// a contributor on Connect who wanted to re-read the welcome screen,
    /// or on Done who wanted to ignore one more project, had no way there
    /// except quitting. Back from Done returns to the projects screen, and
    /// the scan screen is skipped on the way back exactly when it was
    /// skipped on the way forward.
    ///
    /// Back from consent lands on Connect, where the device is by then
    /// already enrolled; that screen says so and offers Continue rather
    /// than a second enrolment (see `OnboardingConnectContent`).
    private var previousStep: Step? {
        step.previous(privacyScanConfigured: model.daemonSettings?.nearAIConfigured == true)
    }

    private func backBar(to previous: Step) -> some View {
        HStack {
            Button {
                step = previous
            } label: {
                Label("Back", systemImage: "chevron.left")
            }
            .buttonStyle(.plain)
            .foregroundStyle(.secondary)
            .accessibilityLabel("Back to the previous step")
            Spacer()
        }
        .padding(.horizontal, 24)
        .padding(.top, 16)
    }

    @ViewBuilder
    private var content: some View {
        switch step {
        case .welcome:
            // `onWhatGetsRemoved` used to be left at its `= {}` default here,
            // so the link on that screen was live, clickable and did nothing.
            // None of these callbacks carry a default any more -- omitting
            // one is a compile error rather than a silent dead control.
            OnboardingWelcomeView(
                onGetStarted: { step = .afterWelcome(needsRoots: model.startup == .needsRoots) },
                onWhatGetsRemoved: { showingWhatGetsRemoved = true }
            )

        case .roots:
            OnboardingRootsView(
                configDirectory: model.configDirectory,
                onStarted: { step = .connect }
            )

        case .connect:
            OnboardingConnectView(onEnrolled: {
                // An invite request may finish after Back was pressed.
                // Enrollment remains valid, but must not move a newer screen.
                if step == .connect { step = .consent }
            })

        case .consent:
            VStack(alignment: .leading, spacing: 8) {
                if consentSaveFailed {
                    Text("""
                    Couldn't save your choices -- the watcher may not be running. \
                    Check your connection and try again.
                    """)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, 24)
                    .padding(.top, 16)
                }
                ConsentScopesView(onContinue: advanceFromConsent, initialSelection: selectedScopes)
            }

        case .privacyScan:
            OnboardingPrivacyScanView(onContinue: { step = .projects })
                // The operator may not have configured the second scanner at
                // all (`OnboardingPrivacyScanView` renders nothing in that
                // case, per its own gate) -- skip straight past it rather
                // than leave the contributor on a blank screen.
                .onAppear {
                    if model.daemonSettings?.nearAIConfigured != true {
                        step = .projects
                    }
                }

        case .projects:
            OnboardingProjectsView(onContinue: { step = .done })

        case .done:
            OnboardingDoneView(onFinish: onComplete)
        }
    }

    /// Applies the chosen scopes via `set_consent_scopes` and only advances
    /// once the daemon confirms it -- see the type comment's "Call
    /// ordering" section for why this call, not `enroll`, is the one that
    /// actually applies consent in this flow. A failure leaves the
    /// contributor on this same screen with their ticks intact (`selected`
    /// is local `@State` inside `ConsentScopesContent` until `onContinue`
    /// fires, and re-entry seeds from `selectedScopes` either way), rather
    /// than silently moving on with the floor-only scope `enroll` left in
    /// place.
    private func advanceFromConsent(_ selected: Set<String>) {
        guard !consentSaveInProgress else { return }
        consentSaveInProgress = true
        selectedScopes = selected
        consentSaveFailed = false
        let alwaysOn = model.consentScopes.filter(\.alwaysOn).map(\.name)
        let scopes = Array(Set(alwaysOn).union(selected))
        Task {
            defer { consentSaveInProgress = false }
            switch await model.setConsentScopes(scopes) {
            case .succeeded:
                if model.daemonSettings?.nearAIConfigured == true {
                    step = .privacyScan
                } else {
                    step = .projects
                }
            case .failed:
                consentSaveFailed = true
            }
        }
    }
}
