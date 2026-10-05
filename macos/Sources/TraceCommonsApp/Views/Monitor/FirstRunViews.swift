#if DEBUG
import SwiftUI
import TCDesign
import TCShellCore

/// First run in the glass system (R12 of #1173): Ron's first run (#1030)
/// over the painted scene.
///
/// The steps are `OnboardingCoordinatorView`'s, the same host the shipping
/// window uses: the same daemon calls, in the same order. Each step draws
/// its own pane and step progress (`FirstRunFrame`), so this view draws
/// only the scene and sizes the pane.
///
/// It is gated as the main window gates its own onboarding
/// (`MainWindowView`): the coordinator is drawn only while
/// `model.requiresOnboarding`, and the window closes itself as soon as
/// that is false, so an onboarded person never lands in the flow.
struct FirstRunWindowView: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.dismissWindow) private var dismissWindow

    var body: some View {
        ZStack {
            // #1146's painted scene: only behind the first-run pane.
            RadialGradient(
                colors: [GlassTokens.Color.sceneWarm.color, GlassTokens.Color.sceneBase.color],
                center: .top, startRadius: 40, endRadius: 900)
                .ignoresSafeArea()
            if model.requiresOnboarding {
                OnboardingCoordinatorView(
                    onComplete: {
                        // As the shipping window does. The window closes
                        // when the marker took (`requiresOnboarding` turns
                        // false, below); with no tenant yet the marker is
                        // not written and the flow stays on screen.
                        model.markOnboardingComplete()
                    })
                    .frame(width: FirstRunProgress.paneWidth)
                    .padding(.vertical, GlassTokens.Space.windowPadding * 3)
            }
        }
        .frame(minWidth: FirstRunProgress.paneWidth + 80, minHeight: 640)
        .onAppear { model.refreshAll() }
        .onChange(of: model.requiresOnboarding, initial: true) { _, requires in
            if !requires { dismissWindow(id: WindowID.firstRun) }
        }
    }
}

/// The first-run pane's size.
enum FirstRunProgress {
    /// Wide enough for Ron's screens.
    static let paneWidth: CGFloat = 700
}
#endif
