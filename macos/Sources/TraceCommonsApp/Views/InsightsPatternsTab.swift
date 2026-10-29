import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// Breakdown over one feed at a time: the daemon's counter pass (feed T)
/// when it sent a week, the saved snapshots (feed S) otherwise. Every word
/// is the core's analytics copy, every figure the core's. Only feed T
/// compares weeks and carries Goals, the goals set with `/goal` in Claude
/// and Codex.
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
                // Goals set with /goal in Claude or Codex, whichever tool
                // the week used, so a Codex user sees theirs too.
                if patterns.feed == "counter_pass", let goals = week?.goals, goals.total > 0 {
                    self.goals(goals)
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

    /// Goals (feed T only): "Goals met {n} of {m}" over one row per goal
    /// set with `/goal` in Claude or Codex this week, newest first: the
    /// tool, its own status, tokens and time, and the folder. Never the
    /// goal's text, which stays in each tool. Hidden in a week with none.
    private func goals(_ goals: DaemonData.InsightsWeekGoals) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
            InsightsSectionTitle(text("analytics_goals_title"), info: [text("analytics_goals_note")])
            VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
                Text(InsightsGoalsWords.metOf(goals, copy: copy)).insightsFigure()
                VStack(spacing: 0) {
                    ForEach(Array(goals.goals.enumerated()), id: \.offset) { index, goal in
                        GlassTableRow(first: index == 0) {
                            HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s4) {
                                Text(InsightsGoalsWords.tool(goal.tool, copy: copy))
                                Text(InsightsGoalsWords.status(goal.status, copy: copy))
                                    .foregroundStyle(goal.met ? GlassTokens.Color.statusOnText.color : GlassColor.textSecondary)
                                Spacer(minLength: GlassTokens.Space.s4)
                                Text(InsightsGoalsWords.line(goal, copy: copy)).insightsMono()
                                if let folder = goal.folder {
                                    Text(folder).insightsCaption().lineLimit(1)
                                }
                            }
                            .accessibilityElement(children: .combine)
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
