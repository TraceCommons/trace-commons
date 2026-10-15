import SwiftUI
import TCDesign
import TCBridge

/// Layout and localized formatting only; every count and denominator comes from
/// the shared reducer. Evidence buttons use this summary's own snapshot IDs.
struct InsightsSummaryView: View {
    let summary: SavedInsightsSummary
    let copy: [String: String]
    let openSnapshot: (String) -> Void
    private func text(_ key: String) -> String { copy[key] ?? "" }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text(text("summary_title")).insightsTitle()
            Text(text("summary_scope"))
            SummaryCountRow(label: text("summary_snapshots"), count: summary.saved_snapshots)
            if summary.saved_snapshots == 0 { Text(text("summary_empty")) }
            Text("\(text("provider")): \(summary.provider.id) · \(summary.provider.version)")
                .glassType(GlassTokens.TypeScale.caption)
            Text("\(text("rubric")): \(summary.provider.rubric_version)").glassType(GlassTokens.TypeScale.caption)
            Text("\(text("summary_analysis_range")): \(analysisRange)").glassType(GlassTokens.TypeScale.caption)
            SummaryCountRow(label: text("summary_assessed"), count: summary.user_reported.assessed_snapshots)
            SummaryCountRow(label: text("summary_unassessed"), count: summary.user_reported.unassessed_snapshots)
            HStack(alignment: .top, spacing: 24) {
                categories
                outcomes
            }
            Text(text("summary_metrics")).insightsHeading()
            ForEach(summary.metrics) { metric in
                VStack(alignment: .leading, spacing: 4) {
                    Text(text("metric_" + metric.id)).insightsHeading()
                    Text("\(text("summary_observed_sum")): \(InsightsSummaryFormatting.value(metric.observed_value_sum, unknown: text("unknown")))")
                    Text("\(text("summary_available")): \(metric.available_snapshots.formatted()) · \(text("summary_missing")): \(metric.missing_snapshots.formatted())")
                        .glassType(GlassTokens.TypeScale.caption)
                    Text("\(text("summary_record_coverage")): \(metric.record_coverage.observed.formatted()) / \(metric.record_coverage.total.formatted()) \(text("summary_unit_" + metric.coverage_unit.rawValue))")
                        .glassType(GlassTokens.TypeScale.caption)
                    evidenceLinks(metric.evidence_snapshot_ids)
                }
            }
            Text(text("summary_limitations")).insightsHeading()
            ForEach(summary.limitations, id: \.self) { limitation in
                Text(text("summary_limitation_" + limitation.rawValue)).glassType(GlassTokens.TypeScale.label.weight(.regular))
            }
            Text(text("summary_evidence")).insightsHeading()
            if summary.snapshots.isEmpty { Text(text("summary_no_evidence")) }
            ForEach(summary.snapshots) { snapshot in
                Button { openSnapshot(snapshot.id) } label: {
                    VStack(alignment: .leading) {
                        Text("\(text("summary_open_snapshot")) · \(text(snapshot.source_format))")
                        Text(InsightsDate.label(snapshot.analyzed_at)).glassType(GlassTokens.TypeScale.caption)
                        Text(snapshot.id).insightsMono().lineLimit(2)
                    }.frame(maxWidth: .infinity, alignment: .leading).insightsRowCard()
                }
                .buttonStyle(GlassPressStyle())
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var analysisRange: String {
        guard let range = summary.snapshot_analysis_range else { return text("unknown") }
        return "\(InsightsDate.label(range.oldest)) to \(InsightsDate.label(range.newest))"
    }

    private var categories: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(text("summary_categories")).insightsHeading()
            ForEach(summary.user_reported.categories) { category in
                SummaryCountRow(label: text("category_" + category.category), count: category.snapshots)
                evidenceLinks(category.evidence_snapshot_ids)
            }
        }.frame(maxWidth: .infinity, alignment: .topLeading)
    }

    private var outcomes: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(text("summary_outcomes")).insightsHeading()
            ForEach(summary.user_reported.outcomes) { outcome in
                SummaryCountRow(label: text("outcome_" + outcome.outcome), count: outcome.snapshots)
                evidenceLinks(outcome.evidence_snapshot_ids)
            }
        }.frame(maxWidth: .infinity, alignment: .topLeading)
    }

    @ViewBuilder
    private func evidenceLinks(_ ids: [String]) -> some View {
        if !ids.isEmpty {
            InsightsDisclosure(text("summary_evidence")) {
                ForEach(ids, id: \.self) { id in
                    Button { openSnapshot(id) } label: {
                        Text(id).insightsMono().lineLimit(2)
                    }
                    .buttonStyle(GlassButtonStyle(.link))
                    .accessibilityLabel("\(text("summary_open_snapshot")) \(id)")
                }
            }.glassType(GlassTokens.TypeScale.caption)
        }
    }
}

private struct SummaryCountRow: View {
    let label: String
    let count: UInt64
    var body: some View {
        HStack(alignment: .firstTextBaseline) {
            Text(label)
            Spacer(minLength: 12)
            Text(count.formatted()).monospacedDigit()
        }.accessibilityElement(children: .combine)
    }
}

enum InsightsSummaryFormatting {
    static func value(_ value: UInt64?, unknown: String) -> String {
        value?.formatted() ?? unknown
    }
}
