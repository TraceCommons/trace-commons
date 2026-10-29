import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// Breakdown over one feed at a time: the daemon's counter pass (feed T)
/// when it sent a week, the saved snapshots (feed S) otherwise. Every word
/// is the core's analytics copy, every figure the core's. Only feed T
/// compares weeks and carries "Your goals".
///
/// Laid out for the main pane (direction B, 2026-10-15): the week's scope,
/// the drill-down, Tokens by day and the goals, in a single column. No
/// pattern card is drawn (owner, 2026-10-29); a picked day's sessions are
/// the window's inspector's (`InsightsBreakdownInspector`).
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
    /// The window's inspector, where a picked day's sessions are shown
    /// (owner, 2026-10-29).
    var inspector = InsightsBreakdownInspector()
    /// The get-started card over this tab drawn from the sample (product
    /// owner, 2026-10-29); `nil` for the person's own week.
    var getStarted: InsightsGetStarted?
    @State private var goalKind = InsightsComparisonsWords.goalKinds[0]
    @State private var goalSource = "claude_code"
    @State private var goalNumber = ""

    private func text(_ key: String) -> String { copy[key] ?? "" }

    var body: some View {
        InsightsTabPage(getStarted: getStarted) {
            if let patterns = model.shown {
                header(patterns)
                // Details leads with the drill-down, by project or by model
                // (product owner, 2026-10-28; By task and By tool removed
                // 2026-10-29). No pattern card is shown any more (owner,
                // 2026-10-29: Retried tool calls, the last, was removed);
                // the goals live only here.
                if let week { InsightsDetailsTable(week: week, copy: copy, dimension: detailsView) }
                // Tokens by day moved here from Overview (product owner,
                // 2026-10-28); a picked day's sessions are the inspector's
                // (owner, 2026-10-29).
                if let overview {
                    InsightsTokensByDay(model: overview, copy: copy, pickedDay: inspector.pickedDay,
                                        pick: { inspector.pick(day: $0) })
                }
                if patterns.feed == "counter_pass", let found = comparisons.comparisons {
                    goals(found)
                }
            } else if model.failed {
                Text(text("analytics_unavailable")).insightsError()
            }
            if model.busy { GlassSpinner() }
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
