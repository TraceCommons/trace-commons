import Charts
import SwiftUI
import TCBridge
import TCDesign

/// Sessions (drill-in) over one saved snapshot (feed S). Every word is the
/// core's analytics copy and every figure the core's. Input sent each turn is
/// stacked cache read, uncached and cache write, with a dashed line at the
/// long-context threshold and the core's lettered markers beneath. An
/// unknown turn is a gap, never a zero bar. A Codex session draws no chart
/// and reads "not recorded" (owner decision D11, open). No what-if card is
/// drawn (owner decision D2, open), and no project name is shown (owner
/// decision D7, open).
struct InsightsSessionsTab: View {
    let model: InsightsSessionsModel
    /// Saved snapshots, newest first, for the session picker. Each is shown
    /// by its label, never its ID.
    let snapshots: [LocalInsight]
    let copy: [String: String]

    private func text(_ key: String) -> String { copy[key] ?? "" }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s8) {
                if !snapshots.isEmpty {
                    GlassSelect(text("analytics_drill_session"), selection: Binding(
                        get: { model.selected ?? "" },
                        set: { model.select($0) }
                    ), options: InsightsSessionsWords.pickerChoices(snapshots, copy: copy)
                        .map { GlassPickerOption($0.title, value: $0.value) })
                    .frame(maxWidth: 360)
                }
                if let drill = model.drill {
                    Text(InsightsSessionsWords.header(drill, copy: copy)).insightsTitle()
                    Text(text("analytics_feed_saved")).insightsCaption()
                    if !drill.reasons.isEmpty {
                        Text(drill.reasons.map { InsightsOverviewWords.reason($0, copy: copy) }
                            .joined(separator: " \u{b7} "))
                            .insightsCaption()
                    }
                    if let line = InsightsSessionsWords.unavailableLine(drill, copy: copy) {
                        Text(line).insightsNote()
                    } else {
                        HStack(alignment: .top, spacing: GlassTokens.Space.s6) {
                            chart(drill)
                            markerCards(drill)
                        }
                    }
                } else if model.failed || snapshots.isEmpty {
                    Text(text("analytics_unavailable")).insightsError()
                }
                if model.busy { GlassSpinner() }
            }
            .padding(24)
            .frame(maxWidth: .infinity, alignment: .leading)
            .insightsSurface()
        }
    }

    private func chart(_ drill: InsightsSessionDrill) -> some View {
        let turns = (drill.series ?? []).map(\.ordinal)
        let first = turns.min() ?? 1
        let last = turns.max() ?? 1
        return VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
            Text(text("analytics_input_each_turn")).insightsHeading()
            Chart {
                ForEach(InsightsSessionsWords.segments(drill, copy: copy)) { segment in
                    BarMark(x: .value(text("analytics_turn"), Int(segment.turn)),
                            y: .value(text("analytics_card_tokens"), segment.tokens))
                    .foregroundStyle(by: .value(text("analytics_card_tokens"), segment.series))
                }
                RuleMark(y: .value(text("analytics_pattern_long_context"), drill.long_context_threshold))
                    .lineStyle(StrokeStyle(lineWidth: 1, dash: [4, 3]))
                    .foregroundStyle(GlassStatus.outside.textColor)
                    .annotation(position: .top, alignment: .trailing) {
                        Text(InsightsOverviewWords.figure(drill.long_context_threshold, copy: copy))
                            .insightsCaption()
                    }
                ForEach(drill.markers) { marker in
                    PointMark(x: .value(text("analytics_turn"), Int(marker.turn_ordinal)), y: .value(text("analytics_card_tokens"), 0))
                        .symbolSize(0)
                        .annotation(position: .bottom) {
                            Text(marker.letter).insightsMono()
                        }
                }
            }
            .chartForegroundStyleScale([
                text("analytics_series_cache_read"): GlassColor.accentText,
                text("analytics_series_uncached"): GlassColor.textSecondary,
                text("analytics_series_cache_write"): GlassColor.accentText.opacity(0.4),
            ])
            .chartXAxis(.hidden)
            .chartXScale(domain: Int(first)...Int(max(last, first)))
            .frame(height: 260)
            .accessibilityHidden(true)
            HStack {
                Text(InsightsSessionsWords.turnLabel(first, copy: copy)).insightsCaption()
                Spacer()
                Text(InsightsSessionsWords.turnLabel(last, copy: copy)).insightsCaption()
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .insightsCard()
    }

    private func markerCards(_ drill: InsightsSessionDrill) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
            ForEach(drill.markers) { marker in
                let lines = InsightsSessionsWords.markerLines(marker, threshold: drill.long_context_threshold,
                                                              copy: copy)
                HStack(alignment: .top, spacing: GlassTokens.Space.s4) {
                    Text(marker.letter).insightsMono()
                    VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                        ForEach(Array(lines.enumerated()), id: \.offset) { index, line in
                            if index == 0 {
                                Text(line).insightsHeading()
                            } else {
                                Text(line).insightsNote()
                            }
                        }
                    }
                }
                .accessibilityElement(children: .combine)
                .frame(maxWidth: .infinity, alignment: .leading)
                .insightsRowCard()
            }
        }
        .frame(width: 320, alignment: .topLeading)
    }
}
