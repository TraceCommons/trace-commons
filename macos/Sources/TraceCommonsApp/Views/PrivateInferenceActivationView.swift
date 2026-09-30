import SwiftUI

/// Cloud sign-in and tool setup require the local daemon, not Commons
/// enrollment. Reuse the explicit capture choices without advancing the
/// contribution coordinator. Layout follows DesignSystem.swift.
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
            ScrollView {
                OnboardingRootsView(configDirectory: model.configDirectory, onStarted: {})
            }
        case .running:
            PrivateInferenceView()
        }
    }
}
