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
    @State private var tab: InsightsTab
    /// "Turn off" on the weekly summary card: the daemon's setting, its only
    /// switch. `nil` without a daemon.
    private let turnOffRecapCard: (@Sendable () async throws -> Void)?
    private let storeSelection: InsightsStoreSelection
    private let storeCopy: [String: String]
    @State private var choosingFile = false
    @State private var source = "codex"

    /// `daemon` is the app's live client, for the week from the daemon's
    /// counter pass (feed T); without one the window shows saved imports.
    @MainActor init(storeSelection: InsightsStoreSelection = .standard,
                    storeCopy: [String: String]? = TCInsights.copy(),
                    daemon: (any DaemonDataClient)? = nil) {
        let router = InsightsServiceRouter(selection: storeSelection)
        let service: InsightsModel.Service = { request in try await router.call(request) }
        let weekReader: InsightsModel.WeekReader? = daemon.map { client in
            { isoWeek in try await client.insightsWeek(isoWeek: isoWeek) }
        }
        let turnOff: (@Sendable () async throws -> Void)? = daemon.map { client in
            { _ = try await client.setInsightsRecapCard(false) }
        }
        self.init(storeSelection: storeSelection, storeCopy: storeCopy,
                  model: InsightsModel(service: service, weekReader: weekReader),
                  comparisonModel: ComparisonTasksModel(service: service),
                  specificationModel: ComparisonSpecificationsModel(service: service),
                  turnOffRecapCard: turnOff)
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
                    initialTab: InsightsTab = .overview,
                    turnOffRecapCard: (@Sendable () async throws -> Void)? = nil) {
        self.storeSelection = storeSelection
        self.storeCopy = storeCopy ?? [:]
        _model = State(initialValue: model)
        _comparisonModel = State(initialValue: comparisonModel)
        _specificationModel = State(initialValue: specificationModel)
        _overviewModel = State(initialValue: overviewModel ?? InsightsOverviewModel(service: model.service))
        _patternsModel = State(initialValue: InsightsPatternsModel(service: model.service))
        _sessionsModel = State(initialValue: InsightsSessionsModel(service: model.service))
        _comparisonsModel = State(initialValue: InsightsComparisonsModel(service: model.service))
        _tab = State(initialValue: initialTab)
        self.turnOffRecapCard = turnOffRecapCard
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

    /// The tab container. Spend is shown disabled with its Later chip.
    /// The models open and close with the container, not with a tab.
    private var tabs: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: GlassTokens.Space.s6) {
                GlassSegmentedTabs(model.text("title"), selection: $tab, segments: [
                    GlassSegment(model.text("analytics_tab_overview"), value: InsightsTab.overview),
                    GlassSegment(model.text("analytics_tab_patterns"), value: InsightsTab.patterns),
                    GlassSegment(model.text("analytics_tab_sessions"), value: InsightsTab.sessions),
                    GlassSegment(model.text("analytics_tab_analyze"), value: InsightsTab.analyze),
                ])
                .frame(maxWidth: 480)
                HStack(spacing: GlassTokens.Space.s3) {
                    Text(model.text("analytics_tab_spend")).foregroundStyle(GlassColor.textTertiary)
                    GlassChip(glass: model.text("analytics_later"), muted: true)
                }
                .accessibilityElement(children: .combine)
                Spacer()
            }
            .padding(.horizontal, 24).padding(.top, 16)
            switch tab {
            case .overview:
                InsightsOverviewTab(
                    model: overviewModel, comparisons: comparisonsModel, copy: model.copy,
                    snapshots: model.snapshots,
                    notice: model.counterPassNoticeKey.map(model.text),
                    selectWeek: selectWeek,
                    openRecap: { if let week = comparisonsModel.openRecap() { selectWeek(week) } },
                    turnOffRecap: turnOffRecapCard.map { turnOff in
                        { Task { try? await turnOff(); await model.loadWeek() } }
                    },
                    showReads: { tab = .patterns })
            case .patterns:
                InsightsPatternsTab(model: patternsModel, comparisons: comparisonsModel, copy: model.copy,
                                    selectWeek: selectWeek)
            case .sessions:
                InsightsSessionsTab(model: sessionsModel, snapshots: model.snapshots, copy: model.copy)
            case .analyze: content
            }
        }
        .onAppear { model.open() }
        .onAppear { comparisonModel.open() }
        .onAppear { specificationModel.open() }
        .onAppear { overviewModel.open() }
        .onAppear { patternsModel.open() }
        .onAppear { sessionsModel.open(); sessionsModel.sync(snapshotIDs: model.snapshots.map(\.id)) }
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
        }
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

    /// The Analyze tab: the whole screen as it was before the tabs.
    private var content: some View {
        ScrollViewReader { proxy in
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    if let location = Self.storeLocationLine(storeSelection, copy: storeCopy) {
                        Text(location)
                            .glassType(GlassTokens.TypeScale.caption).textSelection(.enabled)
                    }
                    Text(model.text("intro"))
                    HStack {
                        GlassSelect(model.text("source"), selection: $source, options: [
                            GlassPickerOption(model.text("codex"), value: "codex"),
                            GlassPickerOption(model.text("claude_code"), value: "claude_code"),
                            GlassPickerOption(model.text("trajectory"), value: "trajectory"),
                        ])
                        Button(model.text("choose_file")) { choosingFile = true }
                        // Short labels repeat on this screen; each is named in full
                        // for VoiceOver, in the core's words.
                        Button(model.text("refresh")) { model.refresh() }
                            .accessibilityLabel(model.text("refresh_accessibility"))
                        if model.busy { GlassSpinner() }
                    }.disabled(model.busy)
                    if let error = model.error { Text(error).foregroundStyle(GlassStatus.outside.textColor) }
                    if !model.invalidatedEpisodeIDs.isEmpty {
                        VStack(alignment: .leading, spacing: 6) {
                            Text(model.text("episode_invalidated_notice"))
                            ForEach(model.invalidatedEpisodeIDs, id: \.self) { id in
                                Text(id).insightsMono()
                            }
                        }.textSelection(.enabled)
                    }
                    Text(model.text("snapshot_notice"))
                        .insightsNote()
                    if model.loadingSummary {
                        HStack(spacing: GlassTokens.Space.s4) {
                            GlassSpinner()
                            Text(model.text("summary_title")).insightsNote()
                        }
                    } else if let summaryError = model.summaryError {
                        Text(summaryError).foregroundStyle(GlassStatus.outside.textColor)
                    } else if let summary = model.summary {
                        InsightsSummaryView(summary: summary, copy: model.copy, openSnapshot: model.explain)
                            .disabled(model.busy)
                    }
                    InsightsRule()
                    if let insight = model.selected {
                        InsightDetail(insight: insight, copy: model.copy)
                            .id("insight-detail")
                        HStack {
                            if model.selectedIsSaved {
                                Button(model.text("delete"), role: .destructive) { model.delete() }
                                    .accessibilityLabel(model.text("delete_accessibility"))
                                    .buttonStyle(GlassButtonStyle(.destructive, small: true))
                            } else {
                                VStack(alignment: .leading) {
                                    Text(model.text("save_notice")).glassType(GlassTokens.TypeScale.caption)
                                    Button(model.text("save")) { model.save() }
                                        .accessibilityLabel(model.text("save_accessibility"))
                                }
                            }
                        }.disabled(model.busy)
                        if model.selectedIsSaved {
                            assessment
                            InsightEvidenceControls(model: model, insight: insight)
                                .id(insight.id)
                        } else {
                            Text(model.text("link_saved_required")).glassType(GlassTokens.TypeScale.caption)
                        }
                    }
                    InsightsRule()
                    InsightsEpisodesView(model: model)
                    InsightsRule()
                    ComparisonTasksView(model: comparisonModel, episodes: model.episodes, copy: model.copy,
                                        openEpisode: model.openEpisode, openSnapshot: model.explain)
                    InsightsRule()
                    ComparisonSpecificationsView(model: specificationModel, copy: model.copy,
                                                 openTask: comparisonModel.select)
                    InsightsRule()
                    InsightCardsView(model: model)
                    InsightsRule()
                    Text(model.text("saved")).insightsHeading()
                    if model.snapshots.isEmpty { Text(model.text("empty")) }
                    ForEach(model.snapshots) { insight in
                        Button { model.explain(insight.id) } label: {
                            VStack(alignment: .leading) {
                                Text(model.text(insight.source_format))
                                Text(InsightsDate.label(insight.analyzed_at)).glassType(GlassTokens.TypeScale.caption)
                                Text(insight.id).insightsMono().lineLimit(1)
                            }.frame(maxWidth: .infinity, alignment: .leading).insightsRowCard()
                        }
                        .buttonStyle(GlassPressStyle())
                        .disabled(model.busy)
                    }
                    Text(model.text("cancellation_notice"))
                        .insightsCaption()
                }.padding(24).frame(maxWidth: .infinity, alignment: .leading)
                .insightsSurface()
            }
            .onChange(of: model.selected?.id) { _, id in
                if id != nil { proxy.scrollTo("insight-detail", anchor: .top) }
            }
        }
        .fileImporter(isPresented: $choosingFile, allowedContentTypes: [.data]) { result in
            if case .success(let file) = result { model.analyze(file: file, source: source) }
        }
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
    private var assessment: some View {
        VStack(alignment: .leading) {
            Text(model.text("assessment_notice"))
            HStack {
                GlassSelect(model.text("category"), selection: $model.assessmentCategory,
                            options: InsightsChoices.categories.map { GlassPickerOption(model.text("category_" + $0), value: $0) })
                GlassSelect(model.text("outcome"), selection: $model.assessmentOutcome,
                            options: InsightsChoices.outcomes.map { GlassPickerOption(model.text("outcome_" + $0), value: $0) })
                Button(model.text("save_assessment")) { model.annotate(category: model.assessmentCategory, outcome: model.assessmentOutcome) }
                Button(model.text("clear_assessment")) { model.clearAnnotation() }
            }.disabled(model.busy)
        }
    }
}

struct InsightDetail: View {
    let insight: LocalInsight
    let copy: [String: String]
    private func text(_ key: String) -> String { copy[key] ?? "" }
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(text("result")).insightsTitle()
            Text(InsightsDate.label(insight.analyzed_at))
            Text(text("boundary_notice"))
            Text("\(text("provider")): \(insight.report.provider.id) · \(insight.report.provider.version)")
            Text("\(text("rubric")): \(insight.report.provider.rubric_version) · \(insight.report.provider.execution_mode)")
            ForEach(insight.report.metrics) { metric in
                VStack(alignment: .leading) {
                    Text("\(text("metric_" + metric.id)): \(metric.value.map { $0.formatted() } ?? text("unknown"))")
                    Text("\(text("coverage")): \(metric.coverage.observed.formatted()) / \(metric.coverage.total.formatted())")
                        .insightsCaption()
                }
            }
            InsightModelSection(observations: insight.model_observations, copy: copy)
            Text(text("unknown_notice"))
            Text(text("coverage_notice"))
            Text("\(text("cost")): \(text("unknown"))")
            if let assessment = insight.manual_annotation {
                Text("\(text("assessment")): \(text("category_" + assessment.category)) / \(text("outcome_" + assessment.outcome))")
                Text(text("assessment_notice")).glassType(GlassTokens.TypeScale.caption)
                Text(InsightsDate.label(assessment.recorded_at)).glassType(GlassTokens.TypeScale.caption)
            }
            VStack(alignment: .leading, spacing: 8) {
                Text(text("evidence")).insightsHeading()
                InsightsRule()
                VStack(alignment: .leading, spacing: 8) {
                    Text(text("evidence_notice"))
                    ForEach(insight.report.evidence) { evidence in
                        Text("\(text("evidence")): \(evidence.id)").insightsMono()
                        Text("\(text("source_digest")): \(evidence.source_digest)").insightsMono().textSelection(.enabled)
                    }
                }
            }
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
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        let date = formatter.date(from: raw) ?? ISO8601DateFormatter().date(from: raw)
        return date?.formatted(date: .abbreviated, time: .standard) ?? raw
    }
}
