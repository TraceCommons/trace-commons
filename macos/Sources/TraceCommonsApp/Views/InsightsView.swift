import SwiftUI
import TCBridge
import UniformTypeIdentifiers

struct InsightsView: View {
    @State private var model = InsightsModel()
    @State private var comparisonModel = ComparisonTasksModel()
    @State private var choosingFile = false
    @State private var source = "codex"

    var body: some View {
        ScrollViewReader { proxy in
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    Text(model.text("intro"))
                    HStack {
                        Picker(model.text("source"), selection: $source) {
                            Text(model.text("codex")).tag("codex")
                            Text(model.text("trajectory")).tag("trajectory")
                        }.frame(maxWidth: 260)
                        Button(model.text("choose_file")) { choosingFile = true }
                        Button(model.text("refresh")) { model.refresh() }
                        if model.busy { ProgressView().controlSize(.small) }
                    }.disabled(model.busy)
                    if let error = model.error { Text(error).foregroundStyle(.red) }
                    if !model.invalidatedEpisodeIDs.isEmpty {
                        VStack(alignment: .leading, spacing: 6) {
                            Text(model.text("episode_invalidated_notice"))
                            ForEach(model.invalidatedEpisodeIDs, id: \.self) { id in
                                Text(id).font(.caption.monospaced())
                            }
                        }.textSelection(.enabled)
                    }
                    Text(model.text("snapshot_notice"))
                        .font(.callout).foregroundStyle(.secondary)
                    if model.loadingSummary {
                        ProgressView(model.text("summary_title"))
                    } else if let summaryError = model.summaryError {
                        Text(summaryError).foregroundStyle(.red)
                    } else if let summary = model.summary {
                        InsightsSummaryView(summary: summary, copy: model.copy, openSnapshot: model.explain)
                            .disabled(model.busy)
                    }
                    Divider()
                    if let insight = model.selected {
                        InsightDetail(insight: insight, copy: model.copy)
                            .id("insight-detail")
                        HStack {
                            if model.selectedIsSaved {
                                Button(model.text("delete"), role: .destructive) { model.delete() }
                            } else {
                                VStack(alignment: .leading) {
                                    Text(model.text("save_notice")).font(.caption)
                                    Button(model.text("save")) { model.save() }
                                }
                            }
                        }.disabled(model.busy)
                        if model.selectedIsSaved {
                            assessment
                            InsightEvidenceControls(model: model, insight: insight)
                                .id(insight.id)
                        } else {
                            Text(model.text("link_saved_required")).font(.caption)
                        }
                    }
                    Divider()
                    InsightsEpisodesView(model: model)
                    Divider()
                    ComparisonTasksView(model: comparisonModel, episodes: model.episodes, copy: model.copy)
                    Divider()
                    InsightCardsView(model: model)
                    Divider()
                    Text(model.text("saved")).font(.headline)
                    if model.snapshots.isEmpty { Text(model.text("empty")) }
                    ForEach(model.snapshots) { insight in
                        Button { model.explain(insight.id) } label: {
                            VStack(alignment: .leading) {
                                Text(model.text(insight.source_format))
                                Text(InsightsDate.label(insight.analyzed_at)).font(.caption)
                                Text(insight.id).font(.caption.monospaced()).lineLimit(1)
                            }.frame(maxWidth: .infinity, alignment: .leading)
                        }.disabled(model.busy)
                    }
                    Text(model.text("cancellation_notice"))
                        .font(.caption).foregroundStyle(.secondary)
                }.padding(24).frame(maxWidth: .infinity, alignment: .leading)
            }
            .onChange(of: model.selected?.id) { _, id in
                if id != nil { proxy.scrollTo("insight-detail", anchor: .top) }
            }
        }
        .fileImporter(isPresented: $choosingFile, allowedContentTypes: [.data]) { result in
            if case .success(let file) = result { model.analyze(file: file, source: source) }
        }
        .onAppear { model.open() }
        .onAppear { comparisonModel.open() }
        .onDisappear { model.close(); comparisonModel.close() }
    }
    private var assessment: some View {
        VStack(alignment: .leading) {
            Text(model.text("assessment_notice"))
            HStack {
                Picker(model.text("category"), selection: $model.assessmentCategory) {
                    ForEach(["unknown", "refactor", "tests", "docs", "debugging", "other"], id: \.self) {
                        Text(model.text("category_" + $0)).tag($0)
                    }
                }
                Picker(model.text("outcome"), selection: $model.assessmentOutcome) {
                    ForEach(["unknown", "accepted", "partial", "rejected"], id: \.self) {
                        Text(model.text("outcome_" + $0)).tag($0)
                    }
                }
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
            Text(text("result")).font(.title2)
            Text(InsightsDate.label(insight.analyzed_at))
            Text(text("boundary_notice"))
            Text("\(text("provider")): \(insight.report.provider.id) · \(insight.report.provider.version)")
            Text("\(text("rubric")): \(insight.report.provider.rubric_version) · \(insight.report.provider.execution_mode)")
            ForEach(insight.report.metrics) { metric in
                VStack(alignment: .leading) {
                    Text("\(text("metric_" + metric.id)): \(metric.value.map { $0.formatted() } ?? text("unknown"))")
                    Text("\(text("coverage")): \(metric.coverage.observed.formatted()) / \(metric.coverage.total.formatted())")
                        .font(.caption).foregroundStyle(.secondary)
                }
            }
            InsightModelSection(observations: insight.model_observations, copy: copy)
            Text(text("unknown_notice"))
            Text(text("coverage_notice"))
            Text("\(text("cost")): \(text("unknown"))")
            if let assessment = insight.manual_annotation {
                Text("\(text("assessment")): \(text("category_" + assessment.category)) / \(text("outcome_" + assessment.outcome))")
                Text(text("assessment_notice")).font(.caption)
                Text(InsightsDate.label(assessment.recorded_at)).font(.caption)
            }
            VStack(alignment: .leading, spacing: 8) {
                Text(text("evidence")).font(.headline)
                Divider()
                VStack(alignment: .leading, spacing: 8) {
                    Text(text("evidence_notice"))
                    ForEach(insight.report.evidence) { evidence in
                        Text("\(text("evidence")): \(evidence.id)").font(.caption.monospaced())
                        Text("\(text("source_digest")): \(evidence.source_digest)").font(.caption.monospaced()).textSelection(.enabled)
                    }
                }
            }
        }.textSelection(.enabled)
    }
}

enum InsightsDate {
    static func label(_ raw: String) -> String {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        let date = formatter.date(from: raw) ?? ISO8601DateFormatter().date(from: raw)
        return date?.formatted(date: .abbreviated, time: .standard) ?? raw
    }
}
