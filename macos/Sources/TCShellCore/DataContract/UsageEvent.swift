import Foundation

/// The anonymous weekly usage counts a shell may report
/// (`record_usage_event {event}`, owner approval 2026-11-16 of #1920).
///
/// The whole list, in order, is
/// `trace_commons_protocol::usage_analytics::USAGE_EVENTS`; the daemon
/// refuses any other name, and `UsageEventTests` pins these raw values to
/// that Rust list. Whether a report is counted at all (the
/// `share_usage_data` setting and the US-only region check) is the
/// daemon's decision, never the shell's: a shell reports and ignores the
/// answer.
public enum UsageEvent: String, CaseIterable, Sendable {
    case tabHomeOpened = "tab_home_opened"
    case tabPrivateAiOpened = "tab_private_ai_opened"
    case tabTracesOpened = "tab_traces_opened"
    case tabInsightsOpened = "tab_insights_opened"
    case insightsOverviewOpened = "insights_overview_opened"
    case insightsBreakdownOpened = "insights_breakdown_opened"
    case insightsSessionsOpened = "insights_sessions_opened"
    case insightsAnalyzeOpened = "insights_analyze_opened"
    case insightsBenchmarksOpened = "insights_benchmarks_opened"
    case insightsRefreshClicked = "insights_refresh_clicked"
    case insightsAnalyzeClicked = "insights_analyze_clicked"
    case setupFinished = "setup_finished"
}
