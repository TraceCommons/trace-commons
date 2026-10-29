import Charts
import SwiftUI
import TCDesign
import TCShellCore

/// Overview's week at a glance (product owner, 2026-10-29): AI spend at API
/// prices, the top model and credits earned, then spend by model and
/// trends. Figures come from `insights_week`'s `work`; every one is the sum
/// as far as known (the coverage line says how many traces that is), and the
/// dash when nothing is known.
enum InsightsWorkWords {
    struct ModelRow: Equatable, Identifiable {
        let id: String
        let label: String
        let tool: String
        let amount: String
        /// Share of the week's known spend, 0...1.
        let share: Double
        let percent: String
    }

    struct Change: Equatable, Identifiable {
        let id: String
        let label: String
        let words: String
        let up: Bool
    }

    struct DayPair: Equatable, Identifiable {
        let id: String
        let label: String
        let week: Double
        let lastWeek: Double?
    }

    private static func text(_ key: String, _ copy: [String: String]) -> String { copy[key] ?? "" }

    /// Whether any of the spend is known: a trace priced in full, or a
    /// known part.
    static func knowsSpend(_ totals: DaemonData.InsightsWorkTotals?) -> Bool {
        guard let totals else { return false }
        return totals.pricedSessions + (totals.nearAiSessions ?? 0) > 0 || totals.usdMicrosKnown > 0
    }

    /// Spend at list prices: the sum as far as known, to the cent; a charge
    /// under a cent keeps `InsightsCreditWords.money`'s digit so it is never
    /// shown as zero.
    static func spend(_ totals: DaemonData.InsightsWorkTotals?, copy: [String: String]) -> String {
        guard knowsSpend(totals), let totals else {
            return text("analytics_unavailable", copy)
        }
        return dollars(totals.usdMicrosKnown)
    }

    static func dollars(_ micros: Int64) -> String {
        guard micros >= 10_000 else { return InsightsCreditWords.money(UInt64(max(0, micros))) }
        return (Decimal(micros) / 1_000_000).formatted(Decimal.FormatStyle.Currency(code: "USD")
            .precision(.fractionLength(2)))
    }

    static func count(_ value: Int64) -> String { value.formatted(.number) }

    /// "▲ 18% vs last week"; `nil` without last week's figure, with no
    /// change, or when either week's figure is only partly known: a minimum
    /// compared with an exact sum says nothing about the change.
    static func change(_ now: Int64, last: Int64?, partial: Bool = false,
                       copy: [String: String]) -> (words: String, up: Bool)? {
        guard !partial, let last, last > 0 else { return nil }
        let percent = Int((Double(abs(now - last)) / Double(last) * 100).rounded())
        guard percent > 0 else { return nil }
        let up = now > last
        return (InsightsOverviewWords.fill(text(up ? "analytics_change_up" : "analytics_change_down", copy),
                                           ["p": String(percent)]), up)
    }

    /// Whether this week's or last week's spend is only partly known.
    static func spendPartial(_ work: DaemonData.InsightsWork) -> Bool {
        work.week.usdPartial || work.week.pricedSessions == 0 || work.lastWeek?.usdPartial == true
    }

    static func toolName(_ tool: String, copy: [String: String]) -> String {
        text("analytics_credit_tool_" + tool, copy)
    }

    /// One row per model, largest known spend first.
    static func modelRows(_ work: DaemonData.InsightsWork, copy: [String: String]) -> [ModelRow] {
        let total = work.byModel.reduce(Int64(0)) { $0 + $1.usdMicrosKnown }
        return work.byModel.filter { $0.usdMicrosKnown > 0 }.map { model in
            let share = total > 0 ? Double(model.usdMicrosKnown) / Double(total) : 0
            return ModelRow(id: model.tool + ":" + model.key, label: model.key,
                            tool: toolName(model.tool, copy: copy),
                            amount: dollars(model.usdMicrosKnown),
                            share: share, percent: "\(Int((share * 100).rounded()))%")
        }
    }

    /// The changes that matter, largest first: spend and each tool's tokens.
    static func changes(_ work: DaemonData.InsightsWork, tokens: [DaemonData.InsightsWeekChange]?,
                        copy: [String: String]) -> [Change] {
        var changes: [Change] = []
        func add(_ id: String, _ label: String, _ now: Int64, _ last: Int64?, partial: Bool) {
            if let change = change(now, last: last, partial: partial, copy: copy) {
                changes.append(Change(id: id, label: label, words: change.words, up: change.up))
            }
        }
        add("spend", text("analytics_tile_spend", copy), work.week.usdMicrosKnown, work.lastWeek?.usdMicrosKnown,
            partial: spendPartial(work))
        for change in tokens ?? [] {
            guard let permille = change.permille, permille != 0 else { continue }
            let percent = Int((Double(abs(permille)) / 10).rounded())
            guard percent > 0 else { continue }
            let up = permille > 0
            changes.append(Change(
                id: "tokens:" + change.source,
                label: toolName(change.source, copy: copy) + " · " + text("analytics_card_tokens", copy),
                words: InsightsOverviewWords.fill(text(up ? "analytics_change_up" : "analytics_change_down", copy),
                                                  ["p": String(percent)]),
                up: up))
        }
        return changes
    }

    /// "Claude and Codex at API prices", or only the tool used this week,
    /// so a Claude user is not told of Codex prices.
    static func apiPrices(_ work: DaemonData.InsightsWork, copy: [String: String]) -> String {
        let tools = Set(work.byModel.map(\.tool)).union(activeTools(work).compactMap(\.key))
        if tools == ["claude_code"] { return text("analytics_tile_api_prices_claude", copy) }
        if tools == ["codex"] { return text("analytics_tile_api_prices_codex", copy) }
        return text("analytics_tile_api_prices", copy)
    }

    /// The tools with a trace this week, so a Claude user's card has no
    /// Codex row and the reverse.
    static func activeTools(_ work: DaemonData.InsightsWork) -> [DaemonData.InsightsWorkTotals] {
        work.byTool.filter { $0.sessions > 0 }
    }

    /// Trends has something to draw: a day with known spend, this week or
    /// the week before, or a change to list.
    static func showsTrends(_ work: DaemonData.InsightsWork, tokens: [DaemonData.InsightsWeekChange]?,
                            copy: [String: String]) -> Bool {
        let spent = work.byDay.contains { $0.usdMicrosKnown > 0 }
            || (work.lastWeekByDay ?? []).contains { $0.usdMicrosKnown > 0 }
        return spent || !changes(work, tokens: tokens, copy: copy).isEmpty
    }

    /// This week's days beside the week before's, in dollars.
    static func days(_ work: DaemonData.InsightsWork) -> [DayPair] {
        let parse = Date.ISO8601FormatStyle().year().month().day().dateSeparator(.dash)
        let utc = TimeZone(secondsFromGMT: 0) ?? .current
        return work.byDay.enumerated().map { index, day in
            let label = (try? parse.parse(day.date)).map {
                $0.formatted(Date.FormatStyle(timeZone: utc).weekday(.abbreviated))
            } ?? String(index + 1)
            let last = work.lastWeekByDay.flatMap { $0.indices.contains(index) ? $0[index] : nil }
            return DayPair(id: day.date, label: label, week: Double(day.usdMicrosKnown) / 1_000_000,
                           lastWeek: last.map { Double($0.usdMicrosKnown) / 1_000_000 })
        }
    }
}

/// The figures that lead Overview.
struct InsightsWeekTiles: View {
    let work: DaemonData.InsightsWork?
    /// Home's credit tile, the same figure and word (Kristi: Home and
    /// Traces must agree); `nil` hides the tile.
    let credit: (value: String, detail: String)?
    let copy: [String: String]

    private func text(_ key: String) -> String { copy[key] ?? "" }

    var body: some View {
        InsightsCardFlow(minimum: 170) {
            // A tile with nothing known is left out, not drawn as the dash.
            if let work {
                if InsightsWorkWords.knowsSpend(work.week) {
                    tile(text("analytics_tile_spend"), InsightsWorkWords.spend(work.week, copy: copy),
                         InsightsWorkWords.change(work.week.usdMicrosKnown, last: work.lastWeek?.usdMicrosKnown,
                                                  partial: InsightsWorkWords.spendPartial(work),
                                                  copy: copy)?.words ?? InsightsWorkWords.apiPrices(work, copy: copy))
                }
                if let top = InsightsWorkWords.modelRows(work, copy: copy).first {
                    tile(text("analytics_tile_top_model"), top.label,
                         InsightsOverviewWords.fill(text("analytics_tile_share_of_spend"), ["p": top.percent
                             .replacingOccurrences(of: "%", with: "")]) + " · " + top.tool)
                }
            }
            if let credit {
                tile(text("analytics_tile_credits"), credit.value, credit.detail)
            }
        }
    }

    /// Home's stat card (owner, 2026-10-29: the tiles' type matches the
    /// rest of the app): the eyebrow label, the 17pt bold value and the
    /// caption under it.
    private func tile(_ label: String, _ value: String, _ note: String?) -> some View {
        GlassStatCard(label, value: value, detail: note)
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            .accessibilityElement(children: .combine)
    }
}

/// Spend by model at API list prices, with a bar per model.
struct InsightsSpendByModelCard: View {
    let work: DaemonData.InsightsWork
    let copy: [String: String]
    var showBenchmarks: () -> Void = {}

    private func text(_ key: String) -> String { copy[key] ?? "" }

    var body: some View {
        let rows = InsightsWorkWords.modelRows(work, copy: copy)
        // The tool under each model only when there are two to tell apart.
        let tools = Set(rows.map(\.tool)).count > 1
        VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
            HStack(alignment: .firstTextBaseline) {
                Text(text("analytics_by_model_title")).insightsHeading()
                Spacer(minLength: GlassTokens.Space.s3)
                Text(InsightsWorkWords.apiPrices(work, copy: copy)).insightsCaption()
            }
            if rows.isEmpty {
                Text(text("analytics_unavailable")).insightsNote()
            }
            ForEach(rows) { row in
                HStack(alignment: .center, spacing: GlassTokens.Space.s3) {
                    VStack(alignment: .leading, spacing: 0) {
                        Text(row.label).glassType(GlassTokens.TypeScale.label.weight(.semibold)).lineLimit(1)
                        if tools { Text(row.tool).insightsCaption() }
                    }
                    .frame(minWidth: 110, alignment: .leading)
                    GeometryReader { proxy in
                        Capsule().fill(GlassTokens.Color.chartGrid.color)
                            .overlay(alignment: .leading) {
                                Capsule().fill(GlassTokens.Color.chartSeries1.color)
                                    .frame(width: max(4, proxy.size.width * row.share))
                            }
                    }
                    .frame(height: 8)
                    Text(row.amount).insightsMono().frame(minWidth: 64, alignment: .trailing)
                    Text(row.percent).insightsCaption().frame(minWidth: 36, alignment: .trailing)
                }
                .accessibilityElement(children: .combine)
            }
            Spacer(minLength: 0)
            Button(text("analytics_tab_benchmarks")) { showBenchmarks() }
                .buttonStyle(GlassButtonStyle(.link))
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .insightsSection()
    }
}

/// Spend per day this week against the week before, and what changed most.
struct InsightsTrendsCard: View {
    let work: DaemonData.InsightsWork
    let tokenChanges: [DaemonData.InsightsWeekChange]?
    let copy: [String: String]

    private func text(_ key: String) -> String { copy[key] ?? "" }

    var body: some View {
        let days = InsightsWorkWords.days(work)
        let changes = InsightsWorkWords.changes(work, tokens: tokenChanges, copy: copy)
        let thisWeek = text("analytics_this_week"), before = text("analytics_trends_week_before")
        VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
            Text(text("analytics_trends_title")).insightsHeading()
            if changes.isEmpty {
                chart(days, thisWeek: thisWeek, before: before)
            } else {
                ViewThatFits(in: .horizontal) {
                    HStack(alignment: .top, spacing: GlassTokens.Space.s6) {
                        chart(days, thisWeek: thisWeek, before: before).frame(minWidth: 320)
                        changeList(changes).frame(minWidth: 200, maxWidth: 280)
                    }
                    VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
                        chart(days, thisWeek: thisWeek, before: before)
                        changeList(changes)
                    }
                }
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .insightsSection()
    }

    @ViewBuilder
    private func chart(_ days: [InsightsWorkWords.DayPair], thisWeek: String, before: String) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
            Text(text("analytics_trends_spend_per_day")).insightsCaption()
            Chart {
                ForEach(days) { day in
                    if let last = day.lastWeek {
                        BarMark(x: .value("day", day.label), y: .value("usd", last))
                            .foregroundStyle(by: .value("week", before))
                            .position(by: .value("week", before))
                    }
                    BarMark(x: .value("day", day.label), y: .value("usd", day.week))
                        .foregroundStyle(by: .value("week", thisWeek))
                        .position(by: .value("week", thisWeek))
                }
            }
            .chartForegroundStyleScale([before: GlassTokens.Color.chartGrid.color,
                                        thisWeek: GlassTokens.Color.chartSeries1.color])
            .chartYAxis(.hidden)
            .frame(height: 120)
            .insightsPlot()
        }
    }

    @ViewBuilder
    private func changeList(_ changes: [InsightsWorkWords.Change]) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
            ForEach(changes) { change in
                InsightsUsageRow(label: change.label, value: change.words)
            }
        }
    }
}
