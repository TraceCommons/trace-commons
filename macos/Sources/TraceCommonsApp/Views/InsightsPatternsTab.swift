import Charts
import SwiftUI
import TCBridge
import TCDesign

/// Patterns ("Where tokens went") over one feed at a time: the daemon's
/// counter pass (feed T) when it sent a week, the saved snapshots (feed S)
/// otherwise. Every word is the core's analytics copy, every figure the
/// core's. A week with no figure is a gap in the bars, never a zero bar;
/// files are letters with an extension, never a path. Only feed T compares
/// weeks and carries "Your goals"; under feed S the change reads as the dash.
struct InsightsPatternsTab: View {
    let model: InsightsPatternsModel
    let comparisons: InsightsComparisonsModel
    let copy: [String: String]
    /// Put a week on screen; the window routes it to the feed showing.
    var selectWeek: (String) -> Void = { _ in }
    @State private var goalKind = InsightsComparisonsWords.goalKinds[0]
    @State private var goalSource = "claude_code"
    @State private var goalNumber = ""

    private func text(_ key: String) -> String { copy[key] ?? "" }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s8) {
                if let patterns = model.shown {
                    header(patterns)
                    HStack(alignment: .top, spacing: GlassTokens.Space.s6) {
                        ForEach(patterns.cards) { card in
                            patternCard(card, threshold: patterns.long_context_threshold)
                        }
                    }
                    if let line = InsightsPatternsWords.claudeOnlyLine(patterns, copy: copy) {
                        Text(line).insightsCaption()
                    }
                    if let sessions = model.sessions {
                        InsightsPatternSessionsView(found: sessions, copy: copy)
                    }
                    rereadTable(patterns)
                    if patterns.feed == "counter_pass", let found = comparisons.comparisons {
                        goals(found)
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

    private func header(_ patterns: InsightsWeekPatterns) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
            HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s6) {
                Text(text("analytics_where_tokens_went")).insightsTitle()
                GlassSelect(text("analytics_this_week"), selection: Binding(
                    get: { patterns.week_start },
                    set: { selectWeek($0) }
                ), options: weekOptions(patterns))
                .frame(maxWidth: 220)
            }
            Text(text("analytics_patterns_intro")).insightsNote()
            Text(text("analytics_patterns_overlap")).insightsNote()
            Text(InsightsOverviewWords.coverageLine(patterns.coverage, copy: copy)).insightsCaption()
            ForEach(InsightsOverviewWords.feedLines(patterns.feed, copy: copy), id: \.self) { line in
                Text(line).insightsCaption()
            }
        }
    }

    private func weekOptions(_ patterns: InsightsWeekPatterns) -> [GlassPickerOption<String>] {
        var weeks = patterns.weeks
        if !weeks.contains(patterns.week_start) { weeks.insert(patterns.week_start, at: 0) }
        return weeks.map { start in
            // UTC midnights, so six whole days land on the Sunday whatever
            // the local zone's daylight changes do.
            let day = Date.ISO8601FormatStyle().year().month().day()
            let end = (try? Date(start, strategy: day))
                .map { $0.addingTimeInterval(6 * 86_400).formatted(day) } ?? start
            return GlassPickerOption(InsightsOverviewWords.weekRange(start: start, end: end), value: start)
        }
    }

    private func patternCard(_ card: InsightsPatternCard, threshold: UInt64) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
            Text(InsightsPatternsWords.title(card.kind, copy: copy)).insightsHeading()
            // The token figure is the headline; the count sits beneath it.
            Text(InsightsOverviewWords.figure(card.tokens, copy: copy))
                .glassType(GlassTokens.TypeScale.title).monospacedDigit()
            Text(InsightsPatternsWords.countLine(card, threshold: threshold, copy: copy)).insightsNote()
            weeklyBars(card)
            Text(InsightsPatternsWords.change(card, copy: copy)).insightsCaption()
            ForEach(InsightsPatternsWords.basisLines(card, copy: copy), id: \.self) { line in
                Text(line).insightsCaption()
            }
            if card.sessions > 0, model.counter == nil {
                Button(InsightsPatternsWords.seeSessions(card, copy: copy)) {
                    if model.sessions?.pattern == card.kind { model.hideSessions() } else { model.showSessions(card.kind) }
                }
                .buttonStyle(.link)
            }
        }
        .frame(maxWidth: .infinity, minHeight: 220, alignment: .topLeading)
        .insightsRowCard()
    }

    /// Six weekly bars on a fixed axis of every week, so an absent week is a
    /// visible gap. A bar, not a ring; no run of weeks is counted (owner
    /// decision D1, open).
    private func weeklyBars(_ card: InsightsPatternCard) -> some View {
        Chart(InsightsPatternsWords.drawnBars(card)) { bar in
            BarMark(x: .value(text("analytics_this_week"), bar.week),
                    y: .value(text("analytics_card_tokens"), bar.tokens ?? 0))
            .foregroundStyle(GlassColor.accentText.opacity(0.55))
        }
        .chartXScale(domain: InsightsPatternsWords.bars(card).map(\.week))
        .chartXAxis(.hidden)
        .chartYAxis(.hidden)
        .frame(height: 48)
        .accessibilityHidden(true)
    }

    /// "Your goals" (feed T only): each goal with six weekly marks, oldest
    /// first, and the change from last week. Marks, not a run of weeks
    /// (owner decision D1, open).
    private func goals(_ found: InsightsComparisons) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
            Text(text("analytics_goals_title")).insightsHeading()
            Text(text("analytics_goals_note")).insightsCaption()
            ForEach(found.goals) { goal in
                GlassTableRow {
                    HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s6) {
                        VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                            Text(InsightsComparisonsWords.goal(goal.goal, copy: copy))
                            Text(InsightsComparisonsWords.goalFigure(goal, copy: copy)).insightsMono()
                            if let change = InsightsComparisonsWords.goalChange(goal, copy: copy) {
                                Text(change).insightsCaption()
                            }
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                        HStack(spacing: GlassTokens.Space.s2) {
                            ForEach(Array((goal.marks?.marks ?? []).enumerated()), id: \.offset) { _, mark in
                                Text(InsightsComparisonsWords.mark(mark, copy: copy)).insightsCaption()
                                    .frame(minWidth: 44)
                            }
                        }
                        Button(text("analytics_goal_delete")) { comparisons.deleteGoal(goal.id) }
                            .buttonStyle(GlassButtonStyle(.link))
                    }
                }
            }
            HStack(alignment: .bottom, spacing: GlassTokens.Space.s4) {
                GlassSelect(text("analytics_goal_add"), selection: $goalKind,
                            options: InsightsComparisonsWords.goalKinds.map { kind in
                                GlassPickerOption(InsightsComparisonsWords.goal(
                                    InsightsGoal(kind: kind), copy: copy), value: kind)
                            })
                if InsightsComparisonsWords.goalNeedsSource(goalKind) {
                    GlassSelect(text("analytics_card_by_tool"), selection: $goalSource, options: [
                        GlassPickerOption(InsightsOverviewWords.harness("claude_code", copy: copy), value: "claude_code"),
                        GlassPickerOption(InsightsOverviewWords.harness("codex", copy: copy), value: "codex"),
                    ])
                }
                GlassTextField(text("analytics_goal_add"), text: $goalNumber, showsLabel: false)
                    .frame(maxWidth: 140)
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
        .insightsCard()
    }

    private func rereadTable(_ patterns: InsightsWeekPatterns) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
            Text(text("analytics_reread_title")).insightsHeading()
            if patterns.reread_files.isEmpty {
                Text(text("analytics_unavailable")).insightsNote()
            } else {
                GlassTableHead {
                    HStack {
                        Text(text("analytics_reread_file")).frame(maxWidth: .infinity, alignment: .leading)
                        Text(text("analytics_reread_reads")).frame(width: 70, alignment: .trailing)
                        Text(text("analytics_reread_after_shrink")).frame(width: 200, alignment: .trailing)
                        Text(text("analytics_reread_tokens")).frame(width: 90, alignment: .trailing)
                    }
                }
                ForEach(patterns.reread_files) { row in
                    GlassTableRow {
                        HStack {
                            Text(InsightsPatternsWords.fileLabel(row, copy: copy)).insightsMono()
                                .frame(maxWidth: .infinity, alignment: .leading)
                            Text(String(row.reads)).insightsMono().frame(width: 70, alignment: .trailing)
                            Text(String(row.after_shrink)).insightsMono().frame(width: 200, alignment: .trailing)
                            Text(InsightsOverviewWords.figure(row.tokens, copy: copy)).insightsMono()
                                .frame(width: 90, alignment: .trailing)
                        }
                    }
                }
            }
        }
        .insightsCard()
    }
}

/// The sessions behind one card, in the core's order.
struct InsightsPatternSessionsView: View {
    let found: InsightsPatternSessions
    let copy: [String: String]

    private func text(_ key: String) -> String { copy[key] ?? "" }

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
            Text(InsightsPatternsWords.title(found.pattern, copy: copy)).insightsHeading()
            GlassTableHead {
                HStack {
                    Text(text("analytics_drill_session")).frame(maxWidth: .infinity, alignment: .leading)
                    Text(text("analytics_drill_tokens")).frame(width: 90, alignment: .trailing)
                    Text(text("analytics_drill_coverage")).frame(width: 90, alignment: .leading)
                    Text(text("analytics_drill_reason")).frame(maxWidth: .infinity, alignment: .leading)
                }
            }
            ForEach(found.sessions) { session in
                GlassTableRow {
                    HStack(alignment: .top) {
                        Text(session.session_ref).insightsMono().lineLimit(1).truncationMode(.middle)
                            .frame(maxWidth: .infinity, alignment: .leading)
                        Text(InsightsOverviewWords.figure(session.tokens, copy: copy))
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
}
