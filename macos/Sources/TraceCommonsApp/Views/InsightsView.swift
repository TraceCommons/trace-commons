import SwiftUI
import TCDesign
import TCBridge
import TCShellCore

struct InsightsView: View {
    @State private var model: InsightsModel
    @State private var overviewModel: InsightsOverviewModel
    @State private var patternsModel: InsightsPatternsModel
    @State private var comparisonsModel: InsightsComparisonsModel
    @State private var submittedStore: InsightsSubmittedStore
    /// Home's figures, for Overview's credit tile; `nil` hides the tile.
    private let home: HomeStore?
    @State private var fallbackSubscriptionModel: SubscriptionUsageModel
    private let sharedSubscriptionModel: SubscriptionUsageModel?
    private var subscriptionModel: SubscriptionUsageModel { sharedSubscriptionModel ?? fallbackSubscriptionModel }
    @State private var tab: InsightsTab
    /// "Turn off" on the weekly summary card: the daemon's setting, its only
    /// switch. `nil` without a daemon.
    private let turnOffRecapCard: (@Sendable () async throws -> Void)?
    private let storeSelection: InsightsStoreSelection
    private let storeCopy: [String: String]
    /// Details' drill-down view; a usage card's "By project" opens it on
    /// By project (product owner, 2026-10-28).
    @State private var detailsView: InsightsCreditWords.Dimension = .folder
    /// The get-started card's one action: Traces, on its folder list.
    /// `nil` draws no link.
    private let openTraces: (() -> Void)?
    /// Traces' History page, where Train consent is added; `nil` draws no
    /// button on Submitted.
    private let openHistory: (() -> Void)?
    /// History's words for statuses and no-credit reasons, which Submitted
    /// shows as History does.
    private let historyLabels: PublicRunCopy?
    /// The sample under the get-started card, built the first time it is
    /// drawn (`InsightsSampleModels`); never the person's own models.
    @State private var sample: InsightsSampleModels?

    /// `daemon` is the app's live client, for the week from the daemon's
    /// counter pass (feed T); without one the window shows saved imports.
    @MainActor init(storeSelection: InsightsStoreSelection = .standard,
                    storeCopy: [String: String]? = TCInsights.copy(),
                    daemon: (any DaemonDataClient)? = nil,
                    subscriptions: SubscriptionUsageModel? = nil,
                    home: HomeStore? = nil,
                    historyLabels: PublicRunCopy? = nil,
                    openTraces: (() -> Void)? = nil,
                    openHistory: (() -> Void)? = nil) {
        let router = InsightsServiceRouter(selection: storeSelection)
        var subscriptionReader: SubscriptionUsageModel.Reader? = daemon.map { client in
            { @Sendable refresh, interactive, provider, login in
                try await client.subscriptionUsage(refresh: refresh, interactive: interactive, provider: provider, login: login)
            }
        }
        var subscriptionWriter: SubscriptionUsageModel.Writer? = daemon.map { client in
            { @Sendable provider, on in try await client.setSubscriptionUsage(provider: provider, on: on) }
        }
        var subscriptions = subscriptions
        var service: InsightsModel.Service = { request in try await router.call(request) }
        var weekReader: InsightsModel.WeekReader? = daemon.map { client in
            { @Sendable isoWeek in try await client.insightsWeek(isoWeek: isoWeek) }
        }
        #if DEBUG
        // A design review's sample week, in place of the daemon's week and
        // the store's answers (`InsightsSampleWeek`).
        if InsightsSampleWeek.enabled {
            service = InsightsSampleWeek.service(fallback: service)
            weekReader = nil
            subscriptionReader = nil
            subscriptionWriter = nil
            subscriptions = nil
        }
        #endif
        var submitted: InsightsSubmittedStore.Reader? = daemon.map { client in
            { @Sendable weeks in try await client.insightsSubmitted(weeks: weeks) }
        }
        #if DEBUG
        if InsightsSampleWeek.enabled { submitted = nil }
        #endif
        let turnOff: (@Sendable () async throws -> Void)? = daemon.map { client in
            { @Sendable in _ = try await client.setInsightsRecapCard(false) }
        }
        self.init(storeSelection: storeSelection, storeCopy: storeCopy,
                  model: InsightsModel(service: service, weekReader: weekReader),
                  turnOffRecapCard: turnOff,
                  subscriptionModel: subscriptions,
                  subscriptionFallback: SubscriptionUsageModel(reader: subscriptionReader, writer: subscriptionWriter),
                  submitted: InsightsSubmittedStore(reader: submitted),
                  home: home, historyLabels: historyLabels, openTraces: openTraces,
                  openHistory: openHistory)
    }

    /// The same view over models the caller built, so a caller that renders
    /// it can wait for the models it shows to be populated rather than for a
    /// timer. The models must be routed to `storeSelection`.
    @MainActor init(storeSelection: InsightsStoreSelection,
                    storeCopy: [String: String]?,
                    model: InsightsModel,
                    overviewModel: InsightsOverviewModel? = nil,
                    initialTab: InsightsTab = .overview,
                    turnOffRecapCard: (@Sendable () async throws -> Void)? = nil,
                    subscriptionModel: SubscriptionUsageModel? = nil,
                    subscriptionFallback: SubscriptionUsageModel? = nil,
                    submitted: InsightsSubmittedStore? = nil,
                    home: HomeStore? = nil,
                    historyLabels: PublicRunCopy? = nil,
                    openTraces: (() -> Void)? = nil,
                    openHistory: (() -> Void)? = nil) {
        self.openTraces = openTraces
        self.openHistory = openHistory
        self.historyLabels = historyLabels ?? PublicRunCopy.decode(fromJSON: TCPublicRun.copyJSON() ?? "")
        self.storeSelection = storeSelection
        self.storeCopy = storeCopy ?? [:]
        _model = State(initialValue: model)
        _overviewModel = State(initialValue: overviewModel ?? InsightsOverviewModel(service: model.service))
        _patternsModel = State(initialValue: InsightsPatternsModel(service: model.service))
        _comparisonsModel = State(initialValue: InsightsComparisonsModel(service: model.service))
        _submittedStore = State(initialValue: submitted ?? InsightsSubmittedStore(reader: nil))
        self.home = home
        self.sharedSubscriptionModel = subscriptionModel
        _fallbackSubscriptionModel = State(initialValue: subscriptionFallback ?? SubscriptionUsageModel())
        _tab = State(initialValue: initialTab)
        self.turnOffRecapCard = turnOffRecapCard
    }

    /// The tabs on the bar, in order: Overview, Breakdown, Submitted
    /// (product owner, 2026-10-29: Submitted in place of Benchmarks).
    nonisolated static let shownTabs: [InsightsTab] = [.overview, .patterns, .submitted]

    /// Home's credit tile, the same figure and word as Home's.
    private var creditTile: (value: String, detail: String)? {
        guard let home, home.rollup != nil || home.credit != nil else { return nil }
        return HomeFormat.creditTile(final: home.rollup?.creditFinal, pending: home.rollup?.creditPending,
                                     accepted: home.rollup?.allTime?.accepted, credit: home.credit)
    }

    /// Whether the week's first answers are in: the daemon's week (or no
    /// daemon to ask) and the store's week.
    private var weekSettled: Bool {
        model.weekSettled && !overviewModel.busy && (overviewModel.shown != nil || overviewModel.failed)
    }

    /// Overview and Breakdown have no trace of the person's own to show
    /// (product owner, 2026-10-29): the week on screen and every kept week
    /// are empty.
    private var weekEmpty: Bool {
        weekSettled && !InsightsEmptyState.weekHasData(overviewModel.shown, history: model.counterWeek?.coreHistory)
    }

    /// Submitted has no trace ever sent to show.
    private var submittedEmpty: Bool { InsightsSubmittedWords.isEmpty(submittedStore) }

    /// Whether the tab on screen draws the get-started card and the sample.
    private var showsSample: Bool {
        switch tab {
        case .overview, .patterns: weekEmpty
        case .submitted: submittedEmpty
        }
    }

    private var getStarted: InsightsGetStarted {
        InsightsGetStarted(copy: model.copy, openTraces: openTraces)
    }

    /// Submitted's get-started card: its steps lead to the first trace sent.
    private var submittedGetStarted: InsightsGetStarted {
        InsightsGetStarted(copy: model.copy, openTraces: openTraces, words: .submitted)
    }

    @ViewBuilder
    var body: some View {
        if let refusal = storeSelection.refusal {
            GlassNotice(tone: .outside, title: storeCopy["insights_store_unavailable"] ?? "") {
                Text(refusalMessage(refusal))
            }
            .padding(24)
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        } else {
            tabs
        }
    }

    /// The tab container. No Spend tab: a tab that cannot be used is
    /// hidden, not drawn greyed. Insights may show money (product owner,
    /// 2026-10-28, over owner decision D5): NEAR AI spend on Overview and
    /// Details, from the NEAR AI ledger's own charge. The NEAR AI balance
    /// stays on the Private AI tab. The models open and
    /// close with the container, not with a tab.
    private var tabs: some View {
        // The page is in the main pane, whose insets it shares: no inset of
        // its own across, so its cards get the pane's whole 408pt.
        VStack(alignment: .leading, spacing: 0) {
            // Overview, Breakdown and Submitted (product owner,
            // 2026-10-29). A picked day shows its sessions in place.
            GlassSegmentedTabs(model.text("title"), selection: $tab, segments: Self.shownTabs.map {
                GlassSegment(model.text($0.copyKey), value: $0)
            })
            // A failed copy or snapshot read, on every tab.
            if let error = model.error { Text(error).foregroundStyle(GlassStatus.outside.textColor) }
            // No top inset of its own: the main pane's `contentTop` is the
            // gap under the tab bar on every tab (owner, 2026-10-19).
            switch tab {
            case .overview where showsSample:
                if let sample {
                    InsightsOverviewTab(
                        model: sample.overview, comparisons: sample.comparisons, copy: model.copy, snapshots: [],
                        subscriptions: sample.subscriptions, usageByDay: sample.week?.usageByDay,
                        work: sample.week?.work, weekChanges: sample.week?.changeVsLastWeek,
                        getStarted: getStarted)
                    .task { await sample.load() }
                }
            case .patterns where showsSample:
                if let sample {
                    InsightsPatternsTab(model: sample.patterns, comparisons: sample.comparisons, copy: model.copy,
                                        week: sample.week, overview: sample.overview, getStarted: getStarted)
                }
            case .submitted where showsSample:
                if let sample {
                    InsightsSubmittedTab(store: sample.submitted, copy: model.copy, labels: historyLabels,
                                         getStarted: submittedGetStarted)
                }
            case .overview:
                InsightsOverviewTab(
                    model: overviewModel, comparisons: comparisonsModel, copy: model.copy,
                    snapshots: model.snapshots,
                    notice: model.counterPassNoticeKey.map(model.text),
                    subscriptions: subscriptionModel,
                    nearAiCredit: model.counterWeek?.nearAiCredit,
                    usageByDay: model.counterWeek?.usageByDay,
                    showProjects: { detailsView = .folder; tab = .patterns },
                    work: model.counterWeek?.work,
                    weekChanges: model.counterWeek?.changeVsLastWeek,
                    creditTile: creditTile,
                    showSubmitted: { tab = .submitted },
                    selectWeek: selectWeek,
                    openRecap: { if let week = comparisonsModel.openRecap() { selectWeek(week) } },
                    turnOffRecap: turnOffRecapCard.map { turnOff in
                        { Task { try? await turnOff(); await model.loadWeek() } }
                    })
                .task(id: ObjectIdentifier(subscriptionModel)) { await subscriptionModel.observe() }
            case .patterns:
                InsightsPatternsTab(model: patternsModel, comparisons: comparisonsModel, copy: model.copy,
                                    selectWeek: selectWeek,
                                    week: model.counterWeek, detailsView: $detailsView,
                                    overview: overviewModel, snapshots: model.snapshots)
            case .submitted:
                InsightsSubmittedTab(store: submittedStore, copy: model.copy, labels: historyLabels,
                                     openHistory: openHistory)
            }
        }
        .onAppear { model.open() }
        .onAppear { overviewModel.open() }
        .onAppear { patternsModel.open() }
        .onAppear { showSampleWeekIfAsked() }
        // The sample is built only when it is first needed, and while it
        // shows, Insights asks again each minute, so the card and the
        // sample go as soon as the first trace is counted.
        .onChange(of: showsSample, initial: true) { _, shows in
            if shows, sample == nil { sample = InsightsSampleModels() }
        }
        .task(id: showsSample) {
            guard showsSample else { return }
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(60))
                guard !Task.isCancelled else { return }
                await model.loadWeek()
                overviewModel.reload()
                await submittedStore.load()
            }
        }
        .onChange(of: model.snapshots.map(\.id)) { _, _ in
            overviewModel.reload()
            patternsModel.reload()
        }
        .onChange(of: model.counterWeek) { _, week in showCounterWeek(week) }
        // Snapshots saved while the app was in the background; a changed
        // list reloads Overview and Breakdown through the onChange above.
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in
            model.reloadSnapshots()
        }
        .onDisappear {
            model.close(); overviewModel.close(); patternsModel.close(); comparisonsModel.clear()
        }
    }

    /// Debug builds: the sample week as the daemon's week (feed T), when
    /// `TRACE_COMMONS_INSIGHTS_SAMPLE=1`, with its counted sessions behind
    /// Overview's Sessions breakdown.
    private func showSampleWeekIfAsked() {
        #if DEBUG
        guard InsightsSampleWeek.enabled,
              let overview = try? InsightsSampleWeek.weekOverview,
              let patterns = try? InsightsSampleWeek.weekPatterns else { return }
        overviewModel.showCounter(overview, sessions: try? InsightsSampleWeek.counterSessions)
        patternsModel.showCounter(patterns)
        comparisonsModel.load(counterWeeks: [], weekStart: InsightsSampleWeek.weekStart, recapCardEnabled: true)
        #endif
    }

    /// Put a week on screen in the feed showing: the daemon's for feed T,
    /// the saved snapshots' for feed S. Never both.
    private func selectWeek(_ weekStart: String) {
        if model.weekFeed == .counterPass, let iso = InsightsOverviewWords.isoWeek(weekStart) {
            Task { await model.loadWeek(isoWeek: iso) }
        } else {
            overviewModel.selectWeek(weekStart); patternsModel.selectWeek(weekStart)
        }
    }

    /// Feed T's week replaces the saved week in both tabs, its counted
    /// sessions back the Sessions card's drill-down, and its weekly figures
    /// go to the core for goals, the lever and the summary card; `nil`
    /// (feed S) takes all of it away.
    private func showCounterWeek(_ week: DaemonData.InsightsWeek?) {
        overviewModel.showCounter(week?.coreOverview, sessions: week.flatMap(InsightsCounterSessions.init(week:)))
        patternsModel.showCounter(week?.corePatterns)
        comparisonsModel.load(counterWeeks: week?.coreHistory, weekStart: week?.weekStart,
                              recapCardEnabled: week?.recapCardEnabled ?? false)
    }

    private func refusalMessage(_ refusal: InsightsStoreSelection.Refusal) -> String {
        switch refusal {
        case .duplicateOption: return storeCopy["insights_store_duplicate"] ?? ""
        case .missingPath: return storeCopy["insights_store_missing_path"] ?? ""
        case .relativePath: return storeCopy["insights_store_relative_path"] ?? ""
        case .pathMissing: return storeCopy["insights_store_path_missing"] ?? ""
        case .notADirectory: return storeCopy["insights_store_not_directory"] ?? ""
        }
    }
}

/// The Insights window's tabs: Overview, Breakdown (`patterns`) and
/// Submitted.
enum InsightsTab: Hashable {
    case overview, patterns, submitted

    /// The copy key of the tab's name.
    var copyKey: String {
        switch self {
        case .overview: "analytics_tab_overview"
        case .patterns: "analytics_tab_patterns"
        case .submitted: "analytics_tab_submitted"
        }
    }
}
