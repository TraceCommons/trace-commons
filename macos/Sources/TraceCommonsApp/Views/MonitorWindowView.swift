#if DEBUG
import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// The glass monitor window (R5 of #1173): the three-pane shell the native
/// screens are built into. The main pane holds the tabs and is always shown;
/// the map and the inspector hide independently (and the map below 1100pt),
/// and the tab and both preferences are restored per window.
///
/// Debug builds only, until the screens it frames (R6 onward) match the
/// design. The shipping window stays `MainWindowView` until then (R15).
/// Every label here is a single word or comes from the Rust core
/// (`ShellWordingTests`).
struct MonitorWindowView: View {
    enum Tab: String, CaseIterable, Identifiable {
        case home = "Home"
        case inference = "Inference"
        case traces = "Traces"

        var id: String { rawValue }
    }

    enum MapTab: String {
        case traces
        case privateAI
    }

    @EnvironmentObject private var model: AppModel
    @Environment(\.openSettings) private var openSettings

    @SceneStorage("monitor.tab") private var tab: Tab = .home
    @SceneStorage("monitor.mapTab") private var mapTab: MapTab = .traces
    /// The person's preferences. The map is also hidden below 1100pt
    /// without touching this, so widening the window brings it back.
    @SceneStorage("monitor.showsMap") private var showsMap = true
    @SceneStorage("monitor.showsInspector") private var showsInspector = true
    /// The selected session's entry id; empty for none.
    @SceneStorage("monitor.selectedSession") private var selectedSession = ""
    /// Home's page: the overview or History, restored per window.
    @SceneStorage("monitor.homePage") private var homePage: HomeTabView.Page = .overview

    /// The screens' data (C1). Sample data in this debug window until K1
    /// moves the screens to the live client: `TRACE_COMMONS_SAMPLE` names
    /// the set (`normalDay` by default).
    @State private var traces = TracesStore(client: MonitorWindowView.dataClient())
    /// The map's Private AI view and the Inference tab (R8).
    @State private var inference = InferenceStore(client: MonitorWindowView.dataClient())
    /// Home and History (R9).
    @State private var home = HomeStore(client: MonitorWindowView.dataClient())

    static func dataClient() -> any DaemonDataClient {
        let name = ProcessInfo.processInfo.environment["TRACE_COMMONS_SAMPLE"] ?? ""
        return DaemonDataWiring.sample(SampleDaemonClient.SampleSet(rawValue: name) ?? .normalDay)
    }
    /// False until this window has seeded the two preferences from its
    /// width (map from 1100pt, inspector from 900pt). After that the
    /// window restores whatever the person chose.
    @SceneStorage("monitor.panesSeeded") private var panesSeeded = false

    var body: some View {
        GlassThreePane(showsMap: showsMap, showsInspector: showsInspector, onFirstLayout: seedPanes) {
            MonitorMainPane(
                tab: $tab,
                inferenceDot: Self.inferenceDot(model.daemonSettings?.privateInferenceState?.surfaceState,
                                                calls: model.privateInferenceCalls),
                inferenceDescription: Self.inferenceDotDescription(
                    model.daemonSettings?.privateInferenceState?.surfaceState,
                    calls: model.privateInferenceCalls),
                tracesBadge: tracesBadge,
                tracesDescription: Self.tracesDescription(traces.decisionsOwed),
                showsMap: $showsMap, showsInspector: $showsInspector,
                onSettings: { openSettings() }
            ) {
                switch tab {
                case .traces:
                    TracesTreeView(store: traces, selection: $selectedSession) { entryId in
                        Self.review(entryId, selection: &selectedSession, showsInspector: &showsInspector)
                    }
                case .inference: InferenceTabView(store: inference)
                case .home:
                    HomeTabView(
                        store: home, traces: traces,
                        statusLabel: { status in model.publicRunCopy?.contributionStatusLabel(for: status) },
                        page: $homePage)
                }
            }
        } map: {
            MonitorMapPane(
                mapTab: $mapTab, privateAILabel: model.privateInferenceCopy?.destination,
                traces: traces, inference: inference,
                sentence: { HarnessSurface.stateSentence($0, calls: model.harnessCalls) })
        } inspector: {
            GlassPane {
                // An empty branch would leave the pane nothing to draw, and
                // it would vanish while the layout still reserved its width.
                switch tab {
                case .traces:
                    SessionInspectorView(store: traces, entry: selectedEntry)
                case .inference:
                    PrivateAIInspectorView(
                        store: inference, destinationLabel: model.privateInferenceCopy?.destination,
                        sentence: { HarnessSurface.stateSentence($0, calls: model.harnessCalls) })
                case .home:
                    HomeSummaryInspector(store: home)
                }
            }
        }
        .glassWindow()
        .task { await traces.run() }
        .task { await inference.run() }
        .task { await home.run() }
    }

    /// The Traces badge (R7): decisions owed, a dash when the core did not
    /// say, nothing at zero. Never queue depth.
    private var tracesBadge: GlassBadgeValue? {
        traces.decisionsOwed.map(GlassBadgeValue.count) ?? .unknown
    }

    /// The Traces badge's text equivalent, from the core: "unavailable" for
    /// an unknown count, never zero; nil at zero, where there is no badge.
    static func tracesDescription(_ decisionsOwed: Int?) -> String? {
        guard let text = TCCoreCopy.decisionsOwedText(decisionsOwed), !text.isEmpty else { return nil }
        return text
    }

    /// The selected session, while it is still in the tree.
    private var selectedEntry: DaemonData.QueueEntry? {
        traces.tree.allSessions.first { $0.entryId == selectedSession }
    }

    /// A session's Review: select it and show the inspector, where its
    /// review is. With the inspector hidden, selecting alone did nothing a
    /// person could see.
    static func review(_ entryId: String, selection: inout String, showsInspector: inout Bool) {
        selection = entryId
        showsInspector = true
    }

    /// The first time this window lays out, open the panes its width suits.
    private func seedPanes(windowWidth: CGFloat) {
        guard !panesSeeded else { return }
        let seed = GlassPaneLayout.firstLaunch(windowWidth: windowWidth)
        showsMap = seed.showsMap
        showsInspector = seed.showsInspector
        panesSeeded = true
    }

    /// Inference's dot: what the listener is doing, from the daemon's own
    /// report, never the switch. The switch says what was asked for; a
    /// switch that is on over a listener that refused to start, or is held,
    /// is drawn as needing attention, not as on. Only the core's "clear"
    /// tone is on. No report, or an unreported state, is no dot: unknown is
    /// neither on nor off.
    static func inferenceDot(_ state: PrivateInferenceState?, calls: PrivateInferenceCalls) -> GlassStatus? {
        guard let state, !state.label.isEmpty else { return nil }
        switch PrivateInferenceSurface.tone(state, calls: calls) {
        case .clear: return .on
        case .held, .attention, .refused: return .ask
        case .neutral: return .off
        }
    }

    /// The dot's text equivalent: the core's sentence for the same state.
    static func inferenceDotDescription(_ state: PrivateInferenceState?, calls: PrivateInferenceCalls) -> String? {
        guard let state, !state.label.isEmpty else { return nil }
        return calls.stateLine(state.label)
    }
}

/// The main pane, always shown: clearance for the traffic lights, the
/// toolbar capsule (map and inspector toggles) and the round Settings
/// button on the top row, then the tabs. The tabs' screens are R6 (Traces),
/// R8 (Inference) and R9 (Home).
private struct MonitorMainPane<Content: View>: View {
    @Binding var tab: MonitorWindowView.Tab
    let inferenceDot: GlassStatus?
    let inferenceDescription: String?
    let tracesBadge: GlassBadgeValue?
    let tracesDescription: String?
    @Binding var showsMap: Bool
    @Binding var showsInspector: Bool
    let onSettings: () -> Void
    @ViewBuilder let content: () -> Content
    /// The window is too narrow for the map: the toggle shows it hidden and
    /// cannot show it, and widening the window brings back the preference.
    @Environment(\.glassMapCompacted) private var mapCompacted
    /// Half the unified title bar's 52pt height.
    static var lightsCentre: CGFloat { 26 }

    var body: some View {
        GlassPane {
            VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
                HStack(spacing: GlassTokens.Space.s4) {
                    Spacer(minLength: 0)
                    GlassToolbarGroup {
                        GlassToolbarButton("Map", systemImage: "map", pressed: showsMap && !mapCompacted) {
                            showsMap.toggle()
                        }
                        .disabled(mapCompacted)
                        GlassToolbarButton("Inspector", systemImage: "sidebar.right", pressed: showsInspector) {
                            showsInspector.toggle()
                        }
                    }
                    GlassRoundButton("Settings", systemImage: "gearshape", small: true, action: onSettings)
                }
                // Clearance for the real traffic lights, not an origin.
                .padding(.leading, GlassTokens.Space.windowControlsWidth - GlassTokens.Space.panePadding)
                .frame(height: GlassTokens.Size.controlLarge)
                // Centre the row on the traffic lights, which the unified
                // title bar centres 26pt below the window's top edge.
                .padding(.top, Self.lightsCentre - GlassTokens.Space.windowPadding - GlassTokens.Space.panePadding
                    - GlassTokens.Size.controlLarge / 2)
                // The same notices the main window puts above everything,
                // here in the pane that is always shown, so a void or a gate
                // hold during monitor use is told whatever the map and the
                // inspector are doing.
                ShellNotices()
                GlassSegmentedTabs(
                    "Monitor",
                    selection: $tab,
                    segments: MonitorWindowView.Tab.allCases.map { item in
                        GlassSegment(
                            item.rawValue, value: item,
                            badgeValue: item == .traces ? tracesBadge : nil,
                            dot: item == .inference ? inferenceDot : nil,
                            accessibilityValue: item == .inference
                                ? inferenceDescription : item == .traces ? tracesDescription : nil)
                    })
                content()
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
            }
        }
    }
}

/// The map (R8): the field, the flow map drawn on it, and the view
/// selector floating at its upper trailing edge (spec, "Flow map"). The
/// selector changes the map's view only, never the main tab, consent or
/// routing, and keeps its choice while the map is hidden.
private struct MonitorMapPane: View {
    @Binding var mapTab: MonitorWindowView.MapTab
    let privateAILabel: String?
    let traces: TracesStore
    let inference: InferenceStore
    /// The core's sentence for a tool's Private AI state.
    let sentence: (HarnessRow) -> String?
    @EnvironmentObject private var model: AppModel

    var body: some View {
        // The map is content, not chrome: an opaque pane, so the selector,
        // zoom and node cards floating on it are its only glass (Apple: no
        // glass on glass; R14).
        GlassPane(padding: 0, isContent: true) {
            ZStack(alignment: .topTrailing) {
                RadialGradient(
                    colors: [GlassTokens.Color.mapFieldInner.color, GlassTokens.Color.mapFieldOuter.color],
                    center: .center, startRadius: 20, endRadius: 520)
                map
                GlassFloatingGroup {
                    GlassSegmentedTabs("Map", selection: $mapTab, segments: segments, floating: true)
                        .padding(GlassTokens.Space.panePadding)
                }
                stateLine
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .bottomLeading)
            }
        }
    }

    @ViewBuilder
    private var map: some View {
        switch shownTab {
        case .traces:
            FlowMapView(
                scene: .traces(traces.tree), legend: [.autoUpload, .ask, .ignore], zoomable: true,
                accessibilityName: MonitorWindowView.Tab.traces.rawValue, state: tracesState)
        case .privateAI:
            if let harnesses = inference.harnesses, let privateAILabel {
                FlowMapView(
                    scene: .privateAI(
                        harnesses, destinationLabel: privateAILabel, sentence: sentence,
                        answering: { HarnessSurface.state($0, calls: model.harnessCalls) == .answering }),
                    legend: [], zoomable: false, accessibilityName: privateAILabel)
            } else {
                Color.clear
            }
        }
    }

    /// The Traces map's state (the stack-wide ScreenState rule): core down
    /// over the last tree, loading before the first, paused or unknown when
    /// the core's status says so or says nothing.
    private var tracesState: ScreenState {
        var failure: DaemonDataError?
        if case .failed(let error) = traces.phase { failure = error }
        return ScreenState.resolve(
            failure: failure, loaded: traces.phase != .loading,
            paused: traces.status?.paused, known: traces.status != nil)
    }

    /// The core's line when the map's state is not current, else nothing.
    @ViewBuilder
    private var stateLine: some View {
        if case .failed(let error) = traces.phase, shownTab == .traces {
            GlassFloatingGroup {
                Text(MonitorWords.table?.line(for: error) ?? "")
                    .glassType(GlassTokens.TypeScale.label)
                    .foregroundStyle(GlassColor.textPrimary)
                    .padding(.horizontal, GlassTokens.Space.s6)
                    .padding(.vertical, GlassTokens.Space.s3)
                    .glassSurface(.nodeCard)
            }
            .padding(GlassTokens.Space.panePadding)
        }
    }

    /// The Private AI view needs the core's name for it; until the core
    /// has said it, the map stays on Traces without changing the choice.
    private var shownTab: MonitorWindowView.MapTab {
        privateAILabel == nil ? .traces : mapTab
    }

    /// Traces, and the Private AI tab once the Rust core has said what it is
    /// called (D11).
    private var segments: [GlassSegment<MonitorWindowView.MapTab>] {
        var segments = [GlassSegment(MonitorWindowView.Tab.traces.rawValue, value: MonitorWindowView.MapTab.traces)]
        if let privateAILabel {
            segments.append(GlassSegment(privateAILabel, value: .privateAI))
        }
        return segments
    }
}

/// The Settings window (D8; R11 of #1173): the section list and, beside
/// it, the selected section alone, each scrolling on its own (spec,
/// "Settings navigation"). The sections are the existing settings, with
/// their behaviour and the core's copy unchanged; the list only chooses
/// which one is drawn. The selection is restored, and opening or closing
/// this window leaves the monitor window as it was.
struct MonitorSettingsWindow: View {
    let navigation: MainWindowNavigation

    @EnvironmentObject private var model: AppModel
    @Environment(ComputeModel.self) private var compute
    @SceneStorage("settings.section") private var section: SettingsSection = .connection

    var body: some View {
        NavigationSplitView {
            // One list with arrow-key selection, not a button per row.
            List(selection: Binding(get: { section }, set: { if let value = $0 { section = value } })) {
                ForEach(SettingsSection.allCases) { item in
                    if let title = item.title(model: model, compute: compute.snapshot?.title) {
                        Label(title, systemImage: item.symbol)
                            .lineLimit(2)
                            .tag(item)
                    }
                }
            }
            .navigationSplitViewColumnWidth(min: 200, ideal: 230, max: 280)
        } detail: {
            Group {
                switch section {
                case .compute:
                    ComputeView(model: compute)
                default:
                    ScrollView {
                        SettingsContent(navigation: navigation, section: section)
                    }
                    .tcScreen()
                }
            }
            // A fresh view per section, so the scroll starts at its top.
            .id(section)
        }
        .frame(minWidth: 760, minHeight: 520)
    }
}
#endif
