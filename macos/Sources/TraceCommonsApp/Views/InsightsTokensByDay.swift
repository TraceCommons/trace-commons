import Charts
import SwiftUI
import TCBridge
import TCDesign

/// Tokens by day (#1695): on Breakdown beside the By project drill-down
/// since the product owner's 2026-10-28 review, which took it off Overview.
/// Claude's days, stacked; hovering a day reads it out, clicking it shows
/// that day's sessions in the window's inspector
/// (`InsightsBreakdownInspector`, owner 2026-10-29), clicking it again puts
/// the week back.
struct InsightsTokensByDay: View {
    let model: InsightsOverviewModel
    let copy: [String: String]
    /// The day clicked on the by-day chart, by its date; `nil` is the whole
    /// week. The inspector then shows that day's sessions.
    var pickedDay: String?
    /// Picks a day, or `nil` for the week again.
    var pick: (String?) -> Void = { _ in }
    /// The day the by-day chart is reading, by its label; `nil` reads the week.
    @State private var readDay: String?

    private func text(_ key: String) -> String { copy[key] ?? "" }
    private func figure(_ value: UInt64?) -> String { InsightsOverviewWords.figure(value, copy: copy) }

    var body: some View {
        if let overview = model.shown, let days = overview.by_day {
            byDay(days)
                .insightsCardBody()
                .frame(maxWidth: .infinity, alignment: .leading)
                .insightsSection()
                .onChange(of: overview.week_start) { _, _ in if pickedDay != nil { pick(nil) } }
        }
    }

    struct Day: Identifiable, Equatable {
        let id: String
        /// The weekday, on the axis.
        let name: String
        /// The weekday and date, read out.
        let label: String
        let values: [InsightsSeries: UInt64]
    }

    static func days(_ days: [InsightsDayTokens]) -> [Day] {
        let parse = Date.ISO8601FormatStyle().year().month().day()
        let utc = TimeZone(secondsFromGMT: 0) ?? .current
        let short = Date.FormatStyle(timeZone: utc).weekday(.abbreviated)
        let long = Date.FormatStyle(timeZone: utc).weekday(.abbreviated).month(.abbreviated).day()
        return days.map { day in
            let date = try? Date(day.date, strategy: parse)
            return Day(id: day.date, name: date?.formatted(short) ?? day.date, label: date?.formatted(long) ?? day.date,
                       values: [.cacheRead: day.cache_read, .uncached: day.uncached,
                                .cacheWrite: day.cache_write, .output: day.output])
        }
    }

    /// Claude Code's days, stacked on the chart surface. Hovering a day
    /// reads it out in the legend; leaving the chart reads the week, or the
    /// picked day. Clicking a day picks it, and clicking it again puts the
    /// week back.
    private func byDay(_ raw: [InsightsDayTokens]) -> some View {
        let days = Self.days(raw)
        let picked = days.first { $0.id == pickedDay }
        let reading = days.first { $0.name == readDay } ?? picked
        // The axis runs a little past the tallest day, not to the next round
        // number, so the bars use the plot's height.
        let tallest = days.map { $0.values.values.reduce(0, +) }.max() ?? 0
        let top = max(Double(tallest) * 1.1, 1)
        return VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
            HStack(alignment: .firstTextBaseline) {
                Text(text("analytics_card_tokens_by_day")).insightsHeading()
                Spacer(minLength: GlassTokens.Space.s4)
                if let reading { Text(reading.label).insightsCaption() }
            }
            LazyVGrid(columns: [GridItem(.adaptive(minimum: 150), spacing: GlassTokens.Space.s3)],
                      spacing: GlassTokens.Space.s3) {
                ForEach(InsightsSeries.allCases, id: \.self) { series in
                    let value = reading.map { $0.values[series] ?? 0 }
                        ?? days.reduce(UInt64(0)) { $0 + ($1.values[series] ?? 0) }
                    GlassLegendCell(text(series.key), value: figure(value), series: series.color)
                }
            }
            Chart {
                if let picked {
                    RectangleMark(x: .value(text("analytics_card_tokens_by_day"), picked.name))
                        .foregroundStyle(GlassColor.ink(0.14))
                }
                if let readDay, readDay != picked?.name {
                    RectangleMark(x: .value(text("analytics_card_tokens_by_day"), readDay))
                        .foregroundStyle(GlassColor.ink(0.08))
                }
                ForEach(days) { day in
                    ForEach(InsightsSeries.stack, id: \.self) { series in
                        BarMark(x: .value(text("analytics_card_tokens_by_day"), day.name),
                                y: .value(text("analytics_card_tokens"), day.values[series] ?? 0),
                                width: .ratio(0.56))
                        .foregroundStyle(by: .value(text("analytics_card_tokens"), text(series.key)))
                    }
                }
            }
            .chartForegroundStyleScale(domain: InsightsSeries.allCases.map { text($0.key) },
                                       range: InsightsSeries.allCases.map { $0.color.color })
            .chartLegend(.hidden)
            .chartXSelection(value: $readDay)
            // A click lands on the day the pointer reads.
            .simultaneousGesture(TapGesture().onEnded {
                guard let day = days.first(where: { $0.name == readDay }) else { return }
                pick(pickedDay == day.id ? nil : day.id)
            })
            // The days in their own order, always. Without it the hover
            // wash, drawn first, made the hovered day the first category,
            // and the bars reordered under the pointer.
            .chartXScale(domain: days.map(\.name))
            .chartYScale(domain: 0...top)
            .chartYAxis {
                AxisMarks(position: .leading, values: .automatic(desiredCount: 3)) { value in
                    AxisGridLine().foregroundStyle(GlassTokens.Color.chartGrid.color)
                    AxisValueLabel {
                        if let tokens = value.as(Double.self) {
                            Text(figure(UInt64(max(0, tokens)))).insightsTick()
                        }
                    }
                }
            }
            .chartXAxis {
                AxisMarks { value in
                    AxisValueLabel {
                        if let name = value.as(String.self) {
                            Text(name).insightsTick(current: name == readDay || name == picked?.name)
                        }
                    }
                }
            }
            .frame(height: 120)
            .insightsPlot()
            // The keyboard and VoiceOver pick a day too: the arrow keys and
            // the adjustable action step through the days, Escape and the
            // last step past either end put the week back.
            .focusable()
            .onMoveCommand { direction in
                switch direction {
                case .left: pick(Self.step(days.map(\.id), from: pickedDay, by: -1))
                case .right: pick(Self.step(days.map(\.id), from: pickedDay, by: 1))
                default: break
                }
            }
            .onExitCommand { if pickedDay != nil { pick(nil) } }
            .accessibilityElement()
            .accessibilityLabel(text("analytics_card_tokens_by_day"))
            .accessibilityValue(picked?.label ?? "")
            .accessibilityAdjustableAction { direction in
                switch direction {
                case .decrement: pick(Self.step(days.map(\.id), from: pickedDay, by: -1))
                case .increment: pick(Self.step(days.map(\.id), from: pickedDay, by: 1))
                @unknown default: break
                }
            }
            .accessibilityActions {
                ForEach(days) { day in
                    Button(day.label) { pick(pickedDay == day.id ? nil : day.id) }
                }
            }
        }
    }

    /// The day `by` steps from `current` among `ids`: from the week, the
    /// first or the last day; past either end, the week again.
    nonisolated static func step(_ ids: [String], from current: String?, by: Int) -> String? {
        guard let current, let index = ids.firstIndex(of: current) else { return by > 0 ? ids.first : ids.last }
        let next = index + by
        return ids.indices.contains(next) ? ids[next] : nil
    }

}
