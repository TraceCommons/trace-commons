import SwiftUI
import TCDesign
import TCBridge
import TCShellCore
import UniformTypeIdentifiers

struct InsightsView: View {
    @State private var model: InsightsModel
    @State private var comparisonModel: ComparisonTasksModel
    @State private var specificationModel: ComparisonSpecificationsModel
    @State private var overviewModel: InsightsOverviewModel
    @State private var patternsModel: InsightsPatternsModel
    @State private var sessionsModel: InsightsSessionsModel
    @State private var comparisonsModel: InsightsComparisonsModel
    @State private var fallbackSubscriptionModel: SubscriptionUsageModel
    private let sharedSubscriptionModel: SubscriptionUsageModel?
    private var subscriptionModel: SubscriptionUsageModel { sharedSubscriptionModel ?? fallbackSubscriptionModel }
    @State private var tab: InsightsTab
    /// "Turn off" on the weekly summary card: the daemon's setting, its only
    /// switch. `nil` without a daemon.
    private let turnOffRecapCard: (@Sendable () async throws -> Void)?
    private let storeSelection: InsightsStoreSelection
    private let storeCopy: [String: String]
    /// The Monitor window's inspector, which shows the open Sessions trace
    /// (`InsightsInspector`); `nil` opens it under its row instead.
    private let inspector: InsightsInspector?
    /// The NEAR AI credit card at the top of Overview (product owner,
    /// 2026-10-28): drawn by the caller, which has the app's model.
    private var credit: AnyView?
    /// The daemon client `insights_credit` is read through; `nil` hides
    /// where credit went.
    private var creditClient: (any DaemonDataClient)?
    @State private var creditModel = InsightsCreditModel()
    @State private var source = "codex"

    /// Puts `view` at the top of Overview, as NEAR AI credit used and left,
    /// and reads where it went through `client`.
    func credit(_ view: AnyView, client: (any DaemonDataClient)?) -> InsightsView {
        var copy = self
        copy.credit = view
        copy.creditClient = client
        return copy
    }

    /// `daemon` is the app's live client, for the week from the daemon's
    /// counter pass (feed T); without one the window shows saved imports.
    @MainActor init(storeSelection: InsightsStoreSelection = .standard,
                    storeCopy: [String: String]? = TCInsights.copy(),
                    daemon: (any DaemonDataClient)? = nil,
                    subscriptions: SubscriptionUsageModel? = nil,
                    inspector: InsightsInspector? = nil) {
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
        var watchedReader: InsightsSessionsModel.WatchedReader? = daemon.map { client in
            { @Sendable cursor in try await client.insightsSessions(before: cursor, limit: InsightsSessionsModel.pageSize) }
        }
        let watchedDrill: InsightsSessionsModel.WatchedDrillReader? = daemon.map { client in
            { @Sendable reference in try await client.insightsSessionDrill(sessionRef: reference) }
        }
        let modelAnswersReader: InsightsModel.ModelAnswersReader? = daemon.map { client in
            { @Sendable in try await client.insightsModelAnswers() }
        }
        let watchedLabel: InsightsSessionsModel.WatchedLabelWriter? = daemon.map { client in
            { @Sendable reference, task, outcome in
                try await client.insightsSessionLabel(sessionRef: reference, task: task, outcome: outcome)
            }
        }
        // The daemon saves to the standard store only: with a custom store
        // its snapshot would never show here, so the action is not offered.
        var sessionAnalyzer: InsightsModel.SessionAnalyzer? = storeSelection.storeDirectory != nil
            ? nil
            : daemon.map { client in
                { @Sendable reference in try await client.insightsAnalyzeSession(sessionRef: reference, save: true) }
            }
        #if DEBUG
        // A design review's sample week, in place of the daemon's week and
        // the store's answers, Analyze's saved traces included
        // (`InsightsSampleWeek`). Analyze & save is not offered: it would
        // save to the real store.
        if InsightsSampleWeek.enabled {
            service = InsightsSampleWeek.service(fallback: service)
            weekReader = nil
            watchedReader = nil
            sessionAnalyzer = nil
            subscriptionReader = nil
            subscriptionWriter = nil
            subscriptions = nil
        }
        #endif
        let turnOff: (@Sendable () async throws -> Void)? = daemon.map { client in
            { @Sendable in _ = try await client.setInsightsRecapCard(false) }
        }
        self.init(storeSelection: storeSelection, storeCopy: storeCopy,
                  model: InsightsModel(service: service, weekReader: weekReader,
                                       modelAnswersReader: modelAnswersReader,
                                       sessionAnalyzer: sessionAnalyzer),
                  comparisonModel: ComparisonTasksModel(service: service),
                  specificationModel: ComparisonSpecificationsModel(service: service),
                  sessionsModel: InsightsSessionsModel(service: service, watchedReader: watchedReader,
                                                       watchedDrill: watchedDrill, watchedLabel: watchedLabel),
                  turnOffRecapCard: turnOff,
                  subscriptionModel: subscriptions,
                  subscriptionFallback: SubscriptionUsageModel(reader: subscriptionReader, writer: subscriptionWriter),
                  inspector: inspector)
    }

    /// The same view over models the caller built, so a caller that renders
    /// it can wait for the models it shows to be populated rather than for a
    /// timer. The models must be routed to `storeSelection`.
    @MainActor init(storeSelection: InsightsStoreSelection,
                    storeCopy: [String: String]?,
                    model: InsightsModel,
                    comparisonModel: ComparisonTasksModel,
                    specificationModel: ComparisonSpecificationsModel,
                    overviewModel: InsightsOverviewModel? = nil,
                    sessionsModel: InsightsSessionsModel? = nil,
                    initialTab: InsightsTab = .overview,
                    turnOffRecapCard: (@Sendable () async throws -> Void)? = nil,
                    subscriptionModel: SubscriptionUsageModel? = nil,
                    subscriptionFallback: SubscriptionUsageModel? = nil,
                    inspector: InsightsInspector? = nil) {
        self.storeSelection = storeSelection
        self.storeCopy = storeCopy ?? [:]
        _model = State(initialValue: model)
        _comparisonModel = State(initialValue: comparisonModel)
        _specificationModel = State(initialValue: specificationModel)
        _overviewModel = State(initialValue: overviewModel ?? InsightsOverviewModel(service: model.service))
        _patternsModel = State(initialValue: InsightsPatternsModel(service: model.service))
        _sessionsModel = State(initialValue: sessionsModel ?? InsightsSessionsModel(service: model.service))
        _comparisonsModel = State(initialValue: InsightsComparisonsModel(service: model.service))
        self.sharedSubscriptionModel = subscriptionModel
        _fallbackSubscriptionModel = State(initialValue: subscriptionFallback ?? SubscriptionUsageModel())
        _tab = State(initialValue: initialTab)
        self.turnOffRecapCard = turnOffRecapCard
        self.inspector = inspector
    }

    /// The line naming a custom store, exactly as the view renders it; `nil`
    /// for the standard store, which shows no location line.
    static func storeLocationLine(_ selection: InsightsStoreSelection,
                                  copy: [String: String]) -> String? {
        guard case .custom(let path) = selection else { return nil }
        return (copy["insights_store_title"] ?? "") + ": " + path
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
    /// hidden, not drawn greyed, and Insights shows no money (owner
    /// decision D5). The models open and close with the container, not with
    /// a tab.
    private var tabs: some View {
        // The page is in the main pane, whose insets it shares: no inset of
        // its own across, so its cards get the pane's whole 408pt.
        VStack(alignment: .leading, spacing: 0) {
            GlassSegmentedTabs(model.text("title"), selection: $tab, segments: [
                GlassSegment(model.text("analytics_tab_overview"), value: InsightsTab.overview),
                GlassSegment(model.text("analytics_tab_patterns"), value: InsightsTab.patterns),
                GlassSegment(model.text("analytics_tab_sessions"), value: InsightsTab.sessions),
                GlassSegment(model.text("analytics_tab_analyze"), value: InsightsTab.analyze),
            ])
            // No top inset of its own: the main pane's `contentTop` is the
            // gap under the tab bar on every tab (owner, 2026-10-19).
            switch tab {
            case .overview:
                InsightsOverviewTab(
                    model: overviewModel, comparisons: comparisonsModel, copy: model.copy,
                    snapshots: model.snapshots,
                    notice: model.counterPassNoticeKey.map(model.text),
                    patterns: patternsModel.shown,
                    models: model.modelAnswers,
                    subscriptions: subscriptionModel,
                    credit: credit,
                    creditModel: creditClient == nil ? nil : creditModel,
                    showDetails: { tab = .patterns },
                    selectWeek: selectWeek,
                    openRecap: { if let week = comparisonsModel.openRecap() { selectWeek(week) } },
                    turnOffRecap: turnOffRecapCard.map { turnOff in
                        { Task { try? await turnOff(); await model.loadWeek() } }
                    },
                    showReads: { tab = .patterns },
                    showPatterns: { tab = .patterns })
                // Each time, so labels set on Sessions since show, and one
                // failed read hides the card only until the next.
                .onAppear { model.refreshModelAnswers() }
                .task(id: ObjectIdentifier(subscriptionModel)) { await subscriptionModel.observe() }
                .task { await creditModel.load(creditClient) }
            case .patterns:
                // Details, in place of Patterns (product owner, 2026-10-28):
                // where NEAR AI credit went. Without a daemon client (saved
                // imports only) the Patterns cards stay.
                if creditClient != nil {
                    InsightsDetailsTab(model: creditModel, copy: model.copy)
                        .task { await creditModel.load(creditClient) }
                } else {
                    InsightsPatternsTab(model: patternsModel, comparisons: comparisonsModel, copy: model.copy,
                                        selectWeek: selectWeek)
                }
            case .sessions:
                InsightsSessionsTab(model: sessionsModel, snapshots: model.snapshots, copy: model.copy,
                                    notice: (model.counterPassNoticeKey ?? sessionsModel.noticeKey).map(model.text),
                                    analysis: model.canAnalyzeSessions ? model : nil,
                                    openAnalysis: { tab = .analyze },
                                    detailInInspector: inspector != nil,
                                    opened: { inspector?.personOpened() })
            case .analyze: content
            }
        }
        .onAppear { model.open() }
        .onAppear { comparisonModel.open() }
        .onAppear { specificationModel.open() }
        .onAppear { overviewModel.open() }
        .onAppear { patternsModel.open() }
        .onAppear { sessionsModel.open(); sessionsModel.sync(snapshotIDs: model.snapshots.map(\.id)) }
        // A saved label changes the Models answers.
        .onAppear {
            // Bound first, so the callback's weak capture is of this local and
            // not of the closure's implicit strong one (Swift 6.4 refuses that).
            let model = model
            sessionsModel.labelSaved = { [weak model] in model?.refreshModelAnswers() }
        }
        .onAppear { showSampleWeekIfAsked() }
        // The open Sessions trace is the inspector's while Sessions is shown.
        .onChange(of: tab, initial: true) { _, shown in showSessionDetail(shown) }
        .onChange(of: comparisonTaskVersions) { _, _ in
            specificationModel.sourceEvidenceChanged(tasks: comparisonModel.tasks, snapshots: model.snapshots)
        }
        .onChange(of: model.snapshots.map(\.id)) { _, _ in
            updateSpecificationSources()
            overviewModel.reload()
            patternsModel.reload()
            sessionsModel.sync(snapshotIDs: model.snapshots.map(\.id))
        }
        .onChange(of: model.counterWeek) { _, week in showCounterWeek(week) }
        .onChange(of: model.comparisonInvalidationGeneration) { _, _ in
            comparisonModel.upstreamEvidenceChanged()
            specificationModel.upstreamEvidenceChanged()
        }
        .onDisappear {
            model.close(); comparisonModel.close(); specificationModel.close(); overviewModel.close()
            patternsModel.close(); sessionsModel.close(); comparisonsModel.clear()
            inspector?.clear()
        }
    }

    /// Hands the inspector the Sessions details while Sessions is `shown`,
    /// and takes them back on any other tab.
    private func showSessionDetail(_ shown: InsightsTab) {
        guard let inspector else { return }
        guard shown == .sessions else { inspector.clear(); return }
        inspector.show(sessionsModel) { [model, sessionsModel] in
            InsightsSessionDetail(model: sessionsModel, copy: model.copy,
                                  notice: (model.counterPassNoticeKey ?? sessionsModel.noticeKey).map(model.text),
                                  analysis: model.canAnalyzeSessions ? model : nil,
                                  openAnalysis: { tab = .analyze })
        }
    }

    /// Debug builds: the sample week as the daemon's week (feed T), when
    /// `TRACE_COMMONS_INSIGHTS_SAMPLE=1`, with its counted sessions behind
    /// Overview's Sessions breakdown. Sessions shows one sample trace until
    /// the sample's saved list arrives.
    private func showSampleWeekIfAsked() {
        #if DEBUG
        guard InsightsSampleWeek.enabled,
              let overview = try? InsightsSampleWeek.weekOverview,
              let patterns = try? InsightsSampleWeek.weekPatterns else { return }
        overviewModel.showCounter(overview, sessions: try? InsightsSampleWeek.counterSessions)
        patternsModel.showCounter(patterns)
        comparisonsModel.load(counterWeeks: [], weekStart: InsightsSampleWeek.weekStart, recapCardEnabled: true)
        if model.snapshots.isEmpty { sessionsModel.sync(snapshotIDs: ["sample"]) }
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
    /// sessions back the Sessions card's drill-down, the Sessions tab lists
    /// watched traces, and its weekly figures go to the core for goals, the
    /// lever and the summary card; `nil` (feed S) takes all of it away.
    private func showCounterWeek(_ week: DaemonData.InsightsWeek?) {
        sessionsModel.showWatched(week != nil)
        overviewModel.showCounter(week?.coreOverview, sessions: week.flatMap(InsightsCounterSessions.init(week:)))
        patternsModel.showCounter(week?.corePatterns)
        comparisonsModel.load(counterWeeks: week?.coreHistory, weekStart: week?.weekStart,
                              recapCardEnabled: week?.recapCardEnabled ?? false)
    }

    /// The Analyze tab: choosing and saving traces, and the saved-history
    /// tools, each section on its own card under its title (owner,
    /// 2026-10-15: between hairlines they ran together). What a section is
    /// and what it keeps is behind its title's "i"; failures, a custom
    /// store and what an action needs first stay on the page.
    private var content: some View {
        ScrollViewReader { proxy in
            ScrollView {
                VStack(alignment: .leading, spacing: InsightsLayout.sectionGap) {
                    analyzeSection
                    summarySection
                    if let insight = model.selected {
                        detailSection(insight)
                            .id("insight-detail")
                    }
                    InsightsEpisodesView(model: model)
                        .insightsSection()
                    ComparisonTasksView(model: comparisonModel, episodes: model.episodes, copy: model.copy,
                                        openEpisode: model.openEpisode, openSnapshot: model.explain)
                        .insightsSection()
                    ComparisonSpecificationsView(model: specificationModel, copy: model.copy,
                                                 openTask: comparisonModel.select)
                        .insightsSection()
                    InsightCardsView(model: model)
                        .insightsSection()
                    // While none is saved, the head says how to save one.
                    if !model.snapshots.isEmpty { savedSection }
                }.padding(.vertical, GlassTokens.Space.s7).frame(maxWidth: .infinity, alignment: .leading)
                .insightsSurface()
            }
            // Initial too: Analyze & save sets the detail while Sessions
            // still shows, so Analyze appears with it already set.
            .onChange(of: model.selected?.id, initial: true) { _, id in
                if id != nil { proxy.scrollTo("insight-detail", anchor: .top) }
            }
        }
    }

    /// Sessions offers Analyze & save on its open watched row: a daemon
    /// that writes the store shown here, and its watched list on screen.
    private var offersSessionAnalyze: Bool {
        model.canAnalyzeSessions && model.weekFeed == .counterPass
    }

    /// Analyze's head: a trace file to analyze, and while Sessions offers
    /// Analyze & save (Insights PM, 2026-10-21: terminal users never see
    /// their trace files), that too. The file stays the one way in without
    /// it: counting off, no daemon, a custom store or sample data. The line
    /// under the title names a button on screen. What analysis keeps, and
    /// that closing discards late results, are behind the title's "i".
    private var analyzeSection: some View {
        VStack(alignment: .leading, spacing: InsightsLayout.groupGap) {
            // Refresh at the title's trailing end, as every refresh is
            // (owner, 2026-10-20).
            InsightsSectionTitle(title: model.text("analytics_tab_analyze"), info: [
                model.text("intro"), model.text("snapshot_notice"),
            ] + (offersSessionAnalyze ? [model.text("analytics_analyze_intro")] : []) + [
                model.text("cancellation_notice"),
            ]) {
                if model.busy { GlassSpinner() }
                GlassRefreshButton(model.text("refresh_accessibility")) { model.refresh() }
                    .disabled(model.busy)
            }
            VStack(alignment: .leading, spacing: InsightsLayout.groupGap) {
                if let location = Self.storeLocationLine(storeSelection, copy: storeCopy) {
                    Text(location)
                        .glassType(GlassTokens.TypeScale.caption).textSelection(.enabled)
                }
                if let head = Self.analyzeHeadKey(empty: model.snapshots.isEmpty, sessions: offersSessionAnalyze) {
                    Text(model.text(head)).insightsNote()
                }
                HStack(spacing: GlassTokens.Space.s4) {
                    GlassSelect(model.text("source"), selection: $source, options: [
                        GlassPickerOption(model.text("codex"), value: "codex"),
                        GlassPickerOption(model.text("claude_code"), value: "claude_code"),
                        GlassPickerOption(model.text("trajectory"), value: "trajectory"),
                    ])
                    .fixedSize()
                    Button(model.text("choose_file")) {
                        if let file = FolderPanel.chooseTraceFile(source: source) {
                            model.analyze(file: file, source: source)
                        }
                    }
                }.disabled(model.busy)
                if let error = model.error { Text(error).foregroundStyle(GlassStatus.outside.textColor) }
                if !model.invalidatedEpisodeIDs.isEmpty {
                    LazyVStack(alignment: .leading, spacing: InsightsLayout.lineGap) {
                        Text(model.text("episode_invalidated_notice"))
                        ForEach(model.invalidatedEpisodeIDs, id: \.self) { id in
                            Text(id).insightsMono()
                        }
                    }.textSelection(.enabled)
                }
            }
            .insightsCardBody()
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .insightsSection()
    }

    /// The line under Analyze's title: how to add a trace, naming only
    /// buttons on screen. Choose file always is; Analyze & save only while
    /// Sessions offers it. Without it, nothing once a trace is saved, as
    /// before Sessions had the action.
    static func analyzeHeadKey(empty: Bool, sessions: Bool) -> String? {
        switch (empty, sessions) {
        case (true, true): return "analytics_analyze_empty"
        case (false, true): return "analytics_analyze_how"
        case (true, false): return "empty"
        case (false, false): return nil
        }
    }

    /// The saved-history summary, its loading line, or its failure.
    @ViewBuilder
    private var summarySection: some View {
        if model.loadingSummary {
            HStack(spacing: GlassTokens.Space.s4) {
                GlassSpinner()
                Text(model.text("summary_title")).insightsNote()
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .insightsSection()
        } else if let summaryError = model.summaryError {
            Text(summaryError).foregroundStyle(GlassStatus.outside.textColor)
                .frame(maxWidth: .infinity, alignment: .leading)
                .insightsSection()
        } else if let summary = model.summary {
            InsightsSummaryView(summary: summary, copy: model.copy, openSnapshot: model.explain)
                .disabled(model.busy)
                .insightsSection()
        }
    }

    /// The trace on screen: its result, then saving or deleting it, its
    /// assessment and its linked evidence.
    private func detailSection(_ insight: LocalInsight) -> some View {
        VStack(alignment: .leading, spacing: InsightsLayout.groupGap) {
            InsightDetail(insight: insight, copy: model.copy)
            HStack(spacing: GlassTokens.Space.s4) {
                if model.selectedIsSaved {
                    Button(model.text("delete"), role: .destructive) { model.delete() }
                        .accessibilityLabel(model.text("delete_accessibility"))
                        .buttonStyle(GlassButtonStyle(.destructive, small: true))
                } else {
                    Button(model.text("save")) { model.save() }
                        .accessibilityLabel(model.text("save_accessibility"))
                    // What saving keeps, beside the action.
                    InsightsInfo(model.text("save"), [model.text("save_notice")])
                }
            }.disabled(model.busy)
            VStack(alignment: .leading, spacing: InsightsLayout.groupGap) {
                if model.selectedIsSaved {
                    assessment
                    InsightEvidenceControls(model: model, insight: insight)
                        .id(insight.id)
                } else {
                    Text(model.text("link_saved_required")).insightsCaption()
                }
            }
            .insightsCardBody()
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .insightsSection()
    }

    /// Every saved trace, newest first; one opens its detail.
    private var savedSection: some View {
        VStack(alignment: .leading, spacing: InsightsLayout.headingGap) {
            InsightsSectionTitle(model.text("saved"))
            LazyVStack(alignment: .leading, spacing: InsightsLayout.headingGap) {
                ForEach(model.snapshots) { insight in
                    Button { model.explain(insight.id) } label: {
                        VStack(alignment: .leading, spacing: InsightsLayout.lineGap) {
                            Text(model.text(insight.source_format))
                            Text(InsightsDate.label(insight.analyzed_at)).insightsCaption()
                            Text(insight.id).insightsMono().lineLimit(1)
                        }.frame(maxWidth: .infinity, alignment: .leading).insightsRowCard()
                    }
                    .buttonStyle(GlassPressStyle())
                    .disabled(model.busy)
                }
            }
            .insightsCardBody()
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .insightsSection()
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
    private func updateSpecificationSources() {
        specificationModel.updateSources(tasks: comparisonModel.tasks, snapshots: model.snapshots)
    }
    private var comparisonTaskVersions: [String] {
        comparisonModel.tasks.map { detail in
            let context = detail.task.context
            return [detail.id, String(detail.task.revision), detail.task.material_digest,
                    context?.configuration_fingerprint ?? "", context?.task_date ?? "",
                    detail.task.outcome?.recorded_at ?? "", detail.task.independence_confirmation?.confirmed_at ?? "",
                    detail.stale_reasons.map(\.rawValue).joined(separator: ",")].joined(separator: ":")
        }
    }
    /// The user's own category and outcome; what they are, and that they
    /// are not verified, behind the heading's "i".
    private var assessment: some View {
        VStack(alignment: .leading, spacing: InsightsLayout.headingGap) {
            InsightsGroupHeading(model.text("assessment"), info: [model.text("assessment_notice")])
            HStack(spacing: GlassTokens.Space.s4) {
                GlassSelect(model.text("category"), selection: $model.assessmentCategory,
                            options: InsightsChoices.categories.map { GlassPickerOption(model.text("category_" + $0), value: $0) })
                GlassSelect(model.text("outcome"), selection: $model.assessmentOutcome,
                            options: InsightsChoices.outcomes.map { GlassPickerOption(model.text("outcome_" + $0), value: $0) })
            }
            HStack(spacing: GlassTokens.Space.s4) {
                Button(model.text("save_assessment")) { model.annotate(category: model.assessmentCategory, outcome: model.assessmentOutcome) }
                Button(model.text("clear_assessment")) { model.clearAnnotation() }
            }
        }.disabled(model.busy)
    }
}

struct InsightDetail: View {
    let insight: LocalInsight
    let copy: [String: String]
    private func text(_ key: String) -> String { copy[key] ?? "" }
    var body: some View {
        VStack(alignment: .leading, spacing: InsightsLayout.groupGap) {
            // What a result can and cannot say, behind the title's "i".
            InsightsSectionTitle(text("result"), info: [
                text("boundary_notice"), text("unknown_notice"), text("coverage_notice"),
            ])
            VStack(alignment: .leading, spacing: InsightsLayout.groupGap) {
                VStack(alignment: .leading, spacing: InsightsLayout.lineGap) {
                    Text(InsightsDate.label(insight.analyzed_at)).insightsCaption()
                    Text("\(text("provider")): \(insight.report.provider.id) · \(insight.report.provider.version)").insightsCaption()
                    Text("\(text("rubric")): \(insight.report.provider.rubric_version) · \(insight.report.provider.execution_mode)")
                        .insightsCaption()
                }
                VStack(alignment: .leading, spacing: InsightsLayout.headingGap) {
                    InsightsGroupHeading(text("summary_metrics"))
                    // Only the metrics this trace recorded (Kristi,
                    // 2026-10-21); what a result cannot say stays in the
                    // title's "i".
                    ForEach(InsightsSummaryFormatting.known(insight.report.metrics)) { metric in
                        VStack(alignment: .leading, spacing: InsightsLayout.lineGap) {
                            Text("\(text("metric_" + metric.id)): \(metric.value.map { $0.formatted() } ?? text("unknown"))")
                            Text("\(text("coverage")): \(metric.coverage.observed.formatted()) / \(metric.coverage.total.formatted())")
                                .insightsCaption()
                        }
                    }
                    Text("\(text("cost")): \(text("unknown"))")
                }
                InsightModelSection(observations: insight.model_observations, copy: copy)
                if let assessment = insight.manual_annotation {
                    VStack(alignment: .leading, spacing: InsightsLayout.headingGap) {
                        InsightsGroupHeading(text("assessment"), info: [text("assessment_notice")])
                        Text("\(text("category_" + assessment.category)) / \(text("outcome_" + assessment.outcome))")
                        Text(InsightsDate.label(assessment.recorded_at)).insightsCaption()
                    }
                }
                VStack(alignment: .leading, spacing: InsightsLayout.headingGap) {
                    InsightsGroupHeading(text("evidence"), info: [text("evidence_notice")])
                    ForEach(insight.report.evidence) { evidence in
                        VStack(alignment: .leading, spacing: InsightsLayout.lineGap) {
                            Text("\(text("evidence")): \(evidence.id)").insightsMono()
                            Text("\(text("source_digest")): \(evidence.source_digest)").insightsMono().textSelection(.enabled)
                        }
                    }
                }
            }
            .insightsCardBody()
        }.textSelection(.enabled)
    }
}

/// The Insights window's tabs that have landed.
enum InsightsTab: Hashable { case overview, patterns, sessions, analyze }

/// The assessment choices, by their wire values; their words are the
/// copy table's `category_` and `outcome_` entries.
enum InsightsChoices {
    static let categories = ["unknown", "refactor", "tests", "docs", "debugging", "other"]
    static let outcomes = ["unknown", "accepted", "partial", "rejected"]
}

enum InsightsDate {
    static func label(_ raw: String) -> String {
        DaemonTimestamp.parse(raw)?.formatted(date: .abbreviated, time: .standard) ?? raw
    }
}
