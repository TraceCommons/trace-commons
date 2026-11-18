import AppKit
import Foundation
import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

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

    /// True only while `captureWindows` draws the Monitor window from a
    /// recorded sample set: `AppModel` then reads onboarding as known and
    /// done, so the window draws the sample's tabs rather than this
    /// machine's start-up state. Debug builds only; false in release.
    static var capturesSampleWindows: Bool {
        #if DEBUG
        renderingSampleWindows
        #else
        false
        #endif
    }

    #if DEBUG
    /// Set and cleared by `captureWindows` around its renders, on the main
    /// actor that both it and every reader run on.
    nonisolated(unsafe) static var renderingSampleWindows = false
    #endif

    @MainActor
    static func scheduleIfRequested(model: AppModel) {
        guard let directory else { return }
        #if DEBUG
        if let choice = sampleChoice {
            Task { @MainActor in
                // Only the window's own first layout to wait for: the
                // stores are loaded from the sample set before each render.
                try? await Task.sleep(nanoseconds: 2_000_000_000)
                await captureWindows(set: choice, model: model, to: directory)
                await quitIfAsked(model: model)
            }
            return
        }
        #endif
        Task { @MainActor in
            // Late enough that the watcher has polled, queued, and scrubbed.
            try? await Task.sleep(nanoseconds: 12_000_000_000)

            // The Monitor's screens. Each store is loaded first, since the
            // window's attaching `task` never runs under `ImageRenderer`.
            let traces = TracesStore(client: nil)
            traces.attach(model.daemonData)
            await traces.load()
            // The first session selected, as Ron's inspector shows it.
            let first = traces.tree.allSessions.first.map { MonitorSelection.session(entryID: $0.entryId) }
            let home = HomeStore(client: nil)
            home.attach(model.daemonData)
            await home.load()
            render(
                TracesTreeView(store: traces, selection: .constant(first), missionsShown: model.missionsShown)
                    .environmentObject(model),
                to: directory + "/macos-shell-traces-tree.png",
                size: CGSize(width: 760, height: 720)
            )
            render(
                TracesInspectorHost(traces: traces, home: home, selection: first)
                    .padding(GlassTokens.Space.cardGap)
                    .environmentObject(model),
                to: directory + "/macos-shell-session-inspector.png",
                size: CGSize(width: 420, height: 720)
            )
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
                VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
                    InferenceBalanceSection()
                    InferenceAccountSection(store: inference)
                }
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
                MenuBarPreviewWindow(navigation: MainWindowNavigation(), store: menuPanel).environmentObject(model),
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
                // Ron's tick, in the core's words, as Look inside draws it.
                let lookInside = MonitorTracesCopy.decode(fromJSON: TCCoreCopy.monitorTracesCopyJSON())?.lookInside
                render(
                    WitnessReviewConsent(
                        copy: copy, confirmLine: lookInside?.witnessConfirmLine,
                        confirmLabel: lookInside?.witnessConfirmLabel, onCancel: {}, onConfirm: {}),
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
            // The edge rail open at each peek. Its switch is in the Startup
            // section's capture.
            // The stores loaded above stand in for the rail's own, whose
            // attaching task never runs here.
            let railData = EdgeRailData(traces: traces, inference: inference, home: home)
            for peek in EdgeRailPeek.allCases {
                let state = EdgeRailState()
                state.open = true
                state.peek = peek
                render(
                    EdgeRailView(state: state, data: railData, request: { _, _ in })
                        .environmentObject(model),
                    to: directory + "/macos-shell-edge-rail-\(peek).png",
                    size: CGSize(width: EdgeRailGeometry.openWidth, height: GlassTokens.Size.edgeRailOpenHeight)
                )
            }
            await quitIfAsked(model: model)
        }
    }

    @MainActor
    private static func quitIfAsked(model: AppModel) async {
        if ProcessInfo.processInfo.environment["TRACE_COMMONS_QUIT_AFTER_SHOT"] == "1" {
            // Late enough that the self-test, which starts on the same
            // clock, has finished writing.
            try? await Task.sleep(nanoseconds: 25_000_000_000)
            model.shutdown()
            NSApp.terminate(nil)
        }
    }

    #if DEBUG
    /// The sample set `TRACE_COMMONS_SAMPLE` names, with the window's own
    /// fallback (`MonitorWindowView.sampleChoice`); nil when it is unset.
    @MainActor
    static var sampleChoice: SampleDaemonClient.SampleSet? {
        guard let name = ProcessInfo.processInfo.environment["TRACE_COMMONS_SAMPLE"] else { return nil }
        return MonitorWindowView.sampleChoice(name).set
    }

    /// One whole-window capture: a tab and which of the map and the
    /// inspector are shown beside the main pane.
    struct WindowShot: Equatable {
        let tab: MonitorWindowView.Tab
        let showsMap: Bool
        let showsInspector: Bool

        /// The panes token of the shared capture name: `main`, then `map`
        /// and `inspector` when shown, joined by `+`.
        var panes: String {
            (["main"] + (showsMap ? ["map"] : []) + (showsInspector ? ["inspector"] : [])).joined(separator: "+")
        }

        /// `<set>-<tab>-<panes>-<appearance>.png`, the name
        /// `scripts/parity/gallery.py` pairs a Linux capture by.
        func fileName(set: SampleDaemonClient.SampleSet, appearance: String) -> String {
            "\(set.rawValue)-\(tab.rawValue.lowercased())-\(panes)-\(appearance).png"
        }

        /// The window's width with these panes at their default widths.
        var width: CGFloat { GlassPaneLayout.windowWidth(showsMap: showsMap, showsInspector: showsInspector) }
    }

    /// Every tab in the default order, each with the map hidden and shown,
    /// and with the inspector too wherever the tab's first page has one
    /// (`MonitorWindowView.inspectorAvailable`): only Traces. A combination
    /// that would draw the same window as another is not captured twice.
    @MainActor
    static func windowShots() -> [WindowShot] {
        MonitorWindowView.Tab.ordered("").flatMap { tab in
            let inspector = MonitorWindowView.inspectorAvailable(tab, homePage: .overview)
            return [false, true].flatMap { map in
                (inspector ? [false, true] : [false]).map { WindowShot(tab: tab, showsMap: map, showsInspector: $0) }
            }
        }
    }

    /// `TRACE_COMMONS_APPEARANCE`'s word in a capture's name.
    static var appearanceName: String { forcedColorScheme == .dark ? "dark" : "light" }

    /// The whole Monitor window, once per `windowShots()`, drawn from the
    /// sample set alone: the stores read `SampleDaemonClient`, never the
    /// live client, so no capture holds anything from this machine's
    /// queue. The synthetic-preview rule above still holds: no preview,
    /// real or recorded, is opened here.
    @MainActor
    static func captureWindows(set: SampleDaemonClient.SampleSet, model: AppModel, to directory: String) async {
        let client = DaemonDataWiring.sample(set)
        // An empty Insights store of this run's own, so Insights draws no
        // local data of the person's.
        let insightsDirectory = FileManager.default.temporaryDirectory
            .appendingPathComponent("trace-commons-capture-insights-\(UUID().uuidString)")
        try? FileManager.default.createDirectory(at: insightsDirectory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: insightsDirectory) }
        for shot in windowShots() {
            let traces = TracesStore(client: nil)
            traces.attach(client)
            traces.markSample(set.rawValue, unknown: false)
            let home = HomeStore(client: nil)
            home.attach(client)
            let inference = InferenceStore(client: nil)
            inference.attach(client)
            await traces.load()
            await home.load()
            await inference.load()
            // The panes' switches, in a suite of this capture's own.
            let suiteName = "trace-commons-capture-\(UUID().uuidString)"
            guard let defaults = UserDefaults(suiteName: suiteName) else { continue }
            defaults.set(shot.showsMap, forKey: "monitor.showsMap")
            defaults.set(shot.showsInspector, forKey: "monitor.showsInspector")
            defaults.set(true, forKey: "monitor.panesSeeded")
            let capture = MonitorWindowView.Capture(
                tab: shot.tab, mapTab: MonitorWindowView.mapTab(for: shot.tab) ?? .traces,
                traces: traces, home: home, inference: inference,
                insightsStore: .custom(insightsDirectory.path))
            // Only around the synchronous render, so the live app never
            // reads onboarding as done or its glass as painted in between.
            renderingSampleWindows = true
            GlassMaterial.paintsForCapture = true
            defer {
                renderingSampleWindows = false
                GlassMaterial.paintsForCapture = false
            }
            render(
                MonitorWindowView(capture: capture, navigation: MainWindowNavigation(), missionDrafts: MissionDraftsModel())
                    .environmentObject(model)
                    .defaultAppStorage(defaults),
                to: directory + "/" + shot.fileName(set: set, appearance: appearanceName),
                size: CGSize(width: shot.width, height: GlassTokens.Size.windowHeight)
            )
            defaults.removePersistentDomain(forName: suiteName)
        }
    }
    #endif

    /// `TRACE_COMMONS_APPEARANCE` as a colour scheme: `ImageRenderer` has no
    /// window to inherit one from, so each capture is pinned to it. Unset,
    /// the captures draw light (`render`): with no window there is no
    /// system appearance to follow, and capture runs always set it.
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
