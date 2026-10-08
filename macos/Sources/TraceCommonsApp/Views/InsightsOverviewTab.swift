import Charts
import SwiftUI
import TCBridge
import TCDesign

/// Overview ("This week") over the saved snapshots (feed S). Every word is
/// the core's analytics copy, every figure the core's; an unknown figure is
/// the core's dash and never a zero. Under feed S nothing is compared with
/// another week, so the change and the best week read as the dash.
struct InsightsOverviewTab: View {
    let model: InsightsOverviewModel
    let copy: [String: String]
    @State private var breakdown: Breakdown = .model

    enum Breakdown: Hashable { case model, project, tool }

    private func text(_ key: String) -> String { copy[key] ?? "" }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s8) {
                if let overview = model.overview {
                    header(overview)
                    HStack(alignment: .top, spacing: GlassTokens.Space.s6) {
                        tokensCard(overview)
                        cacheCard(overview)
                        sessionsCard(overview)
                    }
                    if let inputs = model.inputs {
                        InsightsCardInputsView(inputs: inputs, copy: copy)
                    }
                    HStack(alignment: .top, spacing: GlassTokens.Space.s6) {
                        byDay(overview).frame(maxWidth: .infinity)
                        breakdownCard(overview).frame(width: 320)
                    }
                } else if model.failed {
                    Text(text("analytics_unavailable")).insightsError()
                }
                if model.busy { GlassSpinner() }
            }
            .padding(24)
            .frame(maxWidth: .infinity, alignment: .leading)
            .insightsSurface()
        }
    }

    private func header(_ overview: InsightsWeekOverview) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
            HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s6) {
                Text(text("analytics_this_week")).insightsTitle()
                GlassSelect(text("analytics_this_week"), selection: Binding(
                    get: { overview.week_start },
                    set: { model.selectWeek($0) }
                ), options: weekOptions(overview))
                .frame(maxWidth: 220)
            }
            Text(InsightsOverviewWords.coverageLine(overview.coverage, copy: copy)).insightsNote()
            if overview.undated_sessions > 0 {
                Text(InsightsOverviewWords.fill(text("analytics_coverage_undated"),
                                                ["d": String(overview.undated_sessions)])).insightsCaption()
            }
            if let overlap = overview.coverage.reasons["reimport_overlap"], overlap > 0 {
                Text(InsightsOverviewWords.fill(text("analytics_coverage_overlap"),
                                                ["r": String(overlap)])).insightsCaption()
            }
            ForEach(InsightsOverviewWords.feedLines(overview.feed, copy: copy), id: \.self) { line in
                Text(line).insightsCaption()
            }
        }
    }

    private func weekOptions(_ overview: InsightsWeekOverview) -> [GlassPickerOption<String>] {
        var weeks = overview.weeks
        if !weeks.contains(overview.week_start) { weeks.insert(overview.week_start, at: 0) }
        return weeks.map { start in
            let end = Calendar(identifier: .iso8601).date(
                byAdding: .day, value: 6,
                to: (try? Date(start, strategy: Date.ISO8601FormatStyle().year().month().day())) ?? Date())
                .map { $0.formatted(Date.ISO8601FormatStyle().year().month().day()) } ?? start
            return GlassPickerOption(InsightsOverviewWords.weekRange(start: start, end: end), value: start)
        }
    }

    /// A card that opens its drill-down.
    private func card<Content: View>(_ name: String, _ inputs: String,
                                     @ViewBuilder content: () -> Content) -> some View {
        // A second press on the open card closes its drill-down.
        Button {
            if model.inputs?.card == inputs { model.hideInputs() } else { model.showInputs(inputs) }
        } label: {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                Text(text(name)).insightsNote()
                content()
            }
            .frame(maxWidth: .infinity, minHeight: 120, alignment: .topLeading)
            .insightsRowCard()
        }
        .buttonStyle(GlassPressStyle())
        .accessibilityHint(text("analytics_drill_title"))
    }

    private func tokensCard(_ overview: InsightsWeekOverview) -> some View {
        card("analytics_card_tokens", "tokens") {
            if overview.sources.isEmpty {
                Text(text("analytics_unavailable")).glassType(GlassTokens.TypeScale.title).monospacedDigit()
            }
            // One line per harness, side by side; never summed.
            HStack(alignment: .top, spacing: GlassTokens.Space.s8) {
                ForEach(overview.sources) { source in
                    VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                        Text(InsightsOverviewWords.figure(source.tokens, copy: copy))
                            .glassType(GlassTokens.TypeScale.title).monospacedDigit()
                        Text(InsightsOverviewWords.sourceLine(source.source, copy: copy)).insightsCaption()
                        Text(InsightsOverviewWords.change(source, copy: copy)).insightsCaption()
                    }
                }
            }
        }
    }

    private func cacheCard(_ overview: InsightsWeekOverview) -> some View {
        card("analytics_card_cache_share", "cache_share") {
            if overview.sources.isEmpty {
                Text(text("analytics_unavailable")).glassType(GlassTokens.TypeScale.title)
            }
            ForEach(overview.sources) { source in
                VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                    // A bar, not a ring (owner decision D1, open).
                    InsightsShareBar(permille: source.cache_share?.permille)
                    Text(InsightsOverviewWords.fill(text("analytics_cache_share_line"), [
                        "source": InsightsOverviewWords.harness(source.source, copy: copy),
                        "p": InsightsOverviewWords.share(source.cache_share, copy: copy),
                    ])).insightsCaption()
                    Text(InsightsOverviewWords.bestWeek(source, copy: copy)).insightsCaption()
                }
            }
        }
    }

    private func sessionsCard(_ overview: InsightsWeekOverview) -> some View {
        card("analytics_card_sessions", "sessions") {
            Text(String(overview.sessions)).glassType(GlassTokens.TypeScale.title).monospacedDigit()
            ForEach(overview.sources) { source in
                if let largest = source.largest_session {
                    Text(InsightsOverviewWords.harness(source.source, copy: copy) + " \u{b7} "
                         + InsightsOverviewWords.fill(text("analytics_largest"), [
                            "t": InsightsOverviewWords.figure(largest.tokens, copy: copy),
                         ])).insightsCaption()
                }
            }
        }
    }

    private struct DayBar: Identifiable {
        let id: String
        let day: String
        let series: String
        let tokens: UInt64
    }

    private func dayBars(_ days: [InsightsDayTokens]) -> [DayBar] {
        let parse = Date.ISO8601FormatStyle().year().month().day()
        let style = Date.FormatStyle(timeZone: TimeZone(secondsFromGMT: 0) ?? .current).weekday(.abbreviated)
        return days.flatMap { day -> [DayBar] in
            let name = (try? Date(day.date, strategy: parse))?.formatted(style) ?? day.date
            return [
                ("analytics_series_uncached", day.uncached), ("analytics_series_cache_read", day.cache_read),
                ("analytics_series_cache_write", day.cache_write), ("analytics_series_output", day.output),
            ].map { key, tokens in
                DayBar(id: day.date + key, day: name, series: text(key), tokens: tokens)
            }
        }
    }

    private func byDay(_ overview: InsightsWeekOverview) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
            Text(text("analytics_card_tokens_by_day")).insightsHeading()
            if let days = overview.by_day {
                Chart(dayBars(days)) { bar in
                    BarMark(x: .value(text("analytics_card_tokens_by_day"), bar.day),
                            y: .value(text("analytics_card_tokens"), bar.tokens))
                    .foregroundStyle(by: .value(text("analytics_card_tokens"), bar.series))
                }
                .chartForegroundStyleScale(domain: [
                    text("analytics_series_uncached"), text("analytics_series_cache_read"),
                    text("analytics_series_cache_write"), text("analytics_series_output"),
                ], range: [GlassColor.textPrimary, GlassColor.accentText,
                           GlassColor.accentText.opacity(0.55), GlassColor.textTertiary])
                .frame(height: 220)
            } else {
                Text(text("analytics_unavailable")).insightsNote()
            }
            if overview.codex_interval_tokens != nil || overview.sources.contains(where: { $0.source == "codex" }) {
                HStack {
                    Text(text("analytics_codex_interval")).insightsCaption()
                    Spacer()
                    Text(InsightsOverviewWords.figure(overview.codex_interval_tokens, copy: copy))
                        .insightsMono()
                }
            }
        }
        .insightsCard()
    }

    private func breakdownCard(_ overview: InsightsWeekOverview) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
            GlassSegmentedTabs(text("analytics_card_by_model"), selection: $breakdown, segments: [
                GlassSegment(text("analytics_card_by_model"), value: Breakdown.model),
                GlassSegment(text("analytics_card_by_project"), value: Breakdown.project),
                GlassSegment(text("analytics_card_by_tool"), value: Breakdown.tool),
            ])
            switch breakdown {
            case .model:
                let rows = InsightsOverviewWords.modelRows(overview, copy: copy)
                let most = rows.map(\.tokens).max() ?? 0
                ForEach(rows) { row in
                    VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                        HStack {
                            Text(row.label)
                            Spacer()
                            Text(InsightsOverviewWords.figure(row.tokens, copy: copy)).insightsMono()
                        }
                        InsightsShareBar(fraction: most == 0 ? 0 : Double(row.tokens) / Double(most))
                    }
                }
                Text(text("analytics_model_labels_note")).insightsCaption()
            case .project:
                // Owner decision D7, open: analyzed files carry no project.
                Text(text("analytics_unavailable")).insightsHeading()
                Text(text("analytics_by_project_unavailable")).insightsCaption()
            case .tool:
                ForEach(overview.by_tool) { tool in
                    HStack {
                        Text(InsightsOverviewWords.harness(tool.source, copy: copy))
                        Spacer()
                        Text(InsightsOverviewWords.figure(tool.tokens, copy: copy)).insightsMono()
                    }
                }
            }
        }
        .insightsCard()
    }
}

/// A horizontal bar. An unknown share draws the empty track only, and its
/// figure beside it is the dash.
struct InsightsShareBar: View {
    let fraction: Double?

    init(permille: UInt64?) { fraction = permille.map { min(Double($0) / 1_000, 1) } }
    init(fraction: Double) { self.fraction = min(max(fraction, 0), 1) }

    var body: some View {
        GeometryReader { proxy in
            ZStack(alignment: .leading) {
                Capsule().fill(GlassTokens.Color.tintNeutral.color)
                if let fraction {
                    Capsule().fill(GlassColor.accentText).frame(width: proxy.size.width * fraction)
                }
            }
        }
        .frame(height: 6)
        .accessibilityHidden(true)
    }
}

/// "What makes up this number": each session's own figure, coverage state
/// and reasons, in the core's order.
struct InsightsCardInputsView: View {
    let inputs: InsightsCardInputs
    let copy: [String: String]

    private func text(_ key: String) -> String { copy[key] ?? "" }

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
            Text(text("analytics_drill_title")).insightsHeading()
            if inputs.card == "cache_share" {
                ForEach(inputs.sources) { source in
                    HStack {
                        Text(InsightsOverviewWords.harness(source.source, copy: copy))
                        Spacer()
                        Text(fraction(source.cache_share)).insightsMono()
                    }
                }
            }
            GlassTableHead {
                HStack {
                    Text(text("analytics_drill_session")).frame(maxWidth: .infinity, alignment: .leading)
                    Text(text("analytics_drill_tokens")).frame(width: 90, alignment: .trailing)
                    Text(text("analytics_drill_coverage")).frame(width: 90, alignment: .leading)
                    Text(text("analytics_drill_reason")).frame(maxWidth: .infinity, alignment: .leading)
                }
            }
            ForEach(inputs.sessions) { session in
                GlassTableRow {
                    HStack(alignment: .top) {
                        VStack(alignment: .leading) {
                            Text(InsightsOverviewWords.harness(session.source, copy: copy))
                            Text(session.session_ref).insightsMono().lineLimit(1).truncationMode(.middle)
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                        Text(inputs.card == "cache_share"
                             ? fraction(session.cache_share)
                             : InsightsOverviewWords.figure(session.tokens, copy: copy))
                            .insightsMono().frame(width: 90, alignment: .trailing)
                        Text(InsightsOverviewWords.state(session.state, copy: copy))
                            .frame(width: 90, alignment: .leading)
                        Text(session.reasons.map { InsightsOverviewWords.reason($0, copy: copy) }
                            .joined(separator: " \u{b7} "))
                            .insightsCaption().frame(maxWidth: .infinity, alignment: .leading)
                    }
                }
            }
        }
        .textSelection(.enabled)
        .insightsCard()
    }

    /// Numerator over denominator, both the core's counts.
    private func fraction(_ share: InsightsShareFigure?) -> String {
        guard let share else { return text("analytics_unavailable") }
        return InsightsOverviewWords.figure(share.numerator, copy: copy) + " / "
            + InsightsOverviewWords.figure(share.denominator, copy: copy)
    }
}
