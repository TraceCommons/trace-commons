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
/// It is gated as the main window gates its own first run
/// (`OnboardingNavigation.hostsFirstRun`, with this window's own `entered`):
/// the core's startup notice while the daemon is starting or was refused
/// at launch, the coordinator while onboarding is required, and the window
/// closes itself as soon as it is not and opens the Monitor, so an
/// onboarded person never lands in the flow.
struct FirstRunWindowView: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.dismissWindow) private var dismissWindow
    @State private var entered = false

    var body: some View {
        ZStack {
            // #1146's painted scene: only behind the first-run pane.
            RadialGradient(
                colors: [GlassTokens.Color.sceneWarm.color, GlassTokens.Color.sceneBase.color],
                center: .top, startRadius: 40, endRadius: 900)
                .ignoresSafeArea()
            if OnboardingNavigation.hostsFirstRun(
                startup: model.startup, requiresOnboarding: model.requiresOnboarding, entered: entered)
            {
                VStack(spacing: GlassTokens.Space.s6) {
                    // A void or a gate hold can arrive while someone is
                    // still setting up, and is told here too, above the
                    // step's own pane.
                    ShellNotices()
                    // Start writes its marker (the tenant's, or watching
                    // only's) itself; the window closes when it took
                    // (`requiresOnboarding` turns false, below).
                    OnboardingCoordinatorView(onComplete: {})
                        .onAppear { entered = true }
                }
                // Bounded by the window, not grown to the step: each step
                // scrolls in its own ScrollView, which a pane sized to its
                // step would let run past the window's bottom edge (a Uses
                // step with many scopes), out of reach.
                .frame(width: FirstRunProgress.paneWidth)
                .padding(.vertical, GlassTokens.Space.windowPadding * 3)
            } else if model.requiresOnboarding {
                DaemonStartupNotice(startup: model.startup)
                    .frame(width: FirstRunProgress.paneWidth)
            }
        }
        .frame(minWidth: FirstRunProgress.paneWidth + 80, minHeight: 640)
        .onAppear { model.refreshAll() }
        // Finishing first run hands off to the Monitor on Home. Initially
        // too: first run opened for someone already onboarded closes and
        // opens the Monitor instead.
        .onChange(of: model.requiresOnboarding, initial: true) { _, requires in
            guard !requires else { return }
            dismissWindow(id: WindowID.firstRun)
            OpenMonitor.request(.home(.overview))
        }
    }
}

/// The first-run pane's size.
enum FirstRunProgress {
    /// Wide enough for Ron's screens.
    static let paneWidth: CGFloat = 700
}
#endif
