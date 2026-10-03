import AppKit
import Foundation
import SwiftUI

/// Writes PNGs of the shell's real views, driven by the real running daemon,
/// when `TRACE_COMMONS_SCREENSHOT_DIR` is set.
///
/// A development hook, not a product feature. It exists because a menu-bar
/// app has to be *seen* to be verified, and it rasterizes with
/// `ImageRenderer` rather than photographing windows: `screencapture` and
/// `cacheDisplay` both come back blank when the desktop session is locked,
/// since nothing is being composited. `ImageRenderer` runs on the CPU and
/// does not care.
///
/// What it renders is the shipping view hierarchy bound to live daemon data
/// -- the same `MainWindowView`, `MenuBarGlassPanel` and `PreviewSheet` a person
/// sees -- not a mock-up. The one accommodation is that `ImageRenderer`
/// never runs `task`/`onAppear`, so the sheet is handed content that was
/// loaded first through the ordinary preview path.
enum DebugScreenshot {
    static var directory: String? {
        let value = ProcessInfo.processInfo.environment["TRACE_COMMONS_SCREENSHOT_DIR"]
        return (value?.isEmpty == false) ? value : nil
    }

    @MainActor
    static func scheduleIfRequested(model: AppModel) {
        guard let directory else { return }
        Task { @MainActor in
            // Late enough that the watcher has polled, queued, and scrubbed.
            try? await Task.sleep(nanoseconds: 12_000_000_000)

            render(
                QueueContent(previewing: .constant(nil)).environmentObject(model),
                to: directory + "/macos-shell-window.png",
                size: CGSize(width: 860, height: 640)
            )
            #if DEBUG
            // The glass menu-bar item over its panel, on a store loaded
            // first, since the strip's own attaching `task` never runs here.
            // Debug-only with the panel until T11 (ruling R-35).
            let menuPanel = MenuPanelStore(client: nil)
            menuPanel.attach(model.daemonData, configDirectory: model.configDirectory)
            await menuPanel.load()
            render(
                MenuBarPreviewWindow(store: menuPanel).environmentObject(model),
                to: directory + "/macos-shell-menu-bar.png",
                size: CGSize(width: 480, height: 760)
            )
            #endif
            render(
                ConsentScopesContent(onContinue: { _ in }).environmentObject(model),
                to: directory + "/macos-shell-consent-scopes.png",
                size: CGSize(width: 660, height: 760)
            )
            // 660 wide like the other onboarding screens: the glass layout
            // has no globe ladder, so one width shows the screen as shipped.
            render(
                OnboardingWelcomeContent(onGetStarted: {}, onWhatGetsRemoved: {}),
                to: directory + "/macos-shell-onboarding-welcome.png",
                size: CGSize(width: 660, height: 560)
            )
            render(
                OnboardingProjectsContent(onContinue: {}).environmentObject(model),
                to: directory + "/macos-shell-onboarding-projects.png",
                size: CGSize(width: 660, height: 520)
            )
            render(
                OnboardingDoneContent(onFinish: {}),
                to: directory + "/macos-shell-onboarding-done.png",
                size: CGSize(width: 660, height: 360)
            )
            render(
                OnboardingConnectContent(
                    onEnrolled: {},
                    previewPhase: .resolved(InviteLink(
                        raw: "https://issuer.tracecommons.ai/onboard#SAMPLECODE",
                        issuerHost: "issuer.tracecommons.ai"
                    ))
                ),
                to: directory + "/macos-shell-onboarding-connect.png",
                size: CGSize(width: 660, height: 420)
            )
            render(
                OnboardingConnectContent(
                    onEnrolled: {},
                    previewPhase: .deadInvite,
                    previewText: "https://issuer.tracecommons.ai/onboard#EXPIRED"
                ),
                to: directory + "/macos-shell-onboarding-connect-dead-invite.png",
                size: CGSize(width: 660, height: 420)
            )
            // Settings is where the local change log lives, and a log is a
            // surface that can only be checked by looking at it: the rows
            // are small secondary text in two columns, which is exactly the
            // combination that fails contrast or collapses at width without
            // anyone noticing from a green build.
            // The log is drawn alone, so it is never the part of a long
            // stack of sections that falls off the bottom of the image.
            render(
                GlassSettingsContent(section: .changes).environmentObject(model),
                to: directory + "/macos-shell-settings.png",
                size: CGSize(width: 860, height: 620)
            )
            render(
                WithdrawalConfirmationCapture().environmentObject(model),
                to: directory + "/macos-shell-withdrawal.png",
                size: CGSize(width: 860, height: 620)
            )
            render(
                OnboardingPrivacyScanContent(onContinue: {}),
                to: directory + "/macos-shell-onboarding-privacy-scan.png",
                size: CGSize(width: 660, height: 560)
            )
            if let rollup = model.rollup {
                render(
                    CreditRecordView(
                        creditFinal: rollup.creditFinal,
                        creditPending: rollup.creditPending,
                        lastRefreshedAt: rollup.lastRefreshedAt
                    )
                    .padding(24)
                    .frame(maxWidth: 620, alignment: .leading),
                    to: directory + "/macos-shell-credit-record.png",
                    size: CGSize(width: 660, height: 260)
                )
            }
            if let copy = model.witnessCopy?.review {
                render(
                    WitnessReviewConsent(copy: copy, onConfirm: {}),
                    to: directory + "/macos-shell-witness-review-consent.png",
                    size: CGSize(width: 560, height: 390)
                )
            }
            if let (entry, preloaded) = await model.loadCaptureSample(needle: "Northwind") {
                // 760 x 620 is the sheet's own frame, not a chosen canvas:
                // `PreviewSheet` sets that width from the design spec's
                // §4.6 sheet measure. A larger canvas here does not enlarge
                // the sheet, it just bands unused ground down the right-hand
                // edge of the capture, which reads as a layout bug in a
                // review. Keep the two numbers together.
                render(
                    PreviewSheet(entry: entry, preloaded: preloaded).environmentObject(model),
                    to: directory + "/macos-shell-preview-sheet.png",
                    size: CGSize(width: 760, height: 620)
                )
            }
            if ProcessInfo.processInfo.environment["TRACE_COMMONS_QUIT_AFTER_SHOT"] == "1" {
                // Late enough that the self-test, which starts on the same
                // clock, has finished writing.
                try? await Task.sleep(nanoseconds: 25_000_000_000)
                model.shutdown()
                NSApp.terminate(nil)
            }
        }
    }

    @MainActor
    private static func render<V: View>(_ view: V, to path: String, size: CGSize) {
        let renderer = ImageRenderer(
            content: view
                .frame(width: size.width, height: size.height)
                .background(Color(nsColor: .windowBackgroundColor))
        )
        renderer.scale = 2
        guard let image = renderer.nsImage,
              let tiff = image.tiffRepresentation,
              let rep = NSBitmapImageRep(data: tiff),
              let data = rep.representation(using: .png, properties: [:])
        else {
            NSLog("trace-commons: could not render \(path)")
            return
        }
        try? data.write(to: URL(fileURLWithPath: path))
        NSLog("trace-commons: wrote \(path)")
    }
}
