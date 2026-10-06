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
/// It is the onboarding gate (R15), through
/// `OnboardingNavigation.hostsFirstRun` with this window's own `entered`:
/// the core's startup notice while the daemon is starting or was refused
/// at launch, the coordinator while onboarding is required, and the window
/// closes itself as soon as it is not and opens the Monitor, so an
/// onboarded person never lands in the flow.
struct FirstRunWindowView: View {
    /// Where an earlier opener asked the Monitor to go, kept through the
    /// hand-off (`LaunchRouting.handOff`).
    let navigation: MainWindowNavigation
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
                // The legacy startup notice left with the legacy window
                // (R15); this says the startup as the Inference tab does: a
                // spinner while starting, the core's down title over the
                // refusal's sentence.
                Group {
                    switch model.startup {
                    case .starting:
                        SettingsAwaiting()
                    case .refused(let sentence):
                        StartupRefusedBanner(sentence: sentence)
                    case .needsRoots, .running:
                        EmptyView()
                    }
                }
                .frame(width: FirstRunProgress.paneWidth)
            }
        }
        .frame(minWidth: FirstRunProgress.paneWidth + 80, minHeight: 640)
        // A screen's modals and confirmations cover the whole window.
        .glassModalHost()
        .onAppear { model.refreshAll() }
        // Finishing first run hands off to the Monitor: at the destination
        // an earlier opener left waiting, on Home otherwise. Initially too:
        // first run opened for someone already onboarded closes and opens
        // the Monitor instead.
        .onChange(of: model.requiresOnboarding, initial: true) { _, requires in
            guard !requires else { return }
            dismissWindow(id: WindowID.firstRun)
            OpenMonitor.request(LaunchRouting.handOff(pending: navigation.pending))
        }
    }
}

/// A daemon that refused to start, said as the Inference tab says it: the
/// core's down title over the refusal's sentence. First run, the Monitor's
/// pane and a writing Settings section all draw this one, never the
/// onboarding notice: a refusal says nothing about whether the person
/// finished onboarding.
struct StartupRefusedBanner: View {
    let sentence: String

    var body: some View {
        GlassHealthBanner(banner: .init(
            title: TracesHealth.coreDownLine?.title ?? TracesHealth.unknownWord ?? "",
            detail: sentence, tone: .outside))
    }
}

/// The first-run pane's size.
enum FirstRunProgress {
    /// Wide enough for Ron's screens.
    static let paneWidth: CGFloat = 700
}
