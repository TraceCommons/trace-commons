import AppKit
import Foundation
import SwiftUI
import TCDesign

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
/// -- the same Traces tree, `MenuBarGlassPanel` and `PreviewSheet` a person
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

            // The Monitor's screens. Each store is loaded first, since the
            // window's attaching `task` never runs under `ImageRenderer`.
            let traces = TracesStore(client: nil)
            traces.attach(model.daemonData)
            await traces.load()
            render(
                TracesTreeView(store: traces, selection: .constant(traces.tree.allSessions.first?.entryId ?? ""))
                    .environmentObject(model),
                to: directory + "/macos-shell-traces-tree.png",
                size: CGSize(width: 760, height: 720)
            )
            render(
                SessionInspectorView(store: traces, entry: traces.tree.allSessions.first)
                    .padding(GlassTokens.Space.cardGap)
                    .environmentObject(model),
                to: directory + "/macos-shell-session-inspector.png",
                size: CGSize(width: 420, height: 720)
            )
            let home = HomeStore(client: nil)
            home.attach(model.daemonData)
            await home.load()
            if let row = home.history?.first {
                render(
                    HistoryDetailInspector(row: row)
                        .padding(GlassTokens.Space.cardGap)
                        .environmentObject(model),
                    to: directory + "/macos-shell-history-inspector.png",
                    size: CGSize(width: 420, height: 720)
                )
            }
            let inference = InferenceStore(client: nil)
            inference.attach(model.daemonData)
            await inference.load()
            render(
                InferenceAccountSection(store: inference)
                    .padding(GlassTokens.Space.cardGap)
                    .environmentObject(model),
                to: directory + "/macos-shell-inference-account.png",
                size: CGSize(width: 420, height: 720)
            )
            #if DEBUG
            // The glass menu-bar item over its panel, in the preview window,
            // which is debug-only (D-18).
            let menuPanel = MenuPanelStore(client: nil)
            menuPanel.attach(model.daemonData, configDirectory: model.configDirectory)
            await menuPanel.load()
            render(
                MenuBarPreviewWindow(store: menuPanel).environmentObject(model),
                to: directory + "/macos-shell-menu-bar.png",
                size: CGSize(width: 480, height: 760)
            )
            #endif
            // Every section is drawn alone, so none is the part of a long
            // stack that falls off the bottom of the image. The change log
            // keeps its historical file name: a log is a surface that can
            // only be checked by looking at it, small secondary text in two
            // columns being what fails contrast or collapses at width.
            // Compute has no section view of its own.
            for section in SettingsSection.allCases where section != .compute {
                let name = section == .changes ? "settings" : "settings-\(section.rawValue)"
                render(
                    GlassSettingsContent(section: section).environmentObject(model),
                    to: directory + "/macos-shell-\(name).png",
                    size: CGSize(width: 860, height: 620)
                )
            }
            // The withdrawal confirmation per tier, from the core's words.
            // Plain statuses suffice: the view takes a status, not a record.
            if let keep = model.publicRunCopy?.keepContribution {
                render(
                    VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
                        ForEach(["quarantined", "accepted", "withdrawn"], id: \.self) { status in
                            WithdrawalConfirmationView(
                                status: status, keepLabel: keep, inFlight: false, onKeep: {}, onConfirm: {})
                        }
                    }
                    .padding(GlassTokens.Space.cardGap)
                    .environmentObject(model),
                    to: directory + "/macos-shell-withdrawal.png",
                    size: CGSize(width: 860, height: 620)
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

    /// `TRACE_COMMONS_APPEARANCE` as a colour scheme: `ImageRenderer` has no
    /// window to inherit one from, so each capture is pinned to it.
    static let forcedColorScheme: ColorScheme? = {
        switch ProcessInfo.processInfo.environment["TRACE_COMMONS_APPEARANCE"] {
        case "dark": .dark
        case "light": .light
        default: nil
        }
    }()

    @MainActor
    private static func render<V: View>(_ view: V, to path: String, size: CGSize) {
        let renderer = ImageRenderer(
            content: view
                .frame(width: size.width, height: size.height)
                .background(Color(nsColor: .windowBackgroundColor))
                .environment(\.colorScheme, forcedColorScheme ?? .light)
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
