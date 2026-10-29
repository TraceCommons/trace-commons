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
    /// Opens the Settings window from the gear.
    @Environment(\.openWindow) private var openWindow
    /// The gate's button, which opens first run: the core's first-run
    /// Continue. With no table the word is empty, never a Swift fallback.
    static let openFirstRun = TCCoreCopy.firstRunCopyJSON().flatMap(FirstRunCopy.decode)?.frame.continueButton ?? ""
    enum Tab: String, CaseIterable, Identifiable {
        case home = "Home"
        /// Traces second and Private AI third (owner, 2026-10-28: the
        /// macOS MVP is "contribute traces, earn credit").
        case traces = "Traces"
        case inference = "Inference"
        /// Insights' own tab (2026-10-15), no longer a Home page. Last, so
        /// the first three keep their shortcuts.
        case insights = "Insights"

        /// The raw value is an identity (scene storage restores the tab by
        /// it); `title` is the word shown, from the core's shell table.
        var id: String { rawValue }

        var title: String {
            guard let shell = MonitorWords.table?.shell else { return "" }
            return switch self {
            case .home: shell.tabHome
            case .traces: shell.tabTraces
            case .inference: shell.tabInference
            case .insights: HomeTabView.insightsHeading ?? ""
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

    /// The NEAR AI payout rail's streams for Insights' NEAR AI card, read
    /// off the main actor. Any refusal (no account session, no rail, a
    /// daemon that predates streams) reads as none, so the card is drawn
    /// as before.
    static func payoutStreams(_ client: DaemonClient?) -> (@Sendable () async -> [PayoutStreamStatus]?)? {
        client.map { client in
            { @Sendable in
                await DaemonWork.run {
                    (try? client.payoutLink.status()).flatMap { PayoutLinkSurface.nearAiRail(in: $0)?.streams }
                }
            }
        }
    }

    enum MapTab: String {
        case traces
        case privateAI
    }

    /// The main pane's side inset: the inspector's (owner, 2026-10-10), so
    /// the tabs, the notices, the tree and every tab's cards sit as far in
    /// from the pane's sides as the inspector's content does.
    nonisolated static let mainInset: CGFloat = GlassTokens.Space.inspectorPaddingHorizontal

    /// Where the gear asks for Settings, which opens in its own window
    /// (owner, 2026-10-08), and where an outside opener's destination waits
    /// (`OpenMonitor`).
    let navigation: MainWindowNavigation
    /// The hosted Insights and Mission drafts screens' inputs (Home pages).
    let insightsStoreSelection: InsightsStoreSelection
    let missionDrafts: MissionDraftsModel

    @EnvironmentObject private var model: AppModel

    @SceneStorage("monitor.tab") private var tab: Tab = .home
    @SceneStorage("monitor.mapTab") private var mapTab: MapTab = .traces
    /// The person's preferences, kept in their defaults so every launch
    /// opens with the panes as they left them (owner, 2026-10-19), however
    /// the app last quit; and the map's width, as they last left it.
    @AppStorage("monitor.showsMap") private var showsMap = true
    @AppStorage("monitor.showsInspector") private var showsInspector = true
    @AppStorage("monitor.mapWidth") private var mapWidth = Double(GlassTokens.Size.mapWidth)
    /// The main pane's width beside the map, as the divider was last left
    /// (owner, 2026-10-20).
    @AppStorage("monitor.mainWidth") private var mainWidth = Double(GlassPaneLayout.mainWidth)
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
    /// The Traces tab's page: the tree, or History (owner, 2026-10-12).
    @SceneStorage("monitor.tracesPage") private var tracesPage: TracesPage = .tree
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
    @AppStorage("monitor.panesSeeded") private var panesSeeded = false
    /// The traces moving across the Traces map and the trace its cards
    /// describe, at the map's foot and the Traces tab's top (owner,
    /// 2026-10-19), fed by `MapTransferFeed`.
    @StateObject private var transfers = MapTransferPlayer()
    /// The popover notices closed with their X until this window opens
    /// again (`MonitorNotices`).
    @State private var closedNotices: Set<String> = []

    var body: some View {
        GlassThreePane(
            showsMap: showsMap, showsInspector: showsInspector && inspectorAvailable, mapWidth: $mapWidth,
            mainWidth: $mainWidth,
            onFirstLayout: seedPanes
        ) {
            MonitorMainPane(
                tab: $tab,
                inferenceDot: Self.inferenceDot(model.daemonSettings?.privateInferenceState?.surfaceState,
                                                calls: model.privateInferenceCalls),
                inferenceDescription: Self.inferenceDotDescription(
                    model.daemonSettings?.privateInferenceState?.surfaceState,
                    calls: model.privateInferenceCalls),
                tracesBadge: tracesBadge,
                tracesDescription: Self.tracesDescription(traces.decisionsOwed),
                showsMap: $showsMap, showsInspector: $showsInspector, inspectorAvailable: inspectorAvailable,
                showsGraph: $showsGraph,
                breadcrumb: Self.breadcrumb(
                    tab: Self.shownTab(tab, requiresOnboarding: model.requiresOnboarding),
                    homePage: homePage.shown(missions: model.missionsShown),
                    tracesPage: tracesPage,
                    hosted: { HomeTabView.hostedHeading($0, missionDrafts: missionDrafts) },
                    back: { homePage = .overview }, backToTraces: { tracesPage = .tree }),
                onSettings: {
                    navigation.requestSettings()
                    openWindow(id: WindowID.settings)
                }
            ) {
                switch Self.shownTab(tab, requiresOnboarding: model.requiresOnboarding) {
                case .traces:
                    switch tracesPage {
                    case .tree:
                        // Equatable: a pass of this window for anything else
                        // (its own state, the app model) does not redraw the
                        // tree's rows.
                        TracesTreeView(
                            store: traces,
                            selection: Binding(
                                get: { selection },
                                set: { Self.select($0, selection: &selection, showsInspector: &showsInspector) }),
                            missionsShown: model.missionsShown,
                            onReview: { entryId in
                                Self.select(.session(entryID: entryId), selection: &selection, showsInspector: &showsInspector)
                            },
                            openHistory: { tracesPage = .history })
                            .equatable()
                    case .history:
                        // Opening a row shows its details in the inspector,
                        // and opens it (Ron's inspector auto-open).
                        TracesHistoryPage(
                            store: home,
                            statusLabel: { HomeFormat.historyStatusLabel(copy: model.publicRunCopy, $0) },
                            selection: Binding(
                                get: { selectedHistory },
                                set: { Self.openHistory($0, selected: &selectedHistory, showsInspector: &showsInspector) }),
                            back: { tracesPage = .tree })
                    }
                case .inference:
                    InferenceTabView(store: inference) {
                        navigation.requestSettings(at: .privateAI)
                        openWindow(id: WindowID.settings)
                    }
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
                        openPrivateAI: { tab = .inference },
                        openHistory: {
                            tab = .traces
                            tracesPage = .history
                        },
                        openInsights: { tab = .insights },
                        insightsStoreSelection: insightsStoreSelection, missionDrafts: missionDrafts,
                        missionsEnabled: model.missionsShown, inference: inference)
                case .insights:
                    InsightsView(storeSelection: insightsStoreSelection, daemon: home.client,
                                 subscriptions: model.subscriptionUsageModel, home: home,
                                 payoutStreams: Self.payoutStreams(model.payoutLinkDaemonClient),
                                 openTraces: {
                                     tab = .traces
                                     tracesPage = .tree
                                 })
                }
            } footer: {
                // Shared over kept under the tree (#1146 `GraphFooter`).
                // Full bleed under its 0.5pt rule, at #1146's `px-3 py-2.5`,
                // sliding up from the pane's bottom edge as it opens.
                if Self.shownTab(tab, requiresOnboarding: model.requiresOnboarding) == .traces && showsGraph {
                    TracesGraphFooter(
                        history: TracesGraphFooter.readable(home.history, failure: home.failures["list_history"]),
                        complete: TracesGraphFooter.countable(home.history, failure: home.failures["list_history"]) != nil,
                        sessions: traces.tree.allSessions, tool: selectedTool,
                        focus: $mapFocus, onFocus: focusMap)
                        .padding(.horizontal, MonitorWindowView.mainInset)
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
            } notices: {
                // Every notice, as popover notices over the top of the tab
                // they belong to (owner, 2026-10-19). The traces moving now
                // are Traces' while it is shown; the map carries them only
                // while another tab is.
                MonitorNotices(
                    place: MonitorNotices.place(
                        shownTab, tracesPage: tracesPage, homePage: homePage.shown(missions: model.missionsShown)),
                    traces: traces, home: home,
                    transferCards: transfers.cards, onCloseTransfer: { transfers.close($0) },
                    closed: $closedNotices)
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
                sentence: { Self.rowSentence($0, copy: model.privateInferenceCopy, calls: model.harnessCalls) },
                // A folder clicked on the map is the tree's selection and the
                // inspector's subject, on Traces (owner, 2026-10-09).
                onSelectFolder: { projectID in
                    tab = .traces
                    selection = .folder(projectID: projectID)
                    showsInspector = true
                },
                transfers: transfers,
                showsTransferCards: shownTab != .traces)
        } inspector: {
            // The whole column scrolls as one, prompts and selection together:
            // unscrolled, a column taller than the window grew the inspector
            // past it and pushed the other panes down.
            GlassPane(padding: 0) {
                ScrollView {
                    inspectorContent
                        // Ron's inspector pane insets its content 16 across and 18
                        // down (`monitor-shell.tsx:166`, `px-4 py-4.5`), wider than
                        // the other panes' 12.
                        .padding(GlassPaneInsets.inspector)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                .scrollIndicators(.never)
            }
        }
        // What History gained, kept and dismissed, for the map's and the
        // Traces tab's trace cards (owner, 2026-10-19).
        .modifier(MapTransferFeed(player: transfers, history: home.history, traces: traces))
        // Settings is its own window (owner, 2026-10-08,
        // `SettingsWindowView`), no longer a modal over these panes.
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
        // Restored scene state cannot hold a mission page open while
        // missions are hidden.
        .onChange(of: model.missionsShown, initial: true) { _, shown in
            homePage = homePage.shown(missions: shown)
        }
        // The sheet for sent traces without Train coding models, when the
        // Insights line's Review or the first run raised it.
        .modifier(SentTracesTrainingPresenter(site: .monitor))
        // Modals and confirmations raised anywhere in the window cover all
        // of it, Settings' sections' own included: the Settings modal sits
        // inside this host.
        .glassModalHost()
        // Every modal raised here has #1146's close button, named in the
        // core's words.
        .environment(\.glassModalCloseLabel, MonitorWords.table?.close ?? "")
        // A folder's tile mark, in the core's words.
        .environment(\.glassFolderMark, TracesTreeWords.table?.folderMark ?? "")
        // Ron's `useInspectorDemand`: a key that was not there before (an
        // undo, a selected session, a folder's Submit all in flight)
        // opens the inspector, so none runs out of sight. A key going away
        // closes nothing. Beside the tree the offers are no demand.
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
        // The Traces list's filter menu sets it on the store (owner,
        // 2026-10-19); the window keeps the choice.
        .onChange(of: traces.showsIgnored) { _, shows in if showsIgnored != shows { showsIgnored = shows } }
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
        Self.land(destination, tab: &tab, homePage: &homePage, tracesPage: &tracesPage,
                  selection: &selection, showsInspector: &showsInspector, missionsEnabled: model.missionsShown)
        navigation.pending = nil
        if destination == .idleSessions {
            // Held by the store until the window's first attach when the
            // window has only just opened.
            Task { await traces.requestIdleOnly() }
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
    static func tracesDescription(_ decisionsOwed: Int?) -> String? {
        TCCoreCopy.decisionsOwedText(decisionsOwed).flatMap { $0.isEmpty ? nil : $0 }
    }

    /// The inspector column for the shown tab, unpadded and unscrolled.
    @ViewBuilder
    private var inspectorContent: some View {
        // An empty branch would leave the pane nothing to draw, and
        // it would vanish while the layout still reserved its width.
        // The inspector opens only on Traces and on Home's History
        // (`inspectorAvailable`); on Inference and Home's other pages it
        // stays closed and draws nothing (owner, 2026-10-08 and 10-09).
        // While onboarding is required only Inference is shown (R-38), so
        // nothing is drawn then either.
        if !model.onboardingKnown || !inspectorAvailable {
            Color.clear
        } else {
            VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
                // On Traces the prompts and the health banners are drawn
                // above the tree, not here (owner, 2026-10-07: offers,
                // undo and health above the tree). History draws them at
                // the top of the inspector, as Ron's shell mounts
                // `WaitingPrompts`: it still offers the Traces selection's
                // Contribute.
                if Self.promptsInInspector(shownTab, tracesPage: tracesPage) {
                    InspectorPromptsHeader(traces: traces)
                }
                // An opened History row is the inspector's selection while
                // History is shown; otherwise the Traces selection's card,
                // as Ron mounts `WaitingPage`.
                if let row = HistorySelection.opened(selectedHistory, onHistory: historyShown, in: home.history) {
                    HistoryInspectorPane(row: row)
                } else {
                    TracesInspectorHost(traces: traces, home: home, selection: selection)
                }
            }
        }
    }

    /// Whether the inspector draws the prompts (and, beside the host, the
    /// health banners): only on Home's History, restored from before
    /// History moved to Traces, where it still offers the Traces
    /// selection's Contribute. Everywhere else they are popover notices
    /// over the tab they belong to (owner, 2026-10-19, `MonitorNotices`):
    /// Traces' tree and its History page alike.
    static func promptsInInspector(_ tab: Tab, tracesPage: TracesPage = .tree) -> Bool {
        tab == .home
    }

    /// Whether History is the page on screen: on Traces now, or on Home
    /// for a window restored to it from before the move.
    static func showsHistory(_ tab: Tab, homePage: HomeTabView.Page, tracesPage: TracesPage) -> Bool {
        (tab == .traces && tracesPage == .history) || (tab == .home && homePage == .history)
    }

    /// Whether the inspector may open on what is shown: on Traces, and on
    /// Home's History, whose opened row is the inspector's. Never on Home's
    /// other pages (owner, 2026-10-08), Insights or Inference (2026-10-09):
    /// they have nothing for it to show, and their prompts head their page.
    static func inspectorAvailable(_ shown: Tab, homePage: HomeTabView.Page) -> Bool {
        switch shown {
        case .traces: true
        case .home: homePage == .history
        case .insights, .inference: false
        }
    }

    /// Whether the toolbar's Graph toggle works: only on Traces, the tab
    /// the graph is drawn under (owner, 2026-10-19).
    static func graphAvailable(_ tab: Tab) -> Bool { tab == .traces }

    /// History is the page on screen (`showsHistory`).
    private var historyShown: Bool {
        Self.showsHistory(shownTab, homePage: homePage, tracesPage: tracesPage)
    }

    private var inspectorAvailable: Bool {
        Self.inspectorAvailable(shownTab, homePage: homePage)
    }

    /// What the inspector must show right now (`InspectorDemand`); the
    /// offers only where the inspector draws them.
    private var demandKeys: Set<String> {
        InspectorDemand.keys(model: model, traces: traces, selection: selection)
            // Offers ask for the inspector only where it draws them and
            // may open: on Home's page they head the main pane.
            .union(Self.promptsInInspector(shownTab, tracesPage: tracesPage) && inspectorAvailable
                ? InspectorDemand.offerKeys(model: model) : [])
    }

    /// The tab on screen (`shownTab(_:requiresOnboarding:)`).
    private var shownTab: Tab { Self.shownTab(tab, requiresOnboarding: model.requiresOnboarding) }

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
        case .home, .insights: nil
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
        tab: Tab, homePage: HomeTabView.Page, tracesPage: TracesPage = .tree,
        hosted: (HomeTabView.Page) -> String? = { _ in nil },
        back: @escaping () -> Void, backToTraces: @escaping () -> Void = {}
    ) -> [GlassCrumb]? {
        // History on Traces: Traces to go back to (owner, 2026-10-12).
        if tab == .traces, tracesPage == .history {
            return [GlassCrumb(Tab.traces.title, action: backToTraces), GlassCrumb(MonitorWords.history)]
        }
        guard tab == .home else { return nil }
        switch homePage {
        case .overview: return nil
        case .history: return [GlassCrumb(Tab.home.title, action: back), GlassCrumb(MonitorWords.history)]
        // The commons catalogue: #1146's Missions is the drafts (hosted).
        case .missions:
            return [GlassCrumb(Tab.home.title, action: back)]
                + (MonitorWords.table.map { [GlassCrumb($0.homeHistory.missionCatalogue)] } ?? [])
        case .missionDrafts:
            return [GlassCrumb(Tab.home.title, action: back)] + (hosted(homePage).map { [GlassCrumb($0)] } ?? [])
        }
    }

    /// Where a destination lands in this window. A Settings destination
    /// opens the Settings modal (`Launcher`) and leaves the tabs alone.
    static func land(
        _ destination: MonitorDestination, tab: inout Tab, homePage: inout HomeTabView.Page,
        selection: inout MonitorSelection?, showsInspector: inout Bool, missionsEnabled: Bool = false
    ) {
        var tracesPage = TracesPage.tree
        land(destination, tab: &tab, homePage: &homePage, tracesPage: &tracesPage,
             selection: &selection, showsInspector: &showsInspector, missionsEnabled: missionsEnabled)
    }

    static func land(
        _ destination: MonitorDestination, tab: inout Tab, homePage: inout HomeTabView.Page,
        tracesPage: inout TracesPage, selection: inout MonitorSelection?, showsInspector: inout Bool,
        missionsEnabled: Bool = false
    ) {
        switch destination {
        case .home(.history):
            // History lives on Traces now (owner, 2026-10-12).
            tab = .traces
            tracesPage = .history
        case .home(let page):
            tab = .home
            // A mission page falls back to the overview while missions are
            // hidden.
            homePage = page.shown(missions: missionsEnabled)
        case .insights:
            tab = .insights
        case .inference:
            // The account is on the main pane (Ron's #1146), and the
            // inspector stays closed on Inference (owner, 2026-10-09).
            tab = .inference
        case .traces(let entryId):
            tab = .traces
            tracesPage = .tree
            if let entryId { select(.session(entryID: entryId), selection: &selection, showsInspector: &showsInspector) }
        case .idleSessions:
            // The store narrows the list (`consumePending`).
            tab = .traces
            tracesPage = .tree
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
    nonisolated static func select(_ wanted: MonitorSelection?, selection: inout MonitorSelection?, showsInspector: inout Bool) {
        selection = wanted
        if case .session = wanted { showsInspector = true }
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
    nonisolated static func inferenceDot(_ state: PrivateInferenceState?, calls: PrivateInferenceCalls) -> GlassStatus? {
        guard let state, !state.label.isEmpty else { return nil }
        return PrivateInferenceIndicator.dotStatus(PrivateInferenceSurface.tone(state, calls: calls))
    }

    /// The dot's text equivalent: the core's sentence for the same state.
    nonisolated static func inferenceDotDescription(_ state: PrivateInferenceState?, calls: PrivateInferenceCalls) -> String? {
        guard let state, !state.label.isEmpty else { return nil }
        return calls.stateLine(state.label)
    }
}

/// The main pane, always shown: clearance for the traffic lights, the
/// toolbar capsule (Graph, Map, Inspector) and the round Settings
/// button on the top row (#1146 `monitor-toolbar.tsx`), then the tabs, the
/// breadcrumb for Home's pages, the tab's screen and, on Traces, the graph
/// footer. The tabs' screens are R6 (Traces), R8 (Inference) and R9 (Home).
private struct MonitorMainPane<Content: View, Footer: View, Notices: View>: View {
    @Binding var tab: MonitorWindowView.Tab
    let inferenceDot: GlassStatus?
    let inferenceDescription: String?
    let tracesBadge: GlassBadgeValue?
    let tracesDescription: String?
    @Binding var showsMap: Bool
    @Binding var showsInspector: Bool
    /// False where the inspector cannot open (Home but for History): its
    /// toggle is then disabled and reads as hidden.
    let inspectorAvailable: Bool
    @Binding var showsGraph: Bool
    /// Home's History or Missions trail; nil for no breadcrumb.
    let breadcrumb: [GlassCrumb]?
    let onSettings: () -> Void
    @ViewBuilder let content: () -> Content
    @ViewBuilder let footer: () -> Footer
    /// The shown tab's popover notices, floating over its top under the
    /// tab bar and pushing nothing down (`MonitorNotices`).
    @ViewBuilder let notices: () -> Notices
    @EnvironmentObject private var model: AppModel
    /// Half the unified title bar's 52pt height.
    static var lightsCentre: CGFloat { 26 }

    /// The Settings button's gap from the toolbar capsule: #1146's `gap-2`
    /// plus `ml-1.5`.
    static var settingsGap: CGFloat { GlassTokens.Space.s4 + GlassTokens.Space.s3 }

    /// The gap from the tab bar's rule to the top of every tab's first
    /// card or control (owner, 2026-10-19: Home and Insights are the spec,
    /// and every other tab matches them). The pane owns it, so no tab adds
    /// a top inset of its own.
    static var contentTop: CGFloat { GlassTokens.Space.s8 }

    /// The tab's own insets in the pane: the inspector's side inset for
    /// every tab (owner, 2026-10-10, `MonitorWindowView.mainInset`). The
    /// Traces tree runs to the pane's bottom (the graph sits under it); the
    /// other tabs keep 12 below.
    static func contentInsets(_ tab: MonitorWindowView.Tab) -> EdgeInsets {
        let side = MonitorWindowView.mainInset
        return EdgeInsets(
            top: 0, leading: side, bottom: tab == .traces ? 0 : GlassTokens.Space.panePadding, trailing: side)
    }

    var body: some View {
        let shown = MonitorWindowView.shownTab(tab, requiresOnboarding: model.requiresOnboarding)
        // The pane draws edge to edge; each row takes #1146's own insets.
        GlassPane(padding: 0) {
            VStack(alignment: .leading, spacing: 0) {
                VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
                    HStack(spacing: Self.settingsGap) {
                        // A staging build's label (spec S7, 4.8), in the
                        // core's word; production has none. Neutral, so it
                        // never reads as a decision waiting.
                        if let label = BuildChannelGate.windowLabel {
                            GlassTag(label)
                        }
                        Spacer(minLength: 0)
                        // No View menu (owner, 2026-10-19): its one item,
                        // Show ignored folders, is in the Traces list's filter
                        // menu, beside what it filters.
                        GlassToolbarGroup {
                            // The graph is the Traces tab's (owner, 2026-10-19):
                            // elsewhere its toggle is disabled and reads as
                            // hidden, as the inspector's does where it cannot open.
                            GlassToolbarButton(
                                MonitorShellWords.graphToggle(shown: showsGraph && MonitorWindowView.graphAvailable(shown)),
                                icon: .glyph(.graph), pressed: showsGraph && MonitorWindowView.graphAvailable(shown)
                            ) {
                                // #1146's footer opens and closes over .25s.
                                withAnimation(GlassMotion.systemReducesMotion ? nil : .easeInOut(duration: 0.25)) {
                                    showsGraph.toggle()
                                }
                            }
                            .disabled(!MonitorWindowView.graphAvailable(shown))
                            GlassToolbarButton(MonitorShellWords.mapToggle(shown: showsMap), icon: .glyph(.map), pressed: showsMap) {
                                showsMap.toggle()
                            }
                            GlassToolbarButton(
                                MonitorShellWords.inspectorToggle(shown: showsInspector && inspectorAvailable),
                                icon: .glyph(.inspector), pressed: showsInspector && inspectorAvailable
                            ) {
                                showsInspector.toggle()
                            }
                            .disabled(!inspectorAvailable)
                        }
                        // Open before onboarding too: Settings gates each section
                        // itself (R-43), so Connection, Startup, Notifications,
                        // Updates, Private AI and Compute are reachable, and a
                        // section that writes what first run asks draws the
                        // onboarding notice. #1146's 28pt round button and gear.
                        GlassRoundButton(MonitorWords.table?.settingsTitle ?? "", icon: .glyph(.gear), action: onSettings)
                    }
                    // Clearance for the real traffic lights, not an origin.
                    .padding(.leading, GlassTokens.Space.windowControlsWidth - MonitorWindowView.mainInset)
                    .frame(height: GlassTokens.Size.controlLarge)
                    // Centre the row on the traffic lights, which the unified
                    // title bar centres 26pt below the window's top edge.
                    .padding(.top, Self.lightsCentre - GlassTokens.Space.windowPadding - GlassTokens.Space.panePadding
                        - GlassTokens.Size.controlLarge / 2)
                    .zIndex(1)
                    // The notices that sat here above the tabs (the shell's
                    // consent and account notices, a refused start, onboarding
                    // owed) are popover notices over the tab now (owner,
                    // 2026-10-19, `MonitorNotices`), so the tabs never move.
                    if gate != .awaiting {
                        GlassRuleTabs(
                            MonitorWords.table?.shell.tabsLabel ?? "",
                            selection: Binding(
                                get: { MonitorWindowView.shownTab(tab, requiresOnboarding: model.requiresOnboarding) },
                                set: { tab = $0 }),
                            segments: MonitorWindowView.Tab.shown(requiresOnboarding: model.requiresOnboarding).map { item in
                                GlassSegment(
                                    item.title, value: item,
                                    badgeValue: item == .traces ? tracesBadge : nil,
                                    dot: item == .inference ? inferenceDot : nil,
                                    accessibilityValue: item == .inference
                                        ? inferenceDescription : item == .traces ? tracesDescription : nil)
                            },
                            bleed: GlassRuleTabs<MonitorWindowView.Tab>.toPaneEdge)
                    }
                }
                .padding(.horizontal, MonitorWindowView.mainInset)
                .padding(.top, GlassTokens.Space.panePadding)
                .zIndex(1)
                if gate == .awaiting {
                    SettingsAwaiting()
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                        .padding(GlassTokens.Space.panePadding)
                } else {
                    // 8pt under the tabs to the breadcrumb (#1146 `mb-2`),
                    // and `contentTop` under it or the tabs to the tab.
                    if let breadcrumb {
                        // Back names Home's trail by the core's "Back to
                        // Home"; Traces' History trail by its first crumb.
                        GlassBreadcrumb(
                            breadcrumb,
                            backLabel: breadcrumb.first?.title == MonitorWindowView.Tab.home.title
                                ? (MonitorWords.table?.shell.backToHome ?? MonitorWindowView.Tab.home.title) : "",
                            onBack: breadcrumb.first?.action)
                            .padding(.horizontal, MonitorWindowView.mainInset)
                            .padding(.top, GlassTokens.Space.s4)
                    }
                    content()
                        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
                        .overlay(alignment: .top) { notices() }
                        .padding(.top, Self.contentTop)
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

/// The Private AI map (`FlowMapScene.privateAI`): every tool and where its
/// calls go, laid out for `space`. The window's map pane draws it, and the
/// edge rail's Private AI peek a small one (owner, 2026-10-22). The one
/// tool list, while the client has read it and the core still answers;
/// otherwise nothing is drawn as current.
struct PrivateAIFlowMap: View {
    let traces: TracesStore
    let inference: InferenceStore
    let label: String
    /// The core's sentence for a tool's Private AI state.
    let sentence: (HarnessRow) -> String?
    let space: CGSize
    @EnvironmentObject private var model: AppModel

    var body: some View {
        let harnesses = PrivateAIInspectorView.liveHarnesses(
            model.harnesses, read: inference.harnesses, failure: inference.failures["harness_list"])
        let destinations = inference.failures["tool_destinations"] == nil ? inference.destinations : nil
        let scene = FlowMapScene.privateAI(
            harnesses, destinationLabel: label,
            privateAI: traces.destinations?.privateAi ?? traces.status?.privateInferenceState?.state,
            sentence: sentence,
            state: { HarnessSurface.state($0, calls: model.harnessCalls) },
            destinations: destinations,
            space: space)
            .addingManagedRoutes(destinations)
        if scene.drawsMap {
            FlowMapView(scene: scene, legend: [], zoomable: false, accessibilityName: label)
        } else {
            Color.clear
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
    /// A folder node clicked on the Traces map, by project id.
    let onSelectFolder: (String) -> Void
    @EnvironmentObject private var model: AppModel
    /// How many folders past the first ten the Traces map's pill has
    /// drawn (`FlowMapScene.folderColumn`).
    @State private var revealedFolders: [String: Int] = [:]
    /// The window's transfer player (`MapTransferFeed`): the traces moving
    /// across the Traces map, and the one its card describes.
    @ObservedObject var transfers: MapTransferPlayer
    /// Whether the map carries the traces' cards: only while another tab
    /// than Traces is shown (owner, 2026-10-19). Traces shows them over
    /// its own list, and with the map hidden too the system's notifications
    /// are what tell (the digest counts what was contributed).
    var showsTransferCards = true
    /// The Traces map's rocket and station, or the laptop and orb
    /// (`TracesMapStyle`, Settings, General); it changes at once.
    @AppStorage(TracesMapStyle.key) private var mapRocket = true
    /// The account's devices (#1668), for the Traces map's other computers:
    /// read when the map has a client and again when the account session
    /// changes, as the Devices section reads them.
    @StateObject private var devices = AccountDevicesModel()

    private var mapStyle: TracesMapStyle { mapRocket ? .launch : .standard }

    var body: some View {
        // The map is content, not chrome: its own field, so the selector,
        // zoom and node cards floating on it are its only glass (Apple: no
        // glass on glass; R14).
        // #1146's map field and map edge, its view tabs 14pt in.
        GlassPane(padding: 0, isContent: true, edge: GlassTokens.Shadow.mapEdge) {
            GlassMapStage(alignment: .topTrailing) {
                map
                GlassFloatingGroup {
                    GlassSegmentedTabs(MonitorWords.table?.shell.mapViewsLabel ?? "", selection: $mapTab, segments: segments, floating: true)
                        .padding(GlassTokens.Space.mapOverlayInset)
                }
                stateLine
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .bottomLeading)
            }
        }
        .task(id: model.payoutLinkDaemonClient.map(ObjectIdentifier.init)) {
            devices.attach(model.payoutLinkDaemonClient)
            await devices.refresh()
        }
        .onChange(of: model.accountSessionGeneration) {
            Task { await devices.refresh() }
        }
    }

    @ViewBuilder
    private var map: some View {
        // Laid out for the pane's own shape (owner, 2026-10-14: reflow the
        // maps for the space there is), so neither letterboxes in it.
        GeometryReader { proxy in
            mapContent(space: FlowMapScene.space(for: proxy.size))
        }
    }

    @ViewBuilder
    private func mapContent(space: CGSize) -> some View {
        switch shownTab {
        case .traces:
            let scene = FlowMapScene.traces(
                traces.tree,
                gate: .init(state: tracesState, status: traces.status, destinations: traces.destinations),
                contributed: .init(history: history, failure: historyFailure), selectedTool: selectedTool,
                revealed: revealedFolders, space: space)
                .addingDevices(devicesOnMap, offersLink: devices.devices != nil && devices.surface != nil)
            FlowMapView(
                scene: scene,
                legend: [.autoUpload, .ask, .ignore], zoomable: true,
                accessibilityName: FlowMapScene.words?.mapLabel ?? MonitorWindowView.Tab.traces.title, state: tracesState,
                focus: scene.focusPoint(tool: focusTool), onSelectFolder: onSelectFolder,
                onReveal: { revealedFolders = FlowMapScene.revealing($0, in: revealedFolders) },
                transferRun: transfers.run, transferCards: showsTransferCards ? transfers.cards : [],
                onCloseTransfer: { transfers.close($0) }, mapStyle: mapStyle,
                onAddDevice: {
                    // Settings' Devices section, which starts linking a
                    // server on the request (owner, 2026-10-27).
                    AccountDeviceLinkRequest.shared.ask()
                    OpenMonitor.request(.settings(.connection))
                },
                addDeviceLabel: devices.surface?.copy.linkAccessibilityName ?? "")
        case .privateAI:
            if let privateAILabel {
                PrivateAIFlowMap(
                    traces: traces, inference: inference, label: privateAILabel, sentence: sentence, space: space)
            } else {
                Color.clear
            }
        }
    }

    /// The other devices on the account, named and dated as the Devices
    /// section shows them; none while its words or its list are unread.
    private var devicesOnMap: [FlowMapScene.OtherDevice] {
        guard let surface = devices.surface else { return [] }
        return FlowMapScene.otherDevices(
            devices.devices, name: { surface.name(for: $0) },
            lastSeen: { surface.lastSeenLine(for: $0, formatted: AccountDevicesView.relative) },
            traces: { count in FlowMapScene.words?.tracesCounted(count) ?? String(count) })
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
/// it; it is kept so anything that still names it routes to Settings,
/// which is its own window again (owner, 2026-10-08, `SettingsWindowView`).
/// It draws nothing of its own: it opens that window and closes itself.
struct MonitorSettingsWindow: View {
    let navigation: MainWindowNavigation

    @Environment(\.openWindow) private var openWindow
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        Color.clear
            .frame(width: 1, height: 1)
            .onAppear {
                navigation.requestSettings()
                openWindow(id: WindowID.settings)
                dismiss()
            }
    }
}
