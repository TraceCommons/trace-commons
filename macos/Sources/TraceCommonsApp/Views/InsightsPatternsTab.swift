import Charts
import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// Patterns ("Where tokens went") over one feed at a time: the daemon's
/// counter pass (feed T) when it sent a week, the saved snapshots (feed S)
/// otherwise. Every word is the core's analytics copy, every figure the
/// core's. A week with no figure is a gap marked with the dash, never a
/// zero bar; files are letters with an extension, never a path. Only feed T
/// compares weeks and carries "Your goals"; under feed S the change is left
/// out.
///
/// Laid out for the main pane (direction B, 2026-10-15): one card per
/// pattern in a single column, each with six labelled weeks on the chart
/// surface and, when a goal covers it, the goal as a dashed line.
struct InsightsPatternsTab: View {
    let model: InsightsPatternsModel
    let comparisons: InsightsComparisonsModel
    let copy: [String: String]
    /// Put a week on screen; the window routes it to the feed showing.
    var selectWeek: (String) -> Void = { _ in }
    /// The daemon's week, for the drill-down that leads Details (product
    /// owner, 2026-10-28); `nil` (no counter pass) hides it.
    var week: DaemonData.InsightsWeek?
    /// The drill-down's view; Overview's "By project" sets it.
    var detailsView: Binding<InsightsCreditWords.Dimension> = .constant(.folder)
    /// The week's Overview model, for Tokens by day beside the drill-down
    /// (product owner, 2026-10-28); `nil` hides it.
    var overview: InsightsOverviewModel?
    /// Saved snapshots, so a picked day's rows read as their labels.
    var snapshots: [LocalInsight] = []
    @State private var goalKind = InsightsComparisonsWords.goalKinds[0]
    @State private var goalSource = "claude_code"
    @State private var goalNumber = ""

    private func text(_ key: String) -> String { copy[key] ?? "" }
    private func figure(_ value: UInt64?) -> String { InsightsOverviewWords.figure(value, copy: copy) }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: InsightsLayout.sectionGap) {
                if let patterns = model.shown {
                    header(patterns)
                    // Details leads with the drill-down, by project, task or
                    // tool (product owner, 2026-10-28). The pattern cards
                    // stay under it, and the goals live only here.
                    if let week { InsightsDetailsTable(week: week, copy: copy, dimension: detailsView) }
                    // Tokens by day moved here from Overview, a picked day's
                    // sessions with it (product owner, 2026-10-28).
                    if let overview { InsightsTokensByDay(model: overview, copy: copy, snapshots: snapshots) }
                    LazyVGrid(columns: InsightsLayout.columns, alignment: .leading, spacing: GlassTokens.Space.cardGap) {
                        ForEach(InsightsPatternsWords.shown(patterns.cards)) { card in
                            VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
                                patternCard(card)
                                if let sessions = model.sessions, sessions.pattern == card.kind {
                                    InsightsPatternSessionsView(found: sessions, copy: copy)
                                }
                            }
                        }
                    }
                    if patterns.feed == "counter_pass", let found = comparisons.comparisons {
                        goals(found)
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

    /// The week picker on its own row, on the right (owner, 2026-10-19),
    /// then one section under it (owner, 2026-10-20), as on Overview: the
    /// week's scope over the coverage line.
    private func header(_ patterns: InsightsWeekPatterns) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
            HStack(spacing: 0) {
                Spacer(minLength: 0)
                InsightsWeekSelect(weekStart: patterns.week_start, weeks: patterns.weeks, copy: copy,
                                   select: selectWeek)
                    .fixedSize()
            }
            VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
                // Only Claude Code records tool calls: the figures cover its
                // traces when another harness is in the week.
                if let line = InsightsPatternsWords.claudeOnlyLine(patterns, copy: copy) {
                    InsightsScopeLine(source: "claude_code", line: line)
                }
                // How the patterns are found, that they overlap, and which
                // feed this is: behind the coverage line's "i".
                InsightsCoverageLine(
                    coverage: patterns.coverage, copy: copy,
                    info: [text("analytics_patterns_intro"), text("analytics_patterns_overlap")]
                        + InsightsOverviewWords.feedLines(patterns.feed, copy: copy))
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .insightsSection()
        }
    }

    private func patternCard(_ card: InsightsPatternCard) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
            // How the figure was arrived at, behind the title's "i".
            InsightsSectionTitle(InsightsPatternsWords.title(card.kind, copy: copy),
                                 info: InsightsPatternsWords.basisLines(card, copy: copy))
            VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
                // The token figure is the headline and the count sits beneath
                // it; an estimate that rounds to 0 leads with the count, and
                // the token change is not drawn beside a count.
                HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s5) {
                    Text(InsightsPatternsWords.headline(card, copy: copy)).insightsFigure()
                    if card.change != nil, !InsightsPatternsWords.leadsWithCount(card) {
                        Text(InsightsPatternsWords.change(card, copy: copy)).insightsCaption()
                    }
                }
                Text(InsightsPatternsWords.countLine(card, copy: copy)).insightsNote()
                    .fixedSize(horizontal: false, vertical: true)
                weeklyBars(card)
            }
            .insightsCardBody()
            if card.sessions > 0, model.counter == nil {
                Button(InsightsPatternsWords.seeSessions(card, copy: copy)) {
                    if model.sessions?.pattern == card.kind { model.hideSessions() } else { model.showSessions(card.kind) }
                }
                .buttonStyle(GlassButtonStyle(.link))
            }
        }
        .frame(maxWidth: .infinity, alignment: .topLeading)
        .insightsSection()
    }

    /// The week's short date on the axis.
    static func weekLabel(_ weekStart: String) -> String {
        let parse = Date.ISO8601FormatStyle().year().month().day()
        guard let day = try? Date(weekStart, strategy: parse) else { return weekStart }
        return day.formatted(Date.FormatStyle(timeZone: TimeZone(secondsFromGMT: 0) ?? .current).month(.abbreviated).day())
    }

    /// Six weekly bars on a fixed axis of every week, so an absent week is a
    /// visible gap, marked with the dash. The week on screen is the last and
    /// carries its figure. A bar, not a ring; no run of weeks is counted
    /// (owner decision D1, open).
    private func weeklyBars(_ card: InsightsPatternCard) -> some View {
        let bars = InsightsPatternsWords.bars(card)
        let current = bars.last?.week
        return Chart {
            ForEach(bars) { bar in
                if let tokens = bar.tokens {
                    BarMark(x: .value(text("analytics_this_week"), Self.weekLabel(bar.week)),
                            y: .value(text("analytics_card_tokens"), tokens), width: .ratio(0.6))
                    .foregroundStyle(GlassTokens.Color.purpleSoft.color)
                    .annotation(position: .top, spacing: GlassTokens.Space.s1) {
                        if bar.week == current,
                           let label = InsightsPatternsWords.currentWeekFigure(card, copy: copy) {
                            Text(label).insightsTick(current: true)
                        }
                    }
                } else {
                    PointMark(x: .value(text("analytics_this_week"), Self.weekLabel(bar.week)),
                              y: .value(text("analytics_card_tokens"), 0))
                    .symbolSize(0)
                    .annotation(position: .top, spacing: 0) {
                        Text(text("analytics_unavailable")).insightsTick()
                    }
                }
            }
        }
        .chartXScale(domain: bars.map { Self.weekLabel($0.week) })
        .chartYAxis(.hidden)
        .chartXAxis {
            AxisMarks { value in
                AxisValueLabel {
                    if let label = value.as(String.self) {
                        Text(label).insightsTick(current: current.map(Self.weekLabel) == label)
                    }
                }
            }
        }
        .frame(height: 76)
        .insightsPlot()
        .accessibilityHidden(true)
    }

    /// "Your goals" (feed T only): each goal with six weekly marks, oldest
    /// first, and the change from last week. A met week is a filled square,
    /// a missed one an outlined square, a week with no figure the dash: shape,
    /// not colour alone. Marks, not a run of weeks (owner decision D1, open).
    private func goals(_ found: InsightsComparisons) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
            InsightsSectionTitle(text("analytics_goals_title"), info: [text("analytics_goals_note")])
            VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
                VStack(spacing: 0) {
                    ForEach(Array(found.goals.filter { InsightsComparisonsWords.goalKinds.contains($0.goal.kind) }.enumerated()), id: \.element.id) { index, goal in
                        GlassTableRow(first: index == 0) {
                            VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                                HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s4) {
                                    Text(InsightsComparisonsWords.goal(goal.goal, copy: copy))
                                        .fixedSize(horizontal: false, vertical: true)
                                    Spacer(minLength: GlassTokens.Space.s4)
                                    Text(InsightsComparisonsWords.goalFigure(goal, copy: copy)).insightsMono()
                                }
                                HStack(alignment: .center, spacing: GlassTokens.Space.s5) {
                                    HStack(spacing: GlassTokens.Space.s2) {
                                        ForEach(Array((goal.marks?.marks ?? []).enumerated()), id: \.offset) { _, mark in
                                            InsightsGoalMark(wire: mark, copy: copy)
                                        }
                                    }
                                    if let change = InsightsComparisonsWords.goalChange(goal, copy: copy) {
                                        Text(change).insightsCaption()
                                    }
                                    Spacer(minLength: GlassTokens.Space.s4)
                                    Button(text("analytics_goal_delete")) { comparisons.deleteGoal(goal.id) }
                                        .buttonStyle(GlassButtonStyle(.link))
                                }
                            }
                        }
                    }
                }
                HStack(spacing: GlassTokens.Space.s6) {
                    ForEach(["met", "not_met"], id: \.self) { wire in
                        HStack(spacing: GlassTokens.Space.s3) {
                            InsightsGoalMark(wire: wire, copy: copy).accessibilityHidden(true)
                            Text(InsightsComparisonsWords.mark(wire, copy: copy)).insightsCaption()
                        }
                    }
                }
            }
            .insightsCardBody()
            VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
                GlassSelect(text("analytics_goal_add"), selection: $goalKind,
                            options: InsightsComparisonsWords.goalKinds.map { kind in
                                GlassPickerOption(InsightsComparisonsWords.goal(
                                    InsightsGoal(kind: kind), copy: copy), value: kind)
                            })
                HStack(alignment: .center, spacing: GlassTokens.Space.s4) {
                    if InsightsComparisonsWords.goalNeedsSource(goalKind) {
                        GlassSelect(text("analytics_card_by_tool"), selection: $goalSource, options: [
                            GlassPickerOption(InsightsOverviewWords.harness("claude_code", copy: copy), value: "claude_code"),
                            GlassPickerOption(InsightsOverviewWords.harness("codex", copy: copy), value: "codex"),
                        ])
                        .fixedSize()
                    }
                    GlassTextField(text("analytics_goal_add"), text: $goalNumber, showsLabel: false)
                    Button(text("analytics_goal_add")) {
                        if let goal = InsightsComparisonsWords.newGoal(kind: goalKind, source: goalSource, number: goalNumber) {
                            comparisons.addGoal(goal); goalNumber = ""
                        }
                    }
                    .buttonStyle(GlassButtonStyle(.glass, small: true))
                    .disabled(InsightsComparisonsWords.newGoal(kind: goalKind, source: goalSource,
                                                               number: goalNumber) == nil)
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .insightsSection()
    }
}

/// One week of a goal: a filled square when met, an outlined one when not,
/// the dash when the week has no figure. Read out in the core's words.
struct InsightsGoalMark: View {
    let wire: String
    let copy: [String: String]

    static let side: CGFloat = 10

    var body: some View {
        Group {
            switch wire {
            case "met":
                RoundedRectangle(cornerRadius: 3, style: .continuous)
                    .fill(GlassTokens.Color.statusOn.color)
            case "not_met":
                RoundedRectangle(cornerRadius: 3, style: .continuous)
                    .strokeBorder(GlassColor.textTertiary, lineWidth: 1.5)
            default:
                Text(InsightsComparisonsWords.mark(wire, copy: copy)).insightsTick()
            }
        }
        .frame(width: Self.side, height: Self.side)
        .accessibilityElement()
        .accessibilityLabel(InsightsComparisonsWords.mark(wire, copy: copy))
    }
}

/// The sessions behind one card, in the core's order, under the card.
struct InsightsPatternSessionsView: View {
    let found: InsightsPatternSessions
    let copy: [String: String]

    private func text(_ key: String) -> String { copy[key] ?? "" }

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
            Text(InsightsPatternsWords.title(found.pattern, copy: copy)).insightsTitle()
            VStack(alignment: .leading, spacing: 0) {
                LazyVStack(spacing: 0) {
                    ForEach(found.sessions) { session in
                        #if DEBUG
                        let _ = OtherCollectionRowBodies.record("insightsPatternSessions", session.id)
                        #endif
                        InsightsDrillRow(
                            title: session.session_ref,
                            figure: InsightsOverviewWords.figure(session.tokens, copy: copy),
                            label: nil,
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
}
