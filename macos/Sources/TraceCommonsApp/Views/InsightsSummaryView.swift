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
        VStack(alignment: .leading, spacing: InsightsLayout.groupGap) {
            // What the summary covers, who computed it, and what it cannot
            // say, behind the title's "i".
            InsightsSectionTitle(text("summary_title"), info: [
                text("summary_scope"),
                "\(text("provider")): \(summary.provider.id) · \(summary.provider.version)",
                "\(text("rubric")): \(summary.provider.rubric_version)",
            ] + summary.limitations.map { text("summary_limitation_" + $0.rawValue) })
            VStack(alignment: .leading, spacing: InsightsLayout.groupGap) {
                Text("\(text("summary_analysis_range")): \(analysisRange)").insightsCaption()
                VStack(alignment: .leading, spacing: InsightsLayout.lineGap) {
                    SummaryCountRow(label: text("summary_snapshots"), count: summary.saved_snapshots)
                    if summary.saved_snapshots == 0 { Text(text("summary_empty")).insightsNote() }
                    SummaryCountRow(label: text("summary_assessed"), count: summary.user_reported.assessed_snapshots)
                    SummaryCountRow(label: text("summary_unassessed"), count: summary.user_reported.unassessed_snapshots)
                }
                HStack(alignment: .top, spacing: InsightsLayout.groupGap) {
                    categories
                    outcomes
                }
                VStack(alignment: .leading, spacing: InsightsLayout.headingGap) {
                    InsightsGroupHeading(text("summary_metrics"))
                    ForEach(summary.metrics) { metric in
                        metricView(metric)
                    }
                }
                LazyVStack(alignment: .leading, spacing: InsightsLayout.headingGap) {
                    InsightsGroupHeading(text("summary_evidence"))
                    if summary.snapshots.isEmpty { Text(text("summary_no_evidence")).insightsNote() }
                    ForEach(summary.snapshots) { snapshot in
                        #if DEBUG
                        let _ = OtherCollectionRowBodies.record("insightsSummarySnapshots", snapshot.id)
                        #endif
                        Button { openSnapshot(snapshot.id) } label: {
                            VStack(alignment: .leading, spacing: InsightsLayout.lineGap) {
                                Text("\(text("summary_open_snapshot")) · \(text(snapshot.source_format))")
                                Text(InsightsDate.label(snapshot.analyzed_at)).insightsCaption()
                                Text(snapshot.id).insightsMono().lineLimit(2)
                            }.frame(maxWidth: .infinity, alignment: .leading).insightsRowCard()
                        }
                        .buttonStyle(GlassPressStyle())
                    }
                }
            }
            .insightsCardBody()
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func metricView(_ metric: SavedInsightsSummary.Metric) -> some View {
        VStack(alignment: .leading, spacing: InsightsLayout.lineGap) {
            Text(text("metric_" + metric.id))
            Text("\(text("summary_observed_sum")): \(InsightsSummaryFormatting.value(metric.observed_value_sum, unknown: text("unknown")))")
                .monospacedDigit()
            Text("\(text("summary_available")): \(metric.available_snapshots.formatted()) · \(text("summary_missing")): \(metric.missing_snapshots.formatted())")
                .insightsCaption()
            Text("\(text("summary_record_coverage")): \(metric.record_coverage.observed.formatted()) / \(metric.record_coverage.total.formatted()) \(text("summary_unit_" + metric.coverage_unit.rawValue))")
                .insightsCaption()
            evidenceLinks(metric.evidence_snapshot_ids)
        }
    }

    private var analysisRange: String {
        guard let range = summary.snapshot_analysis_range else { return text("unknown") }
        return "\(InsightsDate.label(range.oldest)) to \(InsightsDate.label(range.newest))"
    }

    private var categories: some View {
        VStack(alignment: .leading, spacing: InsightsLayout.headingGap) {
            InsightsGroupHeading(text("summary_categories"))
            ForEach(summary.user_reported.categories) { category in
                SummaryCountRow(label: text("category_" + category.category), count: category.snapshots)
                evidenceLinks(category.evidence_snapshot_ids)
            }
        }.frame(maxWidth: .infinity, alignment: .topLeading)
    }

    private var outcomes: some View {
        VStack(alignment: .leading, spacing: InsightsLayout.headingGap) {
            InsightsGroupHeading(text("summary_outcomes"))
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
                LazyVStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                    ForEach(ids, id: \.self) { id in
                        Button { openSnapshot(id) } label: {
                            Text(id).insightsMono().lineLimit(2)
                        }
                        .buttonStyle(GlassButtonStyle(.link))
                        .accessibilityLabel("\(text("summary_open_snapshot")) \(id)")
                    }
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
