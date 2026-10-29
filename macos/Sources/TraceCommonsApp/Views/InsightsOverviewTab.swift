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
    /// "Watched-folder counting is unavailable right now", shown with feed S
    /// when feed T is switched on but could not be read.
    var notice: String?
    /// The same week's patterns, for "Where tokens went"; `nil` hides it.
    var patterns: InsightsWeekPatterns?
    /// Current subscription limits are independent of the selected week.
    var subscriptions: SubscriptionUsageModel?
    /// NEAR AI spend from this Mac (product owner, 2026-10-28): the week's
    /// `near_ai_credit`; `nil` hides the NEAR AI usage card.
    var nearAiCredit: DaemonData.InsightsNearAiCredit?
    /// Claude and Codex tokens on each of the last 7 days, for the usage
    /// cards' bars.
    var usageByDay: [DaemonData.InsightsUsageDay]?
    /// A usage card's "By project": Details, on its By project view.
    var showProjects: () -> Void = {}
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

    /// By project is not drawn: analyzed files carry no project and
    /// watched-folder rows keep only a digest of one (owner decision D7,
    /// open), and a tab that cannot be used is hidden.
    enum Breakdown: Hashable { case model, tool }

    private func text(_ key: String) -> String { copy[key] ?? "" }
    private func figure(_ value: UInt64?) -> String { InsightsOverviewWords.figure(value, copy: copy) }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: InsightsLayout.sectionGap) {
                // AI usage at a glance leads (product owner, 2026-10-28):
                // one card per resource found on this Mac.
                InsightsUsageCards(subscriptions: subscriptions, nearAiCredit: nearAiCredit, usageByDay: usageByDay,
                                   copy: copy, showProjects: showProjects)
                if let overview = model.shown {
                    if let recap = comparisons.comparisons?.recap { recapCard(recap) }
                    // The Sessions breakdown and Tokens by day are off
                    // Overview (product owner, 2026-10-28): Tokens by day,
                    // and a picked day's sessions, are on Details.
                    header(overview)
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

    /// One harness: its tokens and cache share side by side, then, for
    /// Codex, the note that it records whole-trace totals. Claude's days are
    /// on Details' Tokens by day (product owner, 2026-10-28).
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
                if source.source == "codex" {
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

/// A Breakdown card's session list opens folded to its first few rows
/// (owner, 2026-10-26: the card had too many rows for Overview), with a
/// link under them to show every row and to fold them again.
struct InsightsBreakdownFold: View {
    /// Rows shown while folded.
    static let folded = 5

    let count: Int
    @Binding var showAll: Bool
    let copy: [String: String]

    static func shown<Row>(_ rows: [Row], all: Bool) -> [Row] {
        all ? rows : Array(rows.prefix(folded))
    }

    var body: some View {
        if count > Self.folded {
            Button(showAll
                   ? copy["analytics_drill_show_fewer"] ?? ""
                   : InsightsOverviewWords.fill(copy["analytics_drill_show_all"] ?? "", ["n": String(count)])) {
                showAll.toggle()
            }
            .buttonStyle(GlassButtonStyle(.link))
            .padding(.top, GlassTokens.Space.s2)
        }
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
    /// The day picked on "Tokens by day"; `nil` is the whole week.
    var day: InsightsDayPick?
    /// "Show week": the whole week again.
    var showWeek: () -> Void = {}
    /// Folded (`InsightsBreakdownFold`) unless the caller opens it with
    /// every row, as the responder-scale tests do.
    @State var showAll = false

    private func text(_ key: String) -> String { copy[key] ?? "" }
    private var routed: Bool { sessions.column == .shown }

    var body: some View {
        let dayRows = day.flatMap { day in
            InsightsDayBreakdown.rows(sessions.rows, day: day.index) { InsightsDayBreakdown.split($0.session) }
        }
        VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
            InsightsBreakdownTitle(day: dayRows == nil ? nil : day, copy: copy, showWeek: showWeek)
            VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
                // A week can contain thousands of sessions. Keep selectable
                // text hosts off the responder tree until their rows are visible.
                if let dayRows {
                    LazyVStack(spacing: 0) {
                        ForEach(InsightsBreakdownFold.shown(dayRows, all: showAll)) { row in
                            self.row(row.session, figure: row.tokens)
                        }
                    }
                    InsightsBreakdownFold(count: dayRows.count, showAll: $showAll, copy: copy)
                    InsightsDayNotes(empty: dayRows.isEmpty,
                                     codex: sessions.rows.contains { $0.session.source == "codex" }, copy: copy)
                } else {
                    LazyVStack(spacing: 0) {
                        ForEach(InsightsBreakdownFold.shown(sessions.rows, all: showAll)) { row in
                            self.row(row, figure: row.session.tokens.flatMap { UInt64(exactly: $0) })
                        }
                    }
                    InsightsBreakdownFold(count: sessions.rows.count, showAll: $showAll, copy: copy)
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
        // A picked day, or the week again, opens folded.
        .onChange(of: day) { _, _ in showAll = false }
    }

    /// One session; `figure`, on the right and in its label, is its week's
    /// tokens, or its tokens on the picked day.
    private func row(_ row: InsightsCounterSessions.Row, figure: UInt64?) -> some View {
        #if DEBUG
        let _ = OtherCollectionRowBodies.record("insightsCounterSessions", String(row.id))
        #endif
        return InsightsDrillRow(
            title: InsightsSessionsWords.counterTitle(row.session, copy: copy),
            figure: InsightsOverviewWords.figure(figure, copy: copy),
            label: InsightsSessionsWords.counterLabel(row.session, tokens: figure, copy: copy),
            state: InsightsOverviewWords.state(row.session.state, copy: copy),
            reasons: row.session.reasons.map { InsightsOverviewWords.reason($0, copy: copy) }
                .joined(separator: " \u{b7} ")
        ) {
            if routed { route(row.session.routing) }
        }
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
    /// The day picked on "Tokens by day"; `nil` is the whole week. Only the
    /// Sessions breakdown passes one.
    var day: InsightsDayPick?
    /// "Show week": the whole week again.
    var showWeek: () -> Void = {}
    /// Folded (`InsightsBreakdownFold`) unless the caller opens it with
    /// every row, as the responder-scale tests do.
    @State var showAll = false

    private func text(_ key: String) -> String { copy[key] ?? "" }

    var body: some View {
        let dayRows = day.flatMap { day in InsightsDayBreakdown.rows(inputs.sessions, day: day.index) { $0.by_day } }
        VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
            InsightsBreakdownTitle(day: dayRows == nil ? nil : day, copy: copy, showWeek: showWeek)
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
                if let dayRows {
                    LazyVStack(spacing: 0) {
                        ForEach(InsightsBreakdownFold.shown(dayRows, all: showAll)) { row in
                            self.row(row.session, figure: InsightsOverviewWords.figure(row.tokens, copy: copy),
                                     tokens: row.tokens)
                        }
                    }
                    InsightsBreakdownFold(count: dayRows.count, showAll: $showAll, copy: copy)
                    InsightsDayNotes(empty: dayRows.isEmpty,
                                     codex: inputs.sessions.contains { $0.source == "codex" }, copy: copy)
                } else {
                    LazyVStack(spacing: 0) {
                        ForEach(InsightsBreakdownFold.shown(inputs.sessions, all: showAll)) { session in
                            row(session, figure: inputs.card == "cache_share"
                                ? fraction(session.cache_share)
                                : InsightsOverviewWords.figure(session.tokens, copy: copy),
                                tokens: session.tokens)
                        }
                    }
                    InsightsBreakdownFold(count: inputs.sessions.count, showAll: $showAll, copy: copy)
                }
            }
            .insightsCardBody()
        }
        .textSelection(.enabled)
        .frame(maxWidth: .infinity, alignment: .leading)
        .insightsSection()
        // A picked day, or the week again, opens folded.
        .onChange(of: day) { _, _ in showAll = false }
    }

    /// One session; `tokens`, in its label, is its week's tokens or its
    /// tokens on the picked day.
    private func row(_ session: InsightsCardSession, figure: String, tokens: UInt64?) -> some View {
        #if DEBUG
        let _ = OtherCollectionRowBodies.record("insightsSavedSessions", session.session_ref)
        #endif
        return InsightsDrillRow(
            title: InsightsOverviewWords.harness(session.source, copy: copy),
            figure: figure,
            label: InsightsSessionsWords.rowLabel(session.session_ref, tokens: tokens,
                                                  snapshots: snapshots, copy: copy),
            state: InsightsOverviewWords.state(session.state, copy: copy),
            reasons: session.reasons.map { InsightsOverviewWords.reason($0, copy: copy) }
                .joined(separator: " \u{b7} "))
    }

    /// Numerator over denominator, both the core's counts.
    private func fraction(_ share: InsightsShareFigure?) -> String {
        guard let share else { return text("analytics_unavailable") }
        return InsightsOverviewWords.figure(share.numerator, copy: copy) + " / "
            + InsightsOverviewWords.figure(share.denominator, copy: copy)
    }
}

/// The Breakdown card's title: "Breakdown", or "Breakdown · Tuesday" with
/// "Show week" beside it while a day is picked.
struct InsightsBreakdownTitle: View {
    let day: InsightsDayPick?
    let copy: [String: String]
    let showWeek: () -> Void

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s4) {
            if let day {
                Text(InsightsOverviewWords.fill(copy["analytics_drill_title_day"] ?? "", ["day": day.weekday]))
                    .insightsTitle()
                Spacer(minLength: GlassTokens.Space.s4)
                Button(copy["analytics_drill_day_week"] ?? "") { showWeek() }
                    .buttonStyle(GlassButtonStyle(.link))
            } else {
                Text(copy["analytics_drill_title"] ?? "").insightsTitle()
            }
        }
    }
}

/// Under a picked day's sessions: that no session had tokens that day, and
/// that Codex sessions, counted over intervals, are in no day.
struct InsightsDayNotes: View {
    let empty: Bool
    let codex: Bool
    let copy: [String: String]

    var body: some View {
        if empty { Text(copy["analytics_drill_day_none"] ?? "").insightsCaption() }
        if codex { Text(copy["analytics_drill_day_codex"] ?? "").insightsCaption() }
    }
}
