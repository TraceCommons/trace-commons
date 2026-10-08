import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// The glass monitor window (R5 of #1173): the three-pane shell the native
/// screens are built into. The main pane holds the tabs and is always shown;
/// the map and the inspector hide independently, each growing or shrinking
/// the window on its right (`GlassPaneLayout`), and the tab and every
/// preference are restored per window.
///
/// The main window since R15. Every label here is a single word or comes
/// from the Rust core (`ShellWordingTests`).
struct MonitorWindowView: View {
    /// The gate's button, which opens first run: the core's first-run
    /// Continue. With no table the word is empty, never a Swift fallback.
    static let openFirstRun = TCCoreCopy.firstRunCopyJSON().flatMap(FirstRunCopy.decode)?.frame.continueButton ?? ""
    enum Tab: String, CaseIterable, Identifiable {
        case home = "Home"
        case inference = "Inference"
        case traces = "Traces"

        /// The raw value is an identity (scene storage restores the tab by
        /// it); `title` is the word shown, from the core's shell table.
        var id: String { rawValue }

        var title: String {
            guard let shell = MonitorWords.table?.shell else { return "" }
            return switch self {
            case .home: shell.tabHome
            case .inference: shell.tabInference
            case .traces: shell.tabTraces
            }
        }

        /// The tabs the strip shows: all three once onboarding is done, and
        /// Inference alone before, where Private AI sign-in is (R-38).
        static func shown(requiresOnboarding: Bool) -> [Tab] {
            requiresOnboarding ? [.inference] : allCases
        }
    }

    /// The tab drawn for the restored one: Inference while onboarding is
    /// required, whatever was restored; the restored tab otherwise.
    static func shownTab(_ tab: Tab, requiresOnboarding: Bool) -> Tab {
        requiresOnboarding ? .inference : tab
    }

    enum MapTab: String {
        case traces
        case privateAI
    }

    /// The Traces tree's inset from the pane's edge (#1146 `px-2`).
    static let treeInset: CGFloat = 8

    /// Settings is drawn over the panes while `navigation.settingsRequest`
    /// is set (Ron's #1146 modal; #1241 Task 10), and an outside opener's
    /// destination waits there (`OpenMonitor`).
    let navigation: MainWindowNavigation
    /// The hosted Insights and Mission drafts screens' inputs (Home pages).
    let insightsStoreSelection: InsightsStoreSelection
    let missionDrafts: MissionDraftsModel

    @EnvironmentObject private var model: AppModel

    @SceneStorage("monitor.tab") private var tab: Tab = .home
    @SceneStorage("monitor.mapTab") private var mapTab: MapTab = .traces
    /// The person's preferences.
    @SceneStorage("monitor.showsMap") private var showsMap = true
    @SceneStorage("monitor.showsInspector") private var showsInspector = true
    /// The Traces graph footer (the toolbar's Graph), on by default as #1146.
    @SceneStorage("monitor.showsGraph") private var showsGraph = true
    /// The View menu's "Show ignored folders"; shown by default, as #1146
    /// (`traces-workspace.tsx`, `useState(true)`).
    @SceneStorage("monitor.showsIgnored") private var showsIgnored = true
    /// The binoculars: the map shows only the selected session's tool.
    @State private var mapFocus = false
    /// What asked for the inspector last time (`InspectorDemand`).
    @State private var lastDemand: Set<String> = []
    /// The Traces tree's selection, a folder or a session; nil for none.
    /// Restored per window, and read only through `TracesStore.resolve`, so
    /// one that has gone is the Summary, never a stale card.
    @SceneStorage("monitor.selection") private var selection: MonitorSelection?
    /// Home's page: the overview or History, restored per window.
    @SceneStorage("monitor.homePage") private var homePage: HomeTabView.Page = .overview
    /// The opened History row's submission id; empty for none. Its details
    /// are the inspector's while History is shown.
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

    /// Whether a nil client means the daemon has not started yet, rather
    /// than that it is down: only while start-up is still under way.
    /// Refused, waiting on its folders, or gone is down, and is said.
    static func awaitingDaemon(_ startup: AppModel.Startup) -> Bool {
        startup == .starting
    }

    /// What the stores are attached to: the live client, and whether the
    /// daemon is still starting. Both are the task's id, so the stores are
    /// re-attached when start-up ends without a client, and a daemon that
    /// never starts is said to be down rather than awaited for good.
    struct Attachment: Hashable {
        let live: ObjectIdentifier?
        let awaiting: Bool

        @MainActor
        init(_ model: AppModel) {
            live = model.liveData.map(ObjectIdentifier.init)
            awaiting = MonitorWindowView.awaitingDaemon(model.startup)
        }
    }

    #if DEBUG
    /// The set a `TRACE_COMMONS_SAMPLE` value names, and whether it named
    /// none (unset or empty is the default, not unknown). Debug builds
    /// only, with the sample sets.
    static func sampleChoice(_ name: String?) -> (set: SampleDaemonClient.SampleSet, unknown: Bool) {
        guard let name, !name.isEmpty else { return (.normalDay, false) }
        guard let set = SampleDaemonClient.SampleSet(rawValue: name) else { return (.normalDay, true) }
        return (set, false)
    }
    #endif
    /// False until this window has seeded the two preferences from the
    /// room its screen has (map from 1100pt, inspector from 900pt). After
    /// that the window restores whatever the person chose.
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
                showsMap: $showsMap, showsInspector: $showsInspector, showsGraph: $showsGraph,
                showsIgnored: $showsIgnored,
                breadcrumb: Self.breadcrumb(
                    tab: Self.shownTab(tab, requiresOnboarding: model.requiresOnboarding), homePage: homePage,
                    hosted: { HomeTabView.hostedHeading($0, missionDrafts: missionDrafts) },
                    back: { homePage = .overview }),
                onSettings: { navigation.requestSettings() }
            ) {
                switch Self.shownTab(tab, requiresOnboarding: model.requiresOnboarding) {
                case .traces:
                    TracesTreeView(
                        store: traces,
                        selection: Binding(
                            get: { selection },
                            set: { Self.select($0, selection: &selection, showsInspector: &showsInspector) })
                    ) { entryId in
                        Self.select(.session(entryID: entryId), selection: &selection, showsInspector: &showsInspector)
                    }
                    // #1146 insets the tree 8 from the pane's edge (`px-2`),
                    // closer than the other tabs' 12.
                    .padding(.horizontal, Self.treeInset - GlassTokens.Space.panePadding)
                case .inference: InferenceTabView(store: inference)
                case .home:
                    HomeTabView(
                        store: home, traces: traces,
                        statusLabel: { HomeFormat.historyStatusLabel(copy: model.publicRunCopy, $0) },
                        page: $homePage,
                        // Opening a row shows its details in the inspector,
                        // and opens it (Ron's inspector auto-open).
                        selection: Binding(
                            get: { selectedHistory },
                            set: { Self.openHistory($0, selected: &selectedHistory, showsInspector: &showsInspector) }),
                        openTraces: { tab = .traces },
                        insightsStoreSelection: insightsStoreSelection, missionDrafts: missionDrafts)
                }
            } footer: {
                // Shared over kept under the tree (#1146 `GraphFooter`).
                // Full bleed under its 0.5pt rule, at #1146's `px-3 py-2.5`,
                // sliding up from the pane's bottom edge as it opens.
                if Self.shownTab(tab, requiresOnboarding: model.requiresOnboarding) == .traces && showsGraph {
                    TracesGraphFooter(
                        history: home.history, sessions: traces.tree.allSessions, tool: selectedTool,
                        focus: $mapFocus, onFocus: focusMap)
                        .padding(.horizontal, GlassTokens.Space.panePadding)
                        .padding(.vertical, GlassTokens.Space.s5)
                        // #1146's fixed height, rule included (height 0 to
                        // 206 over .25s as it opens).
                        .frame(
                            maxWidth: .infinity, minHeight: TracesGraphFooter.height,
                            maxHeight: TracesGraphFooter.height, alignment: .top)
                        .overlay(alignment: .top) {
                            GlassHairline(GlassTokens.Color.rule.color)
                        }
                        .transition(.move(edge: .bottom).combined(with: .opacity))
                }
            }
        } map: {
            MonitorMapPane(
                mapTab: $mapTab, privateAILabel: model.privateInferenceCopy?.destination,
                privateAIDot: Self.inferenceDot(model.daemonSettings?.privateInferenceState?.surfaceState,
                                                calls: model.privateInferenceCalls),
                privateAIDescription: Self.inferenceDotDescription(
                    model.daemonSettings?.privateInferenceState?.surfaceState, calls: model.privateInferenceCalls),
                traces: traces, history: home.history, historyFailure: home.failures["list_history"], inference: inference,
                focusTool: mapFocus ? selectedTool?.rawValue : nil,
                selectedTool: selectedTool?.rawValue,
                sentence: { Self.rowSentence($0, copy: model.privateInferenceCopy, calls: model.harnessCalls) })
        } inspector: {
            // Ron's inspector pane insets its content 16 across and 18 down
            // (`monitor-shell.tsx:166`, `px-4 py-4.5`), wider than the
            // other panes' 12.
            GlassPane(insets: GlassPaneInsets.inspector) {
                // An empty branch would leave the pane nothing to draw, and
                // it would vanish while the layout still reserved its width.
                // While onboarding is required only Inference is shown, so
                // the Private AI inspector is the only one admitted (R-38).
                if !model.onboardingKnown {
                    Color.clear
                } else {
                    switch Self.shownTab(tab, requiresOnboarding: model.requiresOnboarding) {
                    // The prompts and the health banners are drawn above the
                    // Traces tree, not here (owner, 2026-10-07: offers, undo
                    // and health above the tree). Inference keeps its own
                    // inspector; Home, Traces and History host the
                    // selection's inspector.
                    case .inference:
                        PrivateAIInspectorView(store: inference, destinationLabel: model.privateInferenceCopy?.destination)
                    case .home, .traces:
                        // An opened History row is the inspector's selection
                        // while History is shown; otherwise the Traces
                        // selection's card, as Ron mounts `WaitingPage`.
                        if tab == .home,
                           let row = HistorySelection.opened(selectedHistory, onHistory: homePage == .history, in: home.history) {
                            HistoryInspectorPane(row: row)
                        } else {
                            TracesInspectorHost(traces: traces, home: home, selection: selection)
                        }
                    }
                }
            }
        }
        // Settings is a modal over all three panes (Ron's #1146): while it is
        // open the panes take no focus and no clicks, are hidden from
        // VoiceOver, and are blurred under its scrim.
        .disabled(navigation.settingsRequest != nil)
        .accessibilityHidden(navigation.settingsRequest != nil)
        .blur(radius: navigation.settingsRequest == nil ? 0 : GlassTokens.Size.modalScrimBlur)
        .overlay {
            if let request = navigation.settingsRequest {
                SettingsModal(
                    request: request, navigation: navigation, paused: traces.status?.paused,
                    onClose: { navigation.settingsRequest = nil },
                    onPrivateAI: { Self.openPrivateAI(tab: &tab, navigation: navigation) })
            }
        }
        .glassWindow()
        .onAppear { model.refreshAll() }
        // The app's own reads (settings, the tools, the credential, the
        // change log) refresh when someone looks, on this always-present
        // container, as the legacy window did.
        .onChange(of: navigation.requests, initial: true) { _, _ in
            // An outside opener's destination, consumed once; initially
            // too, for a request that opened this window. Each request,
            // not `pending`'s value: first run's hand-off asks again for
            // the destination already waiting.
            consumePending()
        }
        // A destination this window could not show yet (before the core
        // said) is taken once it can.
        .onChange(of: model.onboardingKnown) { _, _ in consumePending() }
        // Modals and confirmations raised anywhere in the window cover all
        // of it, Settings' sections' own included: the Settings modal sits
        // inside this host.
        .glassModalHost()
        // Every modal raised here has #1146's close button, named in the
        // core's words.
        .environment(\.glassModalCloseLabel, MonitorWords.table?.close ?? "")
        // Ron's `useInspectorDemand`: a key that was not there before (an
        // undo, a selected session, a folder's Submit all in flight)
        // opens the inspector, so none runs out of sight. A key going away
        // closes nothing. The offers are drawn above the tree, not here.
        .onChange(of: demandKeys) { _, current in
            if InspectorDemand.opens(previous: lastDemand, current: current) { showsInspector = true }
            lastDemand = current
        }
        .onAppear {
            // `onChange` sees changes only: a demand already there when the
            // window appears (an undo made while the Monitor was closed,
            // over a restored closed inspector) opens it too, as Ron's
            // effect does on mount.
            if InspectorDemand.opensOnAppear(keys: demandKeys) { showsInspector = true }
            lastDemand = demandKeys
            traces.showsIgnored = showsIgnored
        }
        .onChange(of: showsIgnored) { _, shows in traces.showsIgnored = shows }
        // #1146 `MonitorShell`: picking Inference shows the map's Private AI
        // view, picking Traces its Traces view; Home keeps the choice.
        .onChange(of: tab) { _, picked in
            if let view = Self.mapTab(for: picked) { mapTab = view }
        }
        // The app's live client, re-attached whenever the daemon restarts
        // or start-up ends; with none, each store draws the core as down,
        // except while the daemon is still starting, when it is loading.
        .task(id: Attachment(model)) {
            let attachment = Attachment(model)
            let client = Self.sampleClient() ?? model.daemonData
            traces.attach(client, awaiting: attachment.awaiting)
            let marker = Self.sampleMarker()
            traces.markSample(marker?.set, unknown: marker?.unknown ?? false)
            inference.attach(client, awaiting: attachment.awaiting)
            home.attach(client, awaiting: attachment.awaiting)
            async let a: () = traces.run()
            async let b: () = inference.run()
            async let c: () = home.run()
            _ = await (a, b, c)
        }
    }

    /// Lands the waiting destination, if this window can show it now
    /// (`LaunchRouting.monitorConsumes`). One it cannot (Home or Traces
    /// while onboarding is required) stays for first run's hand-off.
    private func consumePending() {
        guard let destination = navigation.pending,
              LaunchRouting.monitorConsumes(destination, requiresOnboarding: model.requiresOnboarding,
                                            onboardingKnown: model.onboardingKnown)
        else { return }
        Self.land(destination, tab: &tab, homePage: &homePage,
                  selection: &selection, showsInspector: &showsInspector)
        navigation.pending = nil
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

    /// What the inspector must show right now (`InspectorDemand`).
    private var demandKeys: Set<String> {
        InspectorDemand.keys(model: model, traces: traces, selection: selection)
    }

    /// The selection's tool: what the graph counts and the binoculars
    /// focus the map on. The tree has no tool level, so it is a session's
    /// own tool, or the tool a folder is drawn under on the map (#1146
    /// focuses a tool, a project or a session).
    private var selectedTool: SourceKind? {
        Self.selectedTool(traces.tree, session: traces.selectedSession(selection), folder: traces.selectedFolder(selection))
    }

    static func selectedTool(
        _ tree: TracesTree, session: DaemonData.QueueEntry?, folder: TracesTree.FolderNode?
    ) -> SourceKind? {
        if let entry = session {
            return SourceKind(rawValue: entry.declaredSource ?? entry.source) ?? SourceKind(rawValue: entry.source)
        }
        guard let folder else { return nil }
        return tree.tools.first { $0.folders.contains { $0.id == folder.id } }?.kind
            ?? TracesTree.majorityTool(folder.sessions)
    }

    /// The map view a tab picks (#1146 sets it on Inference and Traces);
    /// nil keeps the current one.
    static func mapTab(for picked: Tab) -> MapTab? {
        switch picked {
        case .inference: .privateAI
        case .traces: .traces
        case .home: nil
        }
    }

    /// The binoculars: focus the map on the selected tool, or back to the
    /// whole map. A hidden map is shown, on its Traces view.
    private func focusMap() {
        mapFocus.toggle()
        mapTab = .traces
        if !showsMap { showsMap = true }
    }

    /// The breadcrumb under the tabs (#1146 `MonitorTabs`): Home's History
    /// and Missions pages, with Home to go back to. Nil elsewhere. The
    /// hosted Insights and Mission drafts pages take the core's heading
    /// (`hosted`); without it the crumb ends at Home, never a word of this
    /// shell's own.
    static func breadcrumb(
        tab: Tab, homePage: HomeTabView.Page, hosted: (HomeTabView.Page) -> String? = { _ in nil },
        back: @escaping () -> Void
    ) -> [GlassCrumb]? {
        guard tab == .home else { return nil }
        switch homePage {
        case .overview: return nil
        case .history: return [GlassCrumb(Tab.home.title, action: back), GlassCrumb(MonitorWords.history)]
        // The commons catalogue: #1146's Missions is the drafts (hosted).
        case .missions:
            return [GlassCrumb(Tab.home.title, action: back)]
                + (MonitorWords.table.map { [GlassCrumb($0.homeHistory.missionCatalogue)] } ?? [])
        case .insights, .missionDrafts:
            return [GlassCrumb(Tab.home.title, action: back)] + (hosted(homePage).map { [GlassCrumb($0)] } ?? [])
        }
    }

    /// Where a destination lands in this window. A Settings destination
    /// opens the Settings modal (`Launcher`) and leaves the tabs alone.
    static func land(
        _ destination: MonitorDestination, tab: inout Tab, homePage: inout HomeTabView.Page,
        selection: inout MonitorSelection?, showsInspector: inout Bool
    ) {
        switch destination {
        case .home(let page):
            tab = .home
            homePage = page
        case .inference:
            // The account is on the main pane (Ron's #1146).
            tab = .inference
            showsInspector = true
        case .traces(let entryId):
            tab = .traces
            if let entryId { select(.session(entryID: entryId), selection: &selection, showsInspector: &showsInspector) }
        case .settings:
            break
        }
    }

    /// Opening a History row: its details are the inspector's, so the
    /// inspector opens (Ron's inspector auto-open). Clearing it closes
    /// nothing.
    static func openHistory(_ submissionId: String, selected: inout String, showsInspector: inout Bool) {
        selected = submissionId
        if !submissionId.isEmpty { showsInspector = true }
    }

    /// A tree selection. A session is a demand on the inspector, as in
    /// Ron's `useInspectorDemand`: selecting one shows the inspector its
    /// card lives in. A folder, or nothing, moves the selection only. The
    /// inspector never closes itself.
    static func select(_ wanted: MonitorSelection?, selection: inout MonitorSelection?, showsInspector: inout Bool) {
        selection = wanted
        if case .session = wanted { showsInspector = true }
    }

    /// The Settings modal's Private AI pointer: close the modal and open the
    /// Inference tab, as Ron's `navigate(routePaths["private-ai"])` does.
    static func openPrivateAI(tab: inout Tab, navigation: MainWindowNavigation) {
        navigation.settingsRequest = nil
        tab = .inference
    }

    /// The first time this window is shown, open the panes its screen has
    /// room for.
    private func seedPanes(windowWidth: CGFloat) {
        guard !panesSeeded else { return }
        let seed = GlassPaneLayout.firstLaunch(windowWidth: windowWidth)
        showsMap = seed.showsMap
        // A narrow first layout seeds the inspector closed, but never over
        // something it must show.
        showsInspector = seed.showsInspector || InspectorDemand.opensOnAppear(keys: demandKeys)
        panesSeeded = true
    }

    /// Inference's dot: what the listener is doing, from the daemon's own
    /// report, never the switch. The switch says what was asked for; a
    /// switch that is on over a listener that refused to start, or is held,
    /// is drawn as not working, never as on. Only the core's "clear" tone
    /// is on; every other is #1146's outside (red). No report, or an
    /// unreported state, is no dot: unknown is neither on nor off.
    static func inferenceDot(_ state: PrivateInferenceState?, calls: PrivateInferenceCalls) -> GlassStatus? {
        guard let state, !state.label.isEmpty else { return nil }
        return PrivateInferenceIndicator.dotStatus(PrivateInferenceSurface.tone(state, calls: calls))
    }

    /// The dot's text equivalent: the core's sentence for the same state.
    static func inferenceDotDescription(_ state: PrivateInferenceState?, calls: PrivateInferenceCalls) -> String? {
        guard let state, !state.label.isEmpty else { return nil }
        return calls.stateLine(state.label)
    }
}

/// The main pane, always shown: clearance for the traffic lights, the
/// toolbar capsule (View menu, Graph, Map, Inspector) and the round Settings
/// button on the top row (#1146 `monitor-toolbar.tsx`), then the tabs, the
/// breadcrumb for Home's pages, the tab's screen and, on Traces, the graph
/// footer. The tabs' screens are R6 (Traces), R8 (Inference) and R9 (Home).
private struct MonitorMainPane<Content: View, Footer: View>: View {
    @Binding var tab: MonitorWindowView.Tab
    let inferenceDot: GlassStatus?
    let inferenceDescription: String?
    let tracesBadge: GlassBadgeValue?
    /// Amber when something waiting is worth a second look.
    let tracesDot: GlassStatus?
    let tracesDescription: String?
    @Binding var showsMap: Bool
    @Binding var showsInspector: Bool
    @Binding var showsGraph: Bool
    @Binding var showsIgnored: Bool
    /// Home's History or Missions trail; nil for no breadcrumb.
    let breadcrumb: [GlassCrumb]?
    let onSettings: () -> Void
    @ViewBuilder let content: () -> Content
    @ViewBuilder let footer: () -> Footer
    @State private var viewMenu = false
    @EnvironmentObject private var model: AppModel
    /// Half the unified title bar's 52pt height.
    static var lightsCentre: CGFloat { 26 }

    /// The View menu's width (#1146 `monitor-toolbar.tsx`, `w-[250px]`).
    static var viewMenuWidth: CGFloat { 250 }
    /// The Settings button's gap from the toolbar capsule: #1146's `gap-2`
    /// plus `ml-1.5`.
    static var settingsGap: CGFloat { GlassTokens.Space.s4 + GlassTokens.Space.s3 }

    /// The tab's own insets in the pane (#1146 `monitor-shell.tsx`): the
    /// Traces tree runs 8pt from the pane's sides and to its bottom (the
    /// graph sits under it); the other tabs keep 12 on the sides and below.
    static func contentInsets(_ tab: MonitorWindowView.Tab) -> EdgeInsets {
        tab == .traces
            ? EdgeInsets(top: 0, leading: GlassTokens.Space.treeInset, bottom: 0, trailing: GlassTokens.Space.treeInset)
            : EdgeInsets(
                top: 0, leading: GlassTokens.Space.panePadding, bottom: GlassTokens.Space.panePadding,
                trailing: GlassTokens.Space.panePadding)
    }

    var body: some View {
        let shown = MonitorWindowView.shownTab(tab, requiresOnboarding: model.requiresOnboarding)
        // The pane draws edge to edge; each row takes #1146's own insets.
        GlassPane(padding: 0) {
            VStack(alignment: .leading, spacing: 0) {
                VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
                    HStack(spacing: Self.settingsGap) {
                        Spacer(minLength: 0)
                        GlassToolbarGroup {
                            GlassToolbarButton(MonitorShellWords.view, icon: .glyph(.viewMenu), expanded: viewMenu) {
                                viewMenu.toggle()
                            }
                            GlassToolbarButton(MonitorShellWords.graphToggle(shown: showsGraph), icon: .glyph(.graph), pressed: showsGraph) {
                                // #1146's footer opens and closes over .25s.
                                withAnimation(GlassMotion.systemReducesMotion ? nil : .easeInOut(duration: 0.25)) {
                                    showsGraph.toggle()
                                }
                            }
                            GlassToolbarButton(MonitorShellWords.mapToggle(shown: showsMap), icon: .glyph(.map), pressed: showsMap) {
                                showsMap.toggle()
                            }
                            GlassToolbarButton(MonitorShellWords.inspectorToggle(shown: showsInspector), icon: .glyph(.inspector), pressed: showsInspector) {
                                showsInspector.toggle()
                            }
                        }
                        // Open before onboarding too: Settings gates each section
                        // itself (R-43), so Connection, Startup, Notifications,
                        // Updates, Private AI and Compute are reachable, and a
                        // section that writes what first run asks draws the
                        // onboarding notice. #1146's 28pt round button and gear.
                        GlassRoundButton(MonitorWords.table?.settingsTitle ?? "", icon: .glyph(.gear), action: onSettings)
                    }
                    // Clearance for the real traffic lights, not an origin.
                    .padding(.leading, GlassTokens.Space.windowControlsWidth - GlassTokens.Space.panePadding)
                    .frame(height: GlassTokens.Size.controlLarge)
                    // Centre the row on the traffic lights, which the unified
                    // title bar centres 26pt below the window's top edge.
                    .padding(.top, Self.lightsCentre - GlassTokens.Space.windowPadding - GlassTokens.Space.panePadding
                        - GlassTokens.Size.controlLarge / 2)
                    // The View menu drops from the toolbar over the tabs, 250pt
                    // wide, its trailing edge on the Settings button's.
                    .overlay(alignment: .topTrailing) {
                        if viewMenu {
                            GlassMenu(onDismiss: { viewMenu = false }) {
                                GlassMenuItem(MonitorShellWords.showIgnoredFolders, checked: showsIgnored) {
                                    showsIgnored.toggle()
                                    viewMenu = false
                                }
                            }
                            .frame(width: Self.viewMenuWidth)
                            .fixedSize(horizontal: false, vertical: true)
                            .padding(.top, GlassTokens.Size.controlLarge + GlassTokens.Space.s2)
                        }
                    }
                    .zIndex(1)
                    // The same notices the main window puts above everything,
                    // here in the pane that is always shown, so a void or a gate
                    // hold during monitor use is told whatever the map and the
                    // inspector are doing.
                    ShellNotices()
                    if gate != .awaiting {
                        switch gate {
                        case .down(let sentence):
                            StartupRefusedBanner(sentence: sentence)
                        case .signedOut:
                            GlassNotice(tone: .ask, title: MonitorWords.signedOut) {
                                Button(MonitorWindowView.openFirstRun) { OpenMonitor.request() }
                            }
                        case .awaiting, .open:
                            EmptyView()
                        }
                        GlassSegmentedTabs(
                            MonitorWords.table?.shell.tabsLabel ?? "",
                            selection: Binding(
                                get: { MonitorWindowView.shownTab(tab, requiresOnboarding: model.requiresOnboarding) },
                                set: { tab = $0 }),
                            segments: MonitorWindowView.Tab.shown(requiresOnboarding: model.requiresOnboarding).map { item in
                                GlassSegment(
                                    item.title, value: item,
                                    badgeValue: item == .traces ? tracesBadge : nil,
                                    dot: item == .inference ? inferenceDot : item == .traces ? tracesDot : nil,
                                    accessibilityValue: item == .inference
                                        ? inferenceDescription : item == .traces ? tracesDescription : nil)
                            })
                    }
                }
                .padding([.horizontal, .top], GlassTokens.Space.panePadding)
                .zIndex(1)
                if gate == .awaiting {
                    SettingsAwaiting()
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                        .padding(GlassTokens.Space.panePadding)
                } else {
                    // 8pt under the tabs and under the breadcrumb (#1146
                    // `mb-2`, `pb-2`).
                    if let breadcrumb {
                        GlassBreadcrumb(breadcrumb, backLabel: MonitorWords.table?.shell.backToHome ?? MonitorWindowView.Tab.home.title,
                                        onBack: breadcrumb.first?.action)
                            .padding(.horizontal, GlassTokens.Space.panePadding)
                            .padding(.top, GlassTokens.Space.s4)
                    }
                    content()
                        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
                        .padding(.top, GlassTokens.Space.s4)
                        .padding(Self.contentInsets(shown))
                    // Full bleed under the tree (#1146 `GraphFooter`).
                    footer()
                }
            }
            // The graph opens and closes over #1146's .25s.
            .animation(GlassMotion.systemReducesMotion ? nil : GlassMotion.curve(GlassTokens.Motion.slide), value: showsGraph)
        }
    }

    /// No Home or Traces before onboarding is done: their screens act on
    /// consent that has not been given. Inference stays, so Private AI
    /// sign-in is reachable before Commons enrollment (R-38); its own
    /// startup handling (roots, starting, refused) is its gate. The button
    /// opens first run, which is where every other request goes until then.
    /// Before the core says, the placeholder status is not "signed out", and
    /// a daemon that refused to start is said as a refusal, not as "signed
    /// out", whoever is at the keyboard (`MonitorGate`).
    private var gate: MonitorGate {
        MonitorGate.of(
            startup: model.startup, onboardingKnown: model.onboardingKnown,
            requiresOnboarding: model.requiresOnboarding)
    }
}

/// The map (R8): the field, the flow map drawn on it, and the view
/// selector floating at its upper trailing edge (spec, "Flow map"). The
/// selector changes the map's view only, never the main tab, consent or
/// routing, and keeps its choice while the map is hidden.
private struct MonitorMapPane: View {
    @Binding var mapTab: MonitorWindowView.MapTab
    let privateAILabel: String?
    /// The Private AI segment's status dot (#1146 `FlowMap`): the same dot
    /// the Inference tab carries; none while the core has not said.
    let privateAIDot: GlassStatus?
    /// The dot's text equivalent, the core's sentence for the same state.
    let privateAIDescription: String?
    let traces: TracesStore
    /// History's rows, for what each node says was contributed; nil while
    /// unread, and then the cards say a dash, never none.
    let history: [DaemonData.HistoryRow]?
    /// Why the last `list_history` failed, if it did: the page above is
    /// then the last good one, not current.
    let historyFailure: DaemonDataError?
    let inference: InferenceStore
    /// The tool the binoculars focus the Traces view on; nil for all.
    let focusTool: String?
    /// The selection's tool: the map fades the others and rings it.
    let selectedTool: String?
    /// The core's sentence for a tool's Private AI state.
    let sentence: (HarnessRow) -> String?
    @EnvironmentObject private var model: AppModel

    var body: some View {
        // The map is content, not chrome: its own field, so the selector,
        // zoom and node cards floating on it are its only glass (Apple: no
        // glass on glass; R14).
        // #1146's map field and map edge, its view tabs 14pt in.
        GlassPane(padding: 0, isContent: true, edge: GlassTokens.Shadow.mapEdge) {
            ZStack(alignment: .topTrailing) {
                GlassMapField()
                map
                GlassFloatingGroup {
                    GlassSegmentedTabs(MonitorWords.table?.shell.mapViewsLabel ?? "", selection: $mapTab, segments: segments, floating: true)
                        .padding(GlassTokens.Space.mapOverlayInset)
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
            let scene = FlowMapScene.traces(
                traces.tree,
                gate: .init(state: tracesState, status: traces.status, destinations: traces.destinations),
                contributed: .init(history: history, failure: historyFailure), selectedTool: selectedTool)
            FlowMapView(
                scene: scene,
                legend: [.autoUpload, .ask, .ignore], zoomable: true,
                accessibilityName: FlowMapScene.words?.mapLabel ?? MonitorWindowView.Tab.traces.title, state: tracesState,
                focus: scene.focusPoint(tool: focusTool))
        case .privateAI:
            // The one tool list, while this window's client has read it and
            // the core still answers; otherwise nothing is drawn as current.
            let harnesses = PrivateAIInspectorView.liveHarnesses(
                model.harnesses, read: inference.harnesses, failure: inference.failures["harness_list"])
            if harnesses != .none, let privateAILabel {
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
            segments.append(GlassSegment(
                privateAILabel, value: .privateAI, dot: privateAIDot, accessibilityValue: privateAIDescription))
        }
        return segments
    }
}

/// The name the Settings scene mounted (D8; R11 of #1173). No scene mounts
/// it since the R15 cutover took Ron's modal; it is kept so anything that
/// still names it routes to the modal. Settings is now Ron's modal over the Monitor
/// (#1241 Task 10, `SettingsModal`), so this draws nothing of its own: it
/// opens the Monitor, asks it for the modal, and closes itself.
struct MonitorSettingsWindow: View {
    let navigation: MainWindowNavigation

    @Environment(\.openWindow) private var openWindow
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        Color.clear
            .frame(width: 1, height: 1)
            .onAppear {
                openWindow(id: WindowID.monitor)
                navigation.requestSettings()
                dismiss()
            }
    }
}
