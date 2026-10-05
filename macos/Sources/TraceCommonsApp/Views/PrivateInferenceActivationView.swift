import SwiftUI

/// Cloud sign-in and tool setup require the local daemon, not Commons
/// enrollment. Reuse the explicit capture choices (the first run's Folders)
/// without finishing the first run. Layout follows DesignSystem.swift.
struct PrivateInferenceActivationView: View {
    @EnvironmentObject private var model: AppModel

    var body: some View {
        content.onAppear { model.refreshAll() }
    }

    @ViewBuilder
    private var content: some View {
        switch model.startup {
        case .starting, .refused:
            DaemonStartupNotice(startup: model.startup)
        case .needsRoots:
            // The first run's Folders step, which starts the daemon with
            // the person's answers. Its commit joins nothing unless Join
            // was visited and answered, and once the daemon runs this view
            // shows Private AI, never the rest of the first run.
            OnboardingCoordinatorView(startAt: .folders, onComplete: {})
        case .running:
            PrivateInferenceView()
        }
    }
}
