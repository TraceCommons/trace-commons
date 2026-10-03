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

    /// Where an outside opener asked this window to go (`OpenMonitor`).
    let navigation: MainWindowNavigation

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
    /// The selected History row's submission id; empty for none.
    @SceneStorage("monitor.selectedHistory") private var selectedHistory = ""

    /// The screens' data, read through the app's live client
    /// (`AppModel.daemonData`), which the body attaches whenever the daemon
    /// starts or restarts. Until then each store says the core is down.
    @State private var traces = TracesStore(client: nil)
    /// The map's Private AI view and the Inference tab (R8).
    @State private var inference = InferenceStore(client: nil)
    /// Home and History (R9).
    @State private var home = HomeStore(client: nil)

    /// Sample data, debug builds only, when `TRACE_COMMONS_SAMPLE` names a
    /// set; nil otherwise, and then the live client is attached below. A
    /// name that is not a set falls back to `normalDay`, and says so in the
    /// log and in the Traces tab's sample marker.
    static func sampleClient() -> (any DaemonDataClient)? {
        #if DEBUG
        let choice = sampleChoice(ProcessInfo.processInfo.environment["TRACE_COMMONS_SAMPLE"])
        guard ProcessInfo.processInfo.environment["TRACE_COMMONS_SAMPLE"] != nil else { return nil }
        if choice.unknown { NSLog("TRACE_COMMONS_SAMPLE unrecognised; fallback %@", choice.set.rawValue) }
        let set = choice.set
        return DaemonDataWiring.sample(set)
        #else
        return nil
        #endif
    }

    /// The Traces tab's sample marker for `sampleClient()`'s set: its name,
    /// and whether `TRACE_COMMONS_SAMPLE` named no set. Nil over the daemon.
    static func sampleMarker() -> (set: String, unknown: Bool)? {
        #if DEBUG
        guard let name = ProcessInfo.processInfo.environment["TRACE_COMMONS_SAMPLE"] else { return nil }
        let choice = sampleChoice(name)
        return (choice.set.rawValue, choice.unknown)
        #else
        return nil
        #endif
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
                case .home:
                    HomeTabView(
                        store: home, traces: traces,
                        statusLabel: { HomeFormat.historyStatusLabel(copy: model.publicRunCopy, $0) },
                        page: $homePage,
                        // Selecting a row shows the inspector, where its
                        // details are, as a session's Review does.
                        selection: Binding(
                            get: { selectedHistory },
                            set: { Self.review($0, selection: &selectedHistory, showsInspector: &showsInspector) }))
                }
            }
        } map: {
            MonitorMapPane(
                mapTab: $mapTab, privateAILabel: model.privateInferenceCopy?.destination,
                traces: traces, inference: inference,
                sentence: { Self.rowSentence($0, copy: model.privateInferenceCopy, calls: model.harnessCalls) })
        } inspector: {
            GlassPane {
                // An empty branch would leave the pane nothing to draw, and
                // it would vanish while the layout still reserved its width.
                if !LaunchRouting.onboardingKnown(startup: model.startup, statusAnswered: model.status.answered)
                    || model.requiresOnboarding {
                    Color.clear
                } else {
                    switch tab {
                    case .traces:
                        SessionInspectorView(store: traces, entry: selectedEntry)
                    case .inference:
                        PrivateAIInspectorView(store: inference, destinationLabel: model.privateInferenceCopy?.destination)
                    case .home:
                        // History's selected row, while it is still listed; the
                        // record as a whole otherwise.
                        if homePage == .history, let row = selectedHistoryRow {
                            HistoryDetailInspector(row: row)
                        } else {
                            HomeSummaryInspector(store: home)
                        }
                    }
                }
            }
        }
        .glassWindow()
        .onChange(of: navigation.pending, initial: true) { _, destination in
            // An outside opener's destination, consumed once; initially
            // too, for a request that opened this window.
            guard let destination else { return }
            Self.land(destination, tab: &tab, homePage: &homePage,
                      selectedSession: &selectedSession, showsInspector: &showsInspector)
            navigation.pending = nil
        }
        // The app's live client, re-attached whenever the daemon restarts;
        // with none, each store draws the core as down.
        .task(id: model.liveData.map(ObjectIdentifier.init)) {
            let client = Self.sampleClient() ?? model.daemonData
            traces.attach(client)
            let marker = Self.sampleMarker()
            traces.markSample(marker?.set, unknown: marker?.unknown ?? false)
            inference.attach(client)
            home.attach(client)
            async let a: () = traces.run()
            async let b: () = inference.run()
            async let c: () = home.run()
            _ = await (a, b, c)
        }
    }

    /// The sentence one tool's row shows (`HarnessSurface.rowSentence`): a
    /// tool that is not on this Mac gets the missing-tool sentence, never
    /// the not-connected one. Without the core's Private AI copy a missing
    /// tool says nothing rather than the wrong sentence.
    static func rowSentence(_ row: HarnessRow, copy: PrivateInferenceCopy?, calls: HarnessCalls) -> String? {
        guard let copy else { return row.installed ? HarnessSurface.stateSentence(row, calls: calls) : nil }
        return HarnessSurface.rowSentence(row, copy: copy, calls: calls)
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

    /// The selected History row, while it is still in the list.
    private var selectedHistoryRow: DaemonData.HistoryRow? {
        guard !selectedHistory.isEmpty else { return nil }
        return home.history?.first { $0.submissionId == selectedHistory }
    }

    /// A session's Review: select it and show the inspector, where its
    /// review is. With the inspector hidden, selecting alone did nothing a
    /// person could see. Selecting a History row goes the same way.
    static func review(_ entryId: String, selection: inout String, showsInspector: inout Bool) {
        selection = entryId
        showsInspector = true
    }

    /// Where a destination lands in this window. A Settings destination
    /// opens the Settings window (`Launcher`) and leaves the tabs alone.
    static func land(
        _ destination: MonitorDestination, tab: inout Tab, homePage: inout HomeTabView.Page,
        selectedSession: inout String, showsInspector: inout Bool
    ) {
        switch destination {
        case .home(let page):
            tab = .home
            homePage = page
        case .inference:
            // The Private AI switch and sign-in are in the inspector.
            tab = .inference
            showsInspector = true
        case .traces(let entryId):
            tab = .traces
            if let entryId { review(entryId, selection: &selectedSession, showsInspector: &showsInspector) }
        case .settings:
            break
        }
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
        return PrivateInferenceIndicator.status(PrivateInferenceSurface.tone(state, calls: calls))
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
    @EnvironmentObject private var model: AppModel
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
                // No tab before onboarding is done: its screens act on
                // consent that has not been given. The button opens first
                // run, which is where every request goes until then. Before
                // the core says, the placeholder status is not "signed out".
                if !LaunchRouting.onboardingKnown(startup: model.startup, statusAnswered: model.status.answered) {
                    SettingsAwaiting()
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                } else if model.requiresOnboarding {
                    GlassNotice(tone: .ask, title: MonitorWords.signedOut) {
                        Button(OnboardingWelcomeWords.getStarted) { OpenMonitor.request() }
                    }
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
                } else {
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
                scene: .traces(traces.tree, gate: .init(state: tracesState, status: traces.status, destinations: traces.destinations)), legend: [.autoUpload, .ask, .ignore], zoomable: true,
                accessibilityName: MonitorWindowView.Tab.traces.title, state: tracesState)
        case .privateAI:
            if let harnesses = inference.harnesses, let privateAILabel {
                FlowMapView(
                    scene: .privateAI(
                        harnesses, destinationLabel: privateAILabel,
                        privateAI: traces.destinations?.privateAi ?? traces.status?.privateInferenceState?.state,
                        sentence: sentence,
                        state: { HarnessSurface.state($0, calls: model.harnessCalls) }),
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
                    // A section whose copy has not loaded is a disabled
                    // placeholder, never a missing row.
                    let row = item.listRow(.init(model: model, compute: compute.snapshot?.title))
                    Label(row.text, systemImage: item.symbol)
                        .lineLimit(2)
                        .foregroundStyle(row.enabled ? .primary : .secondary)
                        .accessibilityLabel(row.enabled ? row.text : MonitorWords.unknown)
                        .selectionDisabled(!row.enabled)
                        .tag(item)
                }
            }
            .navigationSplitViewColumnWidth(min: 200, ideal: 230, max: 280)
        } detail: {
            Group {
                switch section {
                case .compute:
                    ScrollView {
                        ComputeView(model: compute)
                            .padding(GlassTokens.Space.panePadding)
                            .frame(maxWidth: 560, alignment: .leading)
                            .frame(maxWidth: .infinity, alignment: .topLeading)
                    }
                default:
                    ScrollView {
                        GlassSettingsContent(section: section)
                    }
                }
            }
            // A fresh view per section, so the scroll starts at its top.
            .id(section)
        }
        .frame(minWidth: 760, minHeight: 520)
        // A request for a section (a quit refusal's Compute), consumed
        // once, whether or not this window was already open.
        .onChange(of: navigation.settingsSection, initial: true) { _, wanted in
            guard let wanted else { return }
            section = wanted
            navigation.settingsSection = nil
        }
    }
}
#endif
