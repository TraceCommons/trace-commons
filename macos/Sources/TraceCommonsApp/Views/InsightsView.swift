import SwiftUI
import TCBridge
import UniformTypeIdentifiers

struct InsightsView: View {
    @State private var model = InsightsModel()
    @State private var choosingFile = false
    @State private var source = "codex"

    var body: some View {
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
                Text(model.text("snapshot_notice"))
                    .font(.callout).foregroundStyle(.secondary)
                if let insight = model.selected {
                    InsightDetail(insight: insight, copy: model.copy)
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
                    }
                }
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
        .fileImporter(isPresented: $choosingFile, allowedContentTypes: [.data]) { result in
            if case .success(let file) = result { model.analyze(file: file, source: source) }
        }
        .onAppear { model.open() }
        .onDisappear { model.close() }
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
