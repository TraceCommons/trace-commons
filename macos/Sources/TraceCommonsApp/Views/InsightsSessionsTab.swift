import Charts
import SwiftUI
import TCBridge
import TCDesign

/// Sessions (drill-in) over one saved snapshot (feed S). Every word is the
/// core's analytics copy and every figure the core's. Input sent each turn is
/// stacked cache read, cache write and uncached on the chart surface, with a
/// dashed line at the long-context threshold and the core's lettered markers
/// drawn at their turns. An unknown turn is a gap, never a zero bar. A Codex
/// session draws no chart and reads "not recorded" (owner decision D11,
/// open). No what-if card is drawn (owner decision D2, open), and no project
/// name is shown (owner decision D7, open).
///
/// Laid out for the main pane (direction B, 2026-10-15): the chart takes
/// the pane's width and the markers are listed under it. Hovering a turn,
/// or pressing a marker, reads that turn out in the legend.
struct InsightsSessionsTab: View {
    let model: InsightsSessionsModel
    /// Saved snapshots, newest first, for the session picker. Each is shown
    /// by its label, never its ID.
    let snapshots: [LocalInsight]
    let copy: [String: String]
    /// The turn being read; `nil` reads the whole session.
    @State private var readTurn: Int?

    private func text(_ key: String) -> String { copy[key] ?? "" }
    private func figure(_ value: UInt64?) -> String { InsightsOverviewWords.figure(value, copy: copy) }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: InsightsLayout.sectionGap) {
                if !snapshots.isEmpty {
                    GlassSelect(text("analytics_drill_session"), selection: Binding(
                        get: { model.selected ?? "" },
                        set: { model.select($0); readTurn = nil }
                    ), options: InsightsSessionsWords.pickerChoices(snapshots, copy: copy)
                        .map { GlassPickerOption($0.title, value: $0.value) })
                }
                if let drill = model.drill {
                    header(drill)
                    if let line = InsightsSessionsWords.unavailableLine(drill, copy: copy) {
                        Text(line).insightsNote()
                    } else {
                        chart(drill)
                        LazyVGrid(columns: InsightsLayout.columns, alignment: .leading,
                                  spacing: GlassTokens.Space.cardGap) {
                            ForEach(drill.markers) { marker in
                                markerCard(marker, threshold: drill.long_context_threshold)
                            }
                        }
                    }
                } else if model.failed || snapshots.isEmpty {
                    Text(text("analytics_unavailable")).insightsError()
                }
                if model.busy { GlassSpinner() }
            }
            .padding(.vertical, GlassTokens.Space.s7)
            .frame(maxWidth: .infinity, alignment: .leading)
            .insightsSurface()
        }
    }

    private func header(_ drill: InsightsSessionDrill) -> some View {
        HStack(alignment: .top, spacing: GlassTokens.Space.s5) {
            InsightsHarnessTile(source: drill.source ?? "")
            VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                Text(InsightsSessionsWords.header(drill, copy: copy)).insightsTitle()
                    .fixedSize(horizontal: false, vertical: true)
                Text(text("analytics_feed_saved")).insightsCaption()
                if !drill.reasons.isEmpty {
                    Text(drill.reasons.map { InsightsOverviewWords.reason($0, copy: copy) }
                        .joined(separator: " \u{b7} "))
                        .insightsCaption()
                }
            }
        }
    }

    /// The stacked segments in stack order: cache read, cache write,
    /// uncached, from the axis up.
    private func stacked(_ drill: InsightsSessionDrill) -> [InsightsSessionsWords.Segment] {
        let rank = Dictionary(uniqueKeysWithValues: InsightsSeries.stack.enumerated().map { (text($1.key), $0) })
        return InsightsSessionsWords.segments(drill, copy: copy)
            .sorted { ($0.turn, rank[$0.series] ?? 0) < ($1.turn, rank[$1.series] ?? 0) }
    }

    /// The legend's figure for one series: the turn being read, or the
    /// session's known turns together. An unknown turn reads the dash.
    private func legendValue(_ series: InsightsSeries, drill: InsightsSessionDrill) -> String {
        let turns = drill.series ?? []
        func value(_ turn: InsightsDrillTurn) -> UInt64? {
            guard let read = turn.cache_read, let uncached = turn.uncached, let write = turn.cache_write else { return nil }
            switch series {
            case .cacheRead: return UInt64(read)
            case .uncached: return UInt64(uncached)
            case .cacheWrite: return write
            case .output: return nil
            }
        }
        if let readTurn {
            return figure(turns.first { Int($0.ordinal) == readTurn }.flatMap(value))
        }
        return figure(turns.compactMap(value).reduce(0, +))
    }

    private func chart(_ drill: InsightsSessionDrill) -> some View {
        let turns = (drill.series ?? []).map(\.ordinal)
        let first = turns.min() ?? 1
        let last = turns.max() ?? 1
        let segments = stacked(drill)
        let tallest = (drill.series ?? []).compactMap { turn -> UInt64? in
            guard let read = turn.cache_read, let uncached = turn.uncached, let write = turn.cache_write else { return nil }
            return UInt64(read) + UInt64(uncached) + write
        }.max() ?? 0
        // Room above the tallest bar and the threshold for the marker letters.
        let top = Double(max(tallest, drill.long_context_threshold)) * 1.18
        let series: [InsightsSeries] = [.cacheRead, .uncached, .cacheWrite]
        return VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
            HStack(alignment: .firstTextBaseline) {
                Text(text("analytics_input_each_turn")).insightsHeading()
                Spacer(minLength: GlassTokens.Space.s4)
                if let readTurn {
                    Text(InsightsSessionsWords.turnLabel(UInt32(readTurn), copy: copy)).insightsCaption()
                }
            }
            LazyVGrid(columns: [GridItem(.adaptive(minimum: 150), spacing: GlassTokens.Space.s3)],
                      spacing: GlassTokens.Space.s3) {
                ForEach(series, id: \.self) { item in
                    GlassLegendCell(text(item.key), value: legendValue(item, drill: drill), series: item.color)
                }
            }
            Chart {
                if let readTurn {
                    RectangleMark(x: .value(text("analytics_turn"), readTurn), width: .ratio(1))
                        .foregroundStyle(GlassColor.ink(0.08))
                }
                ForEach(segments) { segment in
                    BarMark(x: .value(text("analytics_turn"), Int(segment.turn)),
                            y: .value(text("analytics_card_tokens"), segment.tokens))
                    .foregroundStyle(by: .value(text("analytics_card_tokens"), segment.series))
                }
                RuleMark(y: .value(text("analytics_pattern_long_context"), drill.long_context_threshold))
                    .lineStyle(StrokeStyle(lineWidth: 1.5, dash: [4, 3]))
                    .foregroundStyle(GlassTokens.Color.statusAsk.color)
                    .annotation(position: .top, alignment: .trailing, spacing: GlassTokens.Space.s1) {
                        Text(figure(drill.long_context_threshold)).insightsTick(current: true)
                    }
                ForEach(drill.markers) { marker in
                    RuleMark(x: .value(text("analytics_turn"), Int(marker.turn_ordinal)))
                        .lineStyle(StrokeStyle(lineWidth: 1))
                        .foregroundStyle(readTurn == Int(marker.turn_ordinal) ? GlassColor.textPrimary : GlassColor.textTertiary)
                        .annotation(position: .top, spacing: 0) {
                            InsightsMarkerBadge(letter: marker.letter)
                        }
                }
            }
            .chartForegroundStyleScale(domain: series.map { text($0.key) }, range: series.map { $0.color.color })
            .chartLegend(.hidden)
            .chartXSelection(value: $readTurn)
            .chartXAxis(.hidden)
            .chartXScale(domain: Int(first) - 1...Int(max(last, first)) + 1)
            .chartYScale(domain: 0...max(top, 1))
            .chartYAxis {
                AxisMarks(position: .leading, values: .automatic(desiredCount: 4)) { value in
                    AxisGridLine().foregroundStyle(GlassTokens.Color.chartGrid.color)
                    AxisValueLabel {
                        if let tokens = value.as(Double.self) {
                            Text(figure(UInt64(max(0, tokens)))).insightsTick()
                        }
                    }
                }
            }
            .frame(height: 200)
            .insightsPlot()
            .accessibilityHidden(true)
            HStack {
                Text(InsightsSessionsWords.turnLabel(first, copy: copy)).insightsTick()
                Spacer()
                Text(InsightsSessionsWords.turnLabel(last, copy: copy)).insightsTick()
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .insightsCard()
    }

    /// A marker: its letter, the core's lines, and its turn. Pressing it
    /// reads its turn on the chart; pressing it again reads the session.
    private func markerCard(_ marker: InsightsDrillMarker, threshold: UInt64) -> some View {
        let lines = InsightsSessionsWords.markerLines(marker, threshold: threshold, copy: copy)
        let turn = Int(marker.turn_ordinal)
        return Button {
            readTurn = readTurn == turn ? nil : turn
        } label: {
            HStack(alignment: .top, spacing: GlassTokens.Space.s5) {
                InsightsMarkerBadge(letter: marker.letter)
                VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                    ForEach(Array(lines.enumerated()), id: \.offset) { index, line in
                        if index == 0 {
                            Text(line).insightsHeading()
                        } else {
                            Text(line).insightsNote()
                        }
                    }
                }
                .fixedSize(horizontal: false, vertical: true)
                Spacer(minLength: GlassTokens.Space.s4)
                Text(InsightsSessionsWords.turnLabel(marker.turn_ordinal, copy: copy)).insightsTick(current: readTurn == turn)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .insightsRowCard()
        }
        .buttonStyle(GlassPressStyle())
        .accessibilityElement(children: .combine)
    }
}

/// A marker's letter in a solid round badge, on the chart and on its card.
struct InsightsMarkerBadge: View {
    let letter: String

    static let side: CGFloat = 16

    var body: some View {
        Text(letter)
            .glassType(GlassTokens.TypeScale.micro)
            .foregroundStyle(GlassTokens.Color.paneOpaque.color)
            .frame(width: Self.side, height: Self.side)
            .background(Circle().fill(GlassColor.textPrimary))
    }
}
