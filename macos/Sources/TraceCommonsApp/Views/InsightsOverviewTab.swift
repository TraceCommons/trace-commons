import Charts
import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// Overview ("This week") over one feed at a time: the daemon's counter
/// pass (feed T) when it sent a week, the saved snapshots (feed S)
/// otherwise. Every word is the core's analytics copy, every figure the
/// core's; an unknown figure is the core's dash and never a zero. Only feed
/// T compares weeks, carries the lever of the week and the weekly summary
/// card; under feed S the change and the best week are left out.
///
/// Laid out for the main pane (direction B, 2026-10-15): the week and the
/// Sessions figure in one row, then one card per harness, since Claude Code
/// and Codex are never summed. Claude Code's card holds the days; Codex
/// records whole-trace totals only.
struct InsightsOverviewTab: View {
    let model: InsightsOverviewModel
    let comparisons: InsightsComparisonsModel
    let copy: [String: String]
    /// Saved snapshots, so a drill-down row reads as its session's label.
    /// No default: a caller that forgets them would show every row as the dash.
    let snapshots: [LocalInsight]
    /// The Subscriptions card, drawn only while its switch is on.
    var subscriptions: SubscriptionUsageModel?
    /// "Watched-folder counting is unavailable right now", shown with feed S
    /// when feed T is switched on but could not be read.
    var notice: String?
    /// The same week's patterns, for "Where tokens went"; `nil` hides it.
    var patterns: InsightsWeekPatterns?
    /// Put a week on screen; the window routes it to the feed showing.
    var selectWeek: (String) -> Void = { _ in }
    /// "Open recap": the window puts the closed week on screen.
    var openRecap: () -> Void = {}
    /// "Hide": the daemon's `insights_recap_card_enabled`.
    var turnOffRecap: (() -> Void)?
    /// "Show the reads": the Patterns tab.
    var showReads: () -> Void = {}
    /// "Patterns": the Patterns tab.
    var showPatterns: () -> Void = {}
    @State private var breakdown: Breakdown = .model
    /// The day the by-day chart is reading, by its label; `nil` reads the week.
    @State private var readDay: String?

    /// By project is not drawn: analyzed files carry no project and
    /// watched-folder rows keep only a digest of one (owner decision D7,
    /// open), and a tab that cannot be used is hidden.
    enum Breakdown: Hashable { case model, tool }

    private func text(_ key: String) -> String { copy[key] ?? "" }
    private func figure(_ value: UInt64?) -> String { InsightsOverviewWords.figure(value, copy: copy) }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: InsightsLayout.sectionGap) {
                if let overview = model.shown {
                    if let recap = comparisons.comparisons?.recap { recapCard(recap) }
                    header(overview)
                    sessionsBreakdown
                    LazyVGrid(columns: InsightsLayout.columns, alignment: .leading, spacing: GlassTokens.Space.cardGap) {
                        ForEach(overview.sources) { source in
                            harnessCard(source, overview: overview)
                        }
                    }
                    if overview.sources.isEmpty {
                        Text(text("analytics_unavailable")).insightsFigure()
                    }
                    if let inputs = model.inputs {
                        InsightsCardInputsView(inputs: inputs, snapshots: snapshots, copy: copy)
                    }
                    if overview.feed == "counter_pass", let lever = comparisons.comparisons?.lever {
                        leverCard(lever)
                    }
                    LazyVGrid(columns: InsightsLayout.columns, alignment: .leading, spacing: GlassTokens.Space.cardGap) {
                        if let patterns, patterns.week_start == overview.week_start, !patterns.cards.isEmpty {
                            whereTokensWent(patterns)
                        }
                        breakdownCard(overview)
                    }
                } else if model.failed {
                    Text(text("analytics_unavailable")).insightsError()
                }
                // Not a figure of the week: drawn whatever week, or none,
                // is showing.
                if let subscriptions, subscriptions.shown {
                    SubscriptionUsageCard(model: subscriptions, copy: copy)
                }
                if model.busy { GlassSpinner() }
            }
            .padding(.vertical, GlassTokens.Space.s7)
            .frame(maxWidth: .infinity, alignment: .leading)
            .insightsSurface()
        }
    }

    /// The week picker on its own row, on the right as on Patterns (owner,
    /// 2026-10-19), then one section under it (owner, 2026-10-20): the
    /// Sessions figure over the coverage line, with whatever qualifies the
    /// counts behind its "i", on a card as the harnesses' are.
    private func header(_ overview: InsightsWeekOverview) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
            HStack(spacing: 0) {
                Spacer(minLength: 0)
                InsightsWeekSelect(weekStart: overview.week_start, weeks: overview.weeks, copy: copy,
                                   select: selectWeek)
            }
            VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
                VStack(alignment: .leading, spacing: 0) {
                    Text(text("analytics_card_sessions")).insightsEyebrow()
                    Text(String(overview.sessions)).insightsFigure()
                }
                InsightsCoverageLine(coverage: overview.coverage, copy: copy, info: headerInfo(overview))
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .insightsSection()
        }
    }

    /// The Sessions figure's breakdown, always shown right under it (owner,
    /// 2026-10-22: it opened from a chevron on the card, and drew below the
    /// harnesses): the daemon's rows under feed T, the core's under feed S.
    /// A week with no rows draws none, never an empty card.
    @ViewBuilder
    private var sessionsBreakdown: some View {
        if let sessions = model.counterSessions {
            if !sessions.rows.isEmpty {
                InsightsCounterSessionsView(sessions: sessions, copy: copy)
            }
        } else if let inputs = model.sessionsInputs, !inputs.sessions.isEmpty {
            InsightsCardInputsView(inputs: inputs, snapshots: snapshots, copy: copy)
        }
    }

    /// What qualifies the week's counts, behind the coverage line's "i":
    /// undated and overlapping traces, which feed this is, and why
    /// watched-folder counting is not showing.
    private func headerInfo(_ overview: InsightsWeekOverview) -> [String] {
        var lines: [String] = []
        if overview.undated_sessions > 0 {
            lines.append(InsightsOverviewWords.fill(text("analytics_coverage_undated"),
                                                    ["d": String(overview.undated_sessions)]))
        }
        if let overlap = overview.coverage.reasons["reimport_overlap"], overlap > 0 {
            lines.append(InsightsOverviewWords.fill(text("analytics_coverage_overlap"), ["r": String(overlap)]))
        }
        lines += InsightsOverviewWords.feedLines(overview.feed, copy: copy)
        if overview.feed == "saved", let notice { lines.append(notice) }
        return lines
    }

    /// The weekly summary card, on the first opens after a week closes.
    private func recapCard(_ recap: InsightsRecap) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
            Text(InsightsComparisonsWords.recapTitle(recap, copy: copy)).insightsTitle()
            VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                HStack(alignment: .top, spacing: GlassTokens.Space.s8) {
                    ForEach(recap.sources) { source in
                        VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                            Text(InsightsOverviewWords.figure(source.tokens, copy: copy))
                                .glassType(GlassTokens.TypeScale.title).monospacedDigit()
                            Text(InsightsOverviewWords.sourceLine(source.source, copy: copy)).insightsCaption()
                            if let change = InsightsComparisonsWords.recapChange(source, sessions: recap.sessions, copy: copy) {
                                Text(change).insightsCaption()
                            }
                        }
                    }
                }
                ForEach(Array(recap.items.enumerated()), id: \.offset) { _, item in
                    if let line = InsightsComparisonsWords.recapItem(item, copy: copy) {
                        Text(line).insightsNote()
                    }
                }
                if let line = InsightsComparisonsWords.recapThreshold(recap, copy: copy) {
                    Text(line).insightsNote()
                }
            }
            .insightsCardBody()
            HStack(spacing: GlassTokens.Space.s4) {
                Button(text("analytics_recap_open")) { openRecap() }
                    .buttonStyle(GlassButtonStyle(.primary, small: true))
                // The decline is a link, after the action (Ron's #1287 review).
                if let turnOffRecap {
                    Button(text("analytics_recap_turn_off")) { turnOffRecap() }
                        .buttonStyle(GlassButtonStyle(.link))
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .insightsSection()
    }

    /// The lever of the week: an observation only (owner decision D2, open).
    @ViewBuilder
    private func leverCard(_ lever: InsightsLeverState) -> some View {
        if let pick = lever.pick {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                HStack(spacing: GlassTokens.Space.s5) {
                    Image(systemName: "chart.line.uptrend.xyaxis")
                        .glassGlyph(12, weight: .semibold)
                        .foregroundStyle(GlassColor.accentText)
                        .frame(width: GlassTokens.Size.toolTile, height: GlassTokens.Size.toolTile)
                        .background(RoundedRectangle(cornerRadius: GlassTokens.Radius.tile, style: .continuous)
                            .fill(GlassTokens.Color.tintAccent.color))
                        .accessibilityHidden(true)
                    Text(text("analytics_lever_title")).insightsTitle()
                }
                VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                    ForEach(InsightsComparisonsWords.leverLines(pick, copy: copy), id: \.self) { line in
                        Text(line).insightsNote()
                    }
                }
                .insightsCardBody()
                HStack(spacing: GlassTokens.Space.s4) {
                    if pick.kind == "repeated_reads" {
                        Button(text("analytics_lever_show_reads")) { showReads() }
                            .buttonStyle(GlassButtonStyle(.glass, small: true))
                    }
                    // The decline is a link, after the action (Ron's #1287 review).
                    Button(text("analytics_lever_not_useful")) { comparisons.notUseful() }
                        .buttonStyle(GlassButtonStyle(.link))
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .insightsSection()
        } else if !lever.disabled.isEmpty {
            Text(text("analytics_lever_off")).insightsCaption()
        }
    }

    /// A figure that opens its drill-down. A second press on the open one
    /// closes it. Feed T rows carry no session reference the core could
    /// look up, so under feed T only the Sessions figure drills, from the
    /// daemon's rows; a figure that does not drill is drawn plain, never
    /// dimmed as a disabled control.
    @ViewBuilder
    private func drillButton<Label: View>(_ inputs: String, @ViewBuilder label: () -> Label) -> some View {
        if model.drills(inputs) {
            Button {
                if model.openCard == inputs { model.hideInputs() } else { model.showInputs(inputs) }
            } label: {
                label().contentShape(Rectangle())
            }
            .buttonStyle(GlassPressStyle())
            .accessibilityHint(text("analytics_drill_title"))
        } else {
            label()
        }
    }

    /// One harness: its tokens and cache share side by side, then its days
    /// (Claude Code) or the note that it records whole-trace totals (Codex).
    private func harnessCard(_ source: InsightsWeekSource, overview: InsightsWeekOverview) -> some View {
        VStack(alignment: .leading, spacing: InsightsLayout.groupGap) {
            // The core's line for the harness says how it is counted:
            // "Claude Code · counted per turn".
            HStack(spacing: GlassTokens.Space.s5) {
                InsightsHarnessTile(source: source.source)
                Text(InsightsOverviewWords.sourceLine(source.source, copy: copy)).insightsTitle()
                    .fixedSize(horizontal: false, vertical: true)
            }
            VStack(alignment: .leading, spacing: InsightsLayout.groupGap) {
                HStack(alignment: .top, spacing: GlassTokens.Space.s8) {
                    drillButton("tokens") {
                        VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
                            Text(text("analytics_card_tokens")).insightsEyebrow()
                            Text(figure(source.tokens)).insightsFigure()
                            if source.change_permille != nil {
                                Text(InsightsOverviewWords.change(source, copy: copy)).insightsCaption()
                            }
                            if let largest = source.largest_session {
                                Text(InsightsOverviewWords.fill(text("analytics_largest"), ["t": figure(largest.tokens)]))
                                    .insightsCaption()
                            }
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    drillButton("cache_share") {
                        VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
                            Text(text("analytics_card_cache_share")).insightsEyebrow()
                            Text(source.cache_share.map { InsightsOverviewWords.share($0, copy: copy) + "%" }
                                 ?? text("analytics_unavailable")).insightsFigure()
                            // A bar, not a ring (owner decision D1, open).
                            InsightsShareBar(permille: source.cache_share?.permille,
                                             tick: InsightsOverviewWords.bestTick(source))
                                .padding(.vertical, GlassTokens.Space.s2)
                            if source.best != nil {
                                Text(InsightsOverviewWords.bestWeek(source, copy: copy)).insightsCaption()
                                    .fixedSize(horizontal: false, vertical: true)
                            }
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                    }
                }
                if source.source == "claude_code", let days = overview.by_day {
                    byDay(days)
                } else if source.source == "codex" {
                    codexInterval(overview)
                }
            }
            .insightsCardBody()
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .insightsSection()
    }

    /// Codex counts whole traces, not days: a dashed box, never zero bars.
    private func codexInterval(_ overview: InsightsWeekOverview) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s4) {
            Text(text("analytics_codex_interval")).insightsCaption()
            Spacer(minLength: GlassTokens.Space.s4)
            Text(figure(overview.codex_interval_tokens)).insightsMono()
        }
        .padding(.horizontal, GlassTokens.Space.s5)
        .padding(.vertical, GlassTokens.Space.s5)
        .background(RoundedRectangle(cornerRadius: GlassTokens.Radius.control, style: .continuous)
            .strokeBorder(GlassTokens.Color.chartGrid.color, style: StrokeStyle(lineWidth: 1, dash: [4, 3])))
        // A well in the card's body: in line with the title, not the text.
        .modifier(InsightsInlineContainer())
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
    /// reads it out in the legend; leaving the chart reads the week.
    private func byDay(_ raw: [InsightsDayTokens]) -> some View {
        let days = Self.days(raw)
        let reading = days.first { $0.name == readDay }
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
                if let readDay {
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
                            Text(name).insightsTick(current: name == readDay)
                        }
                    }
                }
            }
            .frame(height: 120)
            .insightsPlot()
            .accessibilityHidden(true)
        }
    }

    /// The same week's patterns, by name and figure, in the core's order.
    private func whereTokensWent(_ patterns: InsightsWeekPatterns) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
            InsightsSectionTitle(title: text("analytics_where_tokens_went"),
                                 info: [text("analytics_patterns_overlap")]) {
                Button(text("analytics_tab_patterns")) { showPatterns() }
                    .buttonStyle(GlassButtonStyle(.link))
            }
            VStack(alignment: .leading, spacing: 0) {
                VStack(spacing: 0) {
                    ForEach(Array(patterns.cards.enumerated()), id: \.element.id) { index, card in
                        GlassTableRow(first: index == 0) {
                            HStack(alignment: .firstTextBaseline) {
                                Text(InsightsPatternsWords.title(card.kind, copy: copy))
                                Spacer(minLength: GlassTokens.Space.s4)
                                // The Patterns card's own headline: a count
                                // when the estimate rounds to 0.
                                Text(InsightsPatternsWords.headline(card, copy: copy)).insightsMono()
                            }
                        }
                    }
                }
            }
            .insightsCardBody()
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .insightsSection()
    }

    private func breakdownCard(_ overview: InsightsWeekOverview) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
            HStack(spacing: GlassTokens.Space.s4) {
                GlassSegmentedTabs(text("analytics_card_by_model"), selection: $breakdown, segments: [
                    GlassSegment(text("analytics_card_by_model"), value: Breakdown.model),
                    GlassSegment(text("analytics_card_by_tool"), value: Breakdown.tool),
                ])
                InsightsInfo(text("analytics_card_by_model"), [text("analytics_model_labels_note")])
            }
            LazyVStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
                switch breakdown {
                case .model:
                    let rows = InsightsOverviewWords.modelRows(overview, copy: copy)
                    let most = rows.map(\.tokens).max() ?? 0
                    ForEach(rows) { row in
                        #if DEBUG
                        let _ = OtherCollectionRowBodies.record("supplementalModels", String(row.id))
                        #endif
                        VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                            HStack {
                                Text(row.label).lineLimit(1).truncationMode(.middle)
                                Spacer()
                                Text(InsightsOverviewWords.figure(row.tokens, copy: copy)).insightsMono()
                            }
                            InsightsShareBar(fraction: most == 0 ? 0 : Double(row.tokens) / Double(most))
                        }
                    }
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
            .insightsCardBody()
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .insightsSection()
    }
}

/// A horizontal bar in the single data series' colour on a recessed
/// track. An unknown share draws the empty track only, and its figure
/// beside it is the dash.
struct InsightsShareBar: View {
    let fraction: Double?
    /// "Your best week": a tick at the previous best share (feed T only).
    var tick: Double?

    init(permille: UInt64?, tick: UInt64? = nil) {
        fraction = permille.map { min(Double($0) / 1_000, 1) }
        self.tick = tick.map { min(Double($0) / 1_000, 1) }
    }
    init(fraction: Double) { self.fraction = min(max(fraction, 0), 1) }

    var body: some View {
        GeometryReader { proxy in
            ZStack(alignment: .leading) {
                Capsule().fill(GlassTokens.Color.wellFill.color)
                if let fraction {
                    Capsule().fill(GlassTokens.Color.purpleSoft.color).frame(width: proxy.size.width * fraction)
                }
                if let tick {
                    RoundedRectangle(cornerRadius: 1).fill(GlassColor.textPrimary)
                        .frame(width: 2, height: 12)
                        .offset(x: max(proxy.size.width * tick - 1, 0))
                }
            }
        }
        .frame(height: 6)
        .accessibilityHidden(true)
    }
}

/// Feed T's "What makes up this number" for the Sessions figure: each
/// counted session by its harness, first event and transcript tokens, its
/// coverage, and, while the daemon sends a route, where its calls went in
/// proxy tokens. The two counts sit side by side and are never added
/// together or reconciled; an unknown figure is the dash.
struct InsightsCounterSessionsView: View {
    let sessions: InsightsCounterSessions
    let copy: [String: String]

    private func text(_ key: String) -> String { copy[key] ?? "" }
    private var routed: Bool { sessions.column == .shown }

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
            Text(text("analytics_drill_title")).insightsTitle()
            VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
                // A week can contain thousands of sessions. Keep selectable
                // text hosts off the responder tree until their rows are visible.
                LazyVStack(spacing: 0) {
                    ForEach(sessions.rows) { row in
                        #if DEBUG
                        let _ = OtherCollectionRowBodies.record("insightsCounterSessions", String(row.id))
                        #endif
                        InsightsDrillRow(
                            title: InsightsSessionsWords.counterTitle(row.session, copy: copy),
                            figure: InsightsOverviewWords.figure(row.session.tokens.flatMap { UInt64(exactly: $0) },
                                                                copy: copy),
                            label: InsightsSessionsWords.counterLabel(row.session, copy: copy),
                            state: InsightsOverviewWords.state(row.session.state, copy: copy),
                            reasons: row.session.reasons.map { InsightsOverviewWords.reason($0, copy: copy) }
                                .joined(separator: " \u{b7} ")
                        ) {
                            if routed { route(row.session.routing) }
                        }
                    }
                }
                if routed {
                    Text(text("analytics_route_measure_note")).insightsCaption()
                }
                if let line = InsightsRouteWords.unavailableLine(sessions.column, copy: copy) {
                    Text(line).insightsCaption()
                }
            }
            .insightsCardBody()
        }
        .textSelection(.enabled)
        .frame(maxWidth: .infinity, alignment: .leading)
        .insightsSection()
    }

    /// "Private AI", the category's word, then its split in proxy tokens
    /// and qualifiers.
    private func route(_ routing: DaemonData.InsightsWeekRouting?) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
            HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s3) {
                Text(text("analytics_drill_private")).insightsEyebrow()
                Text(InsightsRouteWords.category(routing, copy: copy)).insightsNote()
            }
            if let split = InsightsRouteWords.split(routing, copy: copy) {
                Text(split).insightsCaption()
            }
            ForEach(InsightsRouteWords.reasons(routing, copy: copy), id: \.self) { reason in
                Text(reason).insightsCaption()
            }
        }
        .padding(.top, GlassTokens.Space.s2)
    }
}

/// "What makes up this number": each session's own figure, coverage state
/// and reasons, in the core's order. A row names its session by the saved
/// snapshot's label, or the dash when none matches; never the snapshot ID.
struct InsightsCardInputsView: View {
    let inputs: InsightsCardInputs
    let snapshots: [LocalInsight]
    let copy: [String: String]

    private func text(_ key: String) -> String { copy[key] ?? "" }

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
            Text(text("analytics_drill_title")).insightsTitle()
            VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
                if inputs.card == "cache_share" {
                    ForEach(inputs.sources) { source in
                        HStack {
                            Text(InsightsOverviewWords.harness(source.source, copy: copy))
                            Spacer()
                            Text(fraction(source.cache_share)).insightsMono()
                        }
                    }
                }
                // Saved-session breakdowns share the same viewport-bound
                // rendering as watched sessions, while retaining text selection.
                LazyVStack(spacing: 0) {
                    ForEach(inputs.sessions) { session in
                        #if DEBUG
                        let _ = OtherCollectionRowBodies.record("insightsSavedSessions", session.session_ref)
                        #endif
                        InsightsDrillRow(
                            title: InsightsOverviewWords.harness(session.source, copy: copy),
                            figure: inputs.card == "cache_share"
                                ? fraction(session.cache_share)
                                : InsightsOverviewWords.figure(session.tokens, copy: copy),
                            label: InsightsSessionsWords.rowLabel(session.session_ref, tokens: session.tokens,
                                                                  snapshots: snapshots, copy: copy),
                            state: InsightsOverviewWords.state(session.state, copy: copy),
                            reasons: session.reasons.map { InsightsOverviewWords.reason($0, copy: copy) }
                                .joined(separator: " \u{b7} "))
                    }
                }
            }
            .insightsCardBody()
        }
        .textSelection(.enabled)
        .frame(maxWidth: .infinity, alignment: .leading)
        .insightsSection()
    }

    /// Numerator over denominator, both the core's counts.
    private func fraction(_ share: InsightsShareFigure?) -> String {
        guard let share else { return text("analytics_unavailable") }
        return InsightsOverviewWords.figure(share.numerator, copy: copy) + " / "
            + InsightsOverviewWords.figure(share.denominator, copy: copy)
    }
}
