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

        /// The raw value is an identity (scene storage restores the tab by
        /// it); `title` is the word shown, marked for localisation.
        var id: String { rawValue }

        var title: String {
            switch self {
            case .home: String(localized: "Home", comment: "Monitor tab")
            case .inference: String(localized: "Inference", comment: "Monitor tab")
            case .traces: String(localized: "Traces", comment: "Monitor tab")
            }
        }
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

    /// The screens' data (C1). Sample data in this debug window until K1
    /// moves the screens to the live client: `TRACE_COMMONS_SAMPLE` names
    /// the set (`normalDay` by default).
    @State private var traces = MonitorWindowView.tracesStore()
    /// The map's Private AI view and the Inference tab (R8).
    @State private var inference = InferenceStore(client: MonitorWindowView.dataClient())

    /// The Traces store over the sample set `TRACE_COMMONS_SAMPLE` names. A
    /// name that is not a set falls back to `normalDay`, and says so: the tab
    /// marks the data as sample, and an unknown name both in the marker and
    /// in the log.
    static func tracesStore() -> TracesStore {
        let choice = sampleChoice(ProcessInfo.processInfo.environment["TRACE_COMMONS_SAMPLE"])
        if choice.unknown {
            NSLog("TRACE_COMMONS_SAMPLE unrecognised; fallback %@", choice.set.rawValue)
        }
        return TracesStore(
            client: DaemonDataWiring.sample(choice.set), sample: choice.set.rawValue, sampleUnknown: choice.unknown)
    }

    /// The sample client for the window's other stores, over the same set.
    static func dataClient() -> any DaemonDataClient {
        DaemonDataWiring.sample(sampleChoice(ProcessInfo.processInfo.environment["TRACE_COMMONS_SAMPLE"]).set)
    }

    /// The set a `TRACE_COMMONS_SAMPLE` value names, and whether it named
    /// none (unset or empty is the default, not unknown).
    static func sampleChoice(_ name: String?) -> (set: SampleDaemonClient.SampleSet, unknown: Bool) {
        guard let name, !name.isEmpty else { return (.normalDay, false) }
        guard let set = SampleDaemonClient.SampleSet(rawValue: name) else { return (.normalDay, true) }
        return (set, false)
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
                tracesDot: traces.shield == .attention ? .ask : nil,
                tracesDescription: Self.tracesDescription(
                    traces.decisionsOwed, shield: traces.shield, secondLook: traces.words?.secondLookWaiting),
                showsMap: $showsMap, showsInspector: $showsInspector,
                onSettings: { openSettings() }
            ) {
                switch tab {
                case .traces:
                    TracesTreeView(store: traces, selection: $selectedSession) { entryId in
                        Self.review(entryId, selection: &selectedSession, showsInspector: &showsInspector)
                    }
                case .inference: InferenceTabView(store: inference)
                case .home: Spacer(minLength: 0)
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
                    Color.clear
                }
            }
        }
        .glassWindow()
        .task { await traces.run() }
        .task { await inference.run() }
    }

    /// The Traces badge (R7): decisions owed, a dash when the core did not
    /// say, nothing at zero. Never queue depth.
    private var tracesBadge: GlassBadgeValue? {
        traces.decisionsOwed.map(GlassBadgeValue.count) ?? .unknown
    }

    /// The Traces badge's text equivalent, from the core: "unavailable" for
    /// an unknown count, never zero; nil at zero, where there is no badge.
    /// When something waiting is worth a second look, the core's words for
    /// that follow, as the badge's amber dot shows it.
    static func tracesDescription(
        _ decisionsOwed: Int?, shield: QueueShieldState = .clear, secondLook: String? = nil
    ) -> String? {
        let count = TCCoreCopy.decisionsOwedText(decisionsOwed).flatMap { $0.isEmpty ? nil : $0 }
        let flagged = shield == .attention ? secondLook : nil
        let parts = [count, flagged].compactMap { $0 }
        return parts.isEmpty ? nil : parts.joined(separator: ", ")
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
    /// Amber when something waiting is worth a second look.
    let tracesDot: GlassStatus?
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
                        GlassToolbarButton(String(localized: "Map", comment: "Map pane toggle"), systemImage: "map", pressed: showsMap && !mapCompacted) {
                            showsMap.toggle()
                        }
                        .disabled(mapCompacted)
                        GlassToolbarButton(String(localized: "Inspector", comment: "Inspector pane toggle"), systemImage: "sidebar.right", pressed: showsInspector) {
                            showsInspector.toggle()
                        }
                    }
                    GlassRoundButton(String(localized: "Settings", comment: "Settings button"), systemImage: "gearshape", small: true, action: onSettings)
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
                    String(localized: "Monitor", comment: "Monitor tabs name"),
                    selection: $tab,
                    segments: MonitorWindowView.Tab.allCases.map { item in
                        GlassSegment(
                            item.title, value: item,
                            badgeValue: item == .traces ? tracesBadge : nil,
                            dot: item == .inference ? inferenceDot : item == .traces ? tracesDot : nil,
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
        GlassPane(padding: 0) {
            ZStack(alignment: .topTrailing) {
                RadialGradient(
                    colors: [GlassTokens.Color.mapFieldInner.color, GlassTokens.Color.mapFieldOuter.color],
                    center: .center, startRadius: 20, endRadius: 520)
                map
                GlassFloatingGroup {
                    GlassSegmentedTabs(String(localized: "Map", comment: "Map view selector name"), selection: $mapTab, segments: segments, floating: true)
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
                accessibilityName: MonitorWindowView.Tab.traces.title, state: tracesState)
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
        var segments = [GlassSegment(MonitorWindowView.Tab.traces.title, value: MonitorWindowView.MapTab.traces)]
        if let privateAILabel {
            segments.append(GlassSegment(privateAILabel, value: .privateAI))
        }
        return segments
    }
}

/// The Settings window (D8): the existing settings, in a macOS Settings
/// window opened with ⌘,. Its theming may later move to the glass system to
/// match the monitor window (#1173).
struct MonitorSettingsWindow: View {
    let navigation: MainWindowNavigation

    var body: some View {
        SettingsView(navigation: navigation)
            .frame(minWidth: 620, minHeight: 520)
    }
}
#endif
