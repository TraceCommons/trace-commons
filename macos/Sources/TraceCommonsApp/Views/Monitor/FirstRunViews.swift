import SwiftUI
import TCDesign
import TCShellCore

/// First run in the glass system (R12 of #1173): the animated first run
/// (Ron's 2026-10-21 canvas) filling the window.
///
/// The steps are `OnboardingCoordinatorView`'s, the same host the Private AI
/// tab uses for its Tools step: the same daemon calls, in the same order.
/// The coordinator draws the scene, the step thread and each step, so this
/// view only hosts it.
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
        ZStack(alignment: .top) {
            FirstRunBackdrop()
            if OnboardingNavigation.hostsFirstRun(
                startup: model.startup, requiresOnboarding: model.requiresOnboarding, entered: entered)
            {
                // Start writes its marker (the tenant's, or watching only's)
                // itself; the window closes when it took (`requiresOnboarding`
                // turns false, below).
                OnboardingCoordinatorView(onComplete: {})
                    .onAppear { entered = true }
                // A void or a gate hold can arrive while someone is still
                // setting up, and is told here too, over the step.
                ShellNotices()
                    .frame(width: FirstRunProgress.noticeWidth)
                    .padding(.top, 52)
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
                .frame(width: FirstRunProgress.noticeWidth)
                .frame(maxHeight: .infinity)
            }
        }
        // A staging build's label (spec S7, 4.8), as on the Monitor, over
        // every state of this window: sign-in is where a tester most needs
        // to tell staging from production. Production has none. It sits at
        // the left of the top row beside the window's buttons, where the
        // Monitor puts it, so it stays clear of the step thread the stage
        // draws across the top centre.
        .overlay(alignment: .topLeading) {
            if let label = BuildChannelGate.windowLabel {
                GlassTag(label)
                    .padding(.top, GlassTokens.Space.s3)
                    .padding(.leading, FirstRunProgress.windowButtonsWidth)
            }
        }
        .frame(minWidth: FirstRunProgress.minimumSize.width, minHeight: FirstRunProgress.minimumSize.height)
        // A screen's modals and confirmations cover the whole window.
        .glassModalHost()
        // A folder's tile mark, in the core's words.
        .environment(\.glassFolderMark, TracesTreeWords.table?.folderMark ?? "")
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
    /// Drawn as a popover notice over the Monitor's tab (owner, 2026-10-19,
    /// `MonitorNotices`) rather than a banner in the flow.
    var popover = false

    private var title: String { TracesHealth.coreDownLine?.title ?? TracesHealth.unknownWord ?? "" }

    var body: some View {
        if popover {
            GlassPopoverNotice(title: title) {
                GlassNoticeMark(.outside)
            } summary: {
                Text(sentence)
            } details: { EmptyView() } actions: { EmptyView() }
        } else {
            GlassHealthBanner(banner: .init(title: title, detail: sentence, tone: .outside))
        }
    }
}

/// The first-run window's sizes.
enum FirstRunProgress {
    /// The smallest window the stage lays out in without scaling down.
    static let minimumSize = CGSize(width: 760, height: 640)
    /// The width the window's close, minimize and zoom buttons take at the
    /// top left, which the staging label starts past.
    static let windowButtonsWidth: CGFloat = 80
    /// The width of the startup notice and of the shell's notices over a
    /// step: Ron's #1030 pane width.
    static let noticeWidth: CGFloat = 450
}
