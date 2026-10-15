import AppKit
import SwiftUI
import TCBridge
import TCDesign
import XCTest
@testable import TraceCommonsApp

/// Direction B (2026-10-15): the Insights tabs are laid out for the main
/// pane at the window's default width, 440pt, which leaves 408pt inside the
/// pane's insets. This renders each tab there from a fixture week through
/// the real models, so the layout can be looked at without a store.
///
/// Set `TC_INSIGHTS_RENDER_DIR` to keep the PNGs, and `TC_GLASS_THEME=flat`
/// to draw them in the theme the app ships with (a test run is classic).
final class InsightsPaneWidthRenderTests: XCTestCase {
    /// `GlassPaneLayout.mainWidth` less the main pane's insets on each side.
    static var paneContentWidth: CGFloat {
        GlassPaneLayout.mainWidth - 2 * MonitorWindowView.mainInset
    }

    func test_thePaneContentWidthIs408AtTheDefaultWindow() {
        XCTAssertEqual(Self.paneContentWidth, 408)
    }

    @MainActor
    func test_everyTabRendersAtThePaneWidth() async throws {
        let copy = try XCTUnwrap(TCInsights.copy())
        let service: InsightsModel.Service = { request in try Fixture.response(request.operation.type) }

        let overview = InsightsOverviewModel(service: service)
        let patterns = InsightsPatternsModel(service: service)
        let sessions = InsightsSessionsModel(service: service)
        let comparisons = InsightsComparisonsModel(service: service)
        let week = try Fixture.decode(InsightsWeekOverview.self, Fixture.overview)
        let pattern = try Fixture.decode(InsightsWeekPatterns.self, Fixture.patterns)
        overview.open(); patterns.open(); sessions.open()
        overview.showCounter(week)
        patterns.showCounter(pattern)
        sessions.sync(snapshotIDs: ["snap-1"])
        comparisons.load(counterWeeks: [], weekStart: week.week_start, recapCardEnabled: false)
        try await settle { !overview.busy && !patterns.busy && !sessions.busy && !comparisons.busy }
        XCTAssertNotNil(comparisons.comparisons)
        XCTAssertNotNil(sessions.drill)

        let tabs: [(String, CGFloat, AnyView)] = [
            ("overview", 1560, AnyView(InsightsOverviewTab(
                model: overview, comparisons: comparisons, copy: copy, snapshots: [], patterns: pattern))),
            ("patterns", 1900, AnyView(InsightsPatternsTab(model: patterns, comparisons: comparisons, copy: copy))),
            ("sessions", 1000, AnyView(InsightsSessionsTab(model: sessions, snapshots: [], copy: copy))),
        ]
        for (name, height, view) in tabs {
            for dark in [true, false] {
                let png = try Self.render(view, height: height, dark: dark)
                XCTAssertGreaterThan(png.count, 20_000, name)
                if let dir = ProcessInfo.processInfo.environment["TC_INSIGHTS_RENDER_DIR"] {
                    try png.write(to: URL(fileURLWithPath: dir)
                        .appendingPathComponent("insights-\(name)-\(dark ? "dark" : "light").png"))
                }
            }
        }
    }

    @MainActor
    private func settle(_ done: () -> Bool) async throws {
        for _ in 0 ..< 200 where !done() { try await Task.sleep(nanoseconds: 10_000_000) }
        XCTAssertTrue(done(), "the models did not settle")
    }

    /// The tab on a stand-in for the pane's glass at the pane's content width.
    @MainActor
    static func render(_ view: AnyView, height: CGFloat, dark: Bool) throws -> Data {
        _ = NSApplication.shared
        let size = CGSize(width: paneContentWidth + 2 * MonitorWindowView.mainInset, height: height)
        let content = view
            .frame(width: paneContentWidth, height: height)
            .padding(.horizontal, MonitorWindowView.mainInset)
            .background {
                // Glass over a mid-grey desktop: the veil the app paints over
                // its material, so cards and plots read as they do on screen.
                ZStack { Color(red: 0.36, green: 0.40, blue: 0.47); GlassTokens.Color.glassVeil.color }
            }
            .environment(\.colorScheme, dark ? .dark : .light)
        let hosting = NSHostingView(rootView: content)
        let bounds = NSRect(origin: .zero, size: size)
        hosting.frame = bounds
        let window = NSWindow(contentRect: bounds, styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
        window.contentView = hosting
        defer { window.close() }
        hosting.layoutSubtreeIfNeeded(); window.displayIfNeeded()
        let bitmap = try XCTUnwrap(hosting.bitmapImageRepForCachingDisplay(in: bounds))
        hosting.cacheDisplay(in: bounds, to: bitmap)
        return try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
    }
}

/// A watched-folder week (feed T) shaped as the core sends it: sample
/// figures, never a pilot's.
private enum Fixture {
    static func decode<T: Decodable>(_ type: T.Type, _ json: String) throws -> T {
        try JSONDecoder().decode(T.self, from: Data(json.utf8))
    }

    static func response(_ type: String) throws -> InsightsResponse {
        let body: String
        switch type {
        case "week_overview": body = #""overview": \#(overview)"#
        case "patterns": body = #""patterns": \#(patterns)"#
        case "comparisons": body = #""comparisons": \#(comparisons)"#
        case "session_drill": body = #""session": \#(session)"#
        default: throw InsightsError.invalidResponse
        }
        return try decode(InsightsResponse.self, #"{"type": "\#(type)", \#(body)}"#)
    }

    static let coverage = #"{"known": 34, "partial": 2, "unknown": 1, "reasons": {}}"#
    static let weeks = #"["2026-10-05", "2026-09-28", "2026-09-21", "2026-09-07", "2026-08-31"]"#

    static let overview = #"""
    {"feed": "counter_pass", "week_start": "2026-10-05", "week_end": "2026-10-11", "tz": 0,
     "coverage": \#(coverage), "undated_sessions": 0, "sessions": 37,
     "sources": [
      {"source": "claude_code", "sessions": 31, "tokens": 45000000,
       "cache_share": {"numerator": 38500000, "denominator": 44150000, "permille": 872},
       "largest_session": {"tokens": 4400000}, "change_permille": -120,
       "best": {"previous_best_permille": 840, "is_new_best": true}},
      {"source": "codex", "sessions": 6, "tokens": 6200000,
       "cache_share": {"numerator": 4588000, "denominator": 6200000, "permille": 740},
       "largest_session": {"tokens": 1300000}, "change_permille": 40, "best_week": "not_best"}],
     "by_day": [
      {"date": "2026-10-05", "uncached": 400000, "cache_read": 6100000, "cache_write": 500000, "output": 120000},
      {"date": "2026-10-06", "uncached": 600000, "cache_read": 8300000, "cache_write": 700000, "output": 150000},
      {"date": "2026-10-07", "uncached": 500000, "cache_read": 9000000, "cache_write": 800000, "output": 180000},
      {"date": "2026-10-08", "uncached": 400000, "cache_read": 7200000, "cache_write": 600000, "output": 140000},
      {"date": "2026-10-09", "uncached": 300000, "cache_read": 5400000, "cache_write": 400000, "output": 100000},
      {"date": "2026-10-10", "uncached": 50000, "cache_read": 900000, "cache_write": 100000, "output": 20000},
      {"date": "2026-10-11", "uncached": 100000, "cache_read": 1600000, "cache_write": 200000, "output": 30000}],
     "codex_interval_tokens": 6200000,
     "by_model": [{"label": "claude-haiku-5-5", "tokens": 2100000}, {"label": "claude-opus-5-5", "tokens": 29000000},
                  {"label": "claude-sonnet-5-5", "tokens": 14000000}, {"label": "gpt-5-codex", "tokens": 6200000},
                  {"label": null, "tokens": 800000}],
     "by_tool": [{"source": "claude_code", "tokens": 45000000}, {"source": "codex", "tokens": 6200000}],
     "by_project": "not_available_for_analyzed_files", "weeks": \#(weeks)}
    """#

    static func patternWeeks(_ values: [Int?]) -> String {
        let starts = ["2026-08-31", "2026-09-07", "2026-09-14", "2026-09-21", "2026-09-28", "2026-10-05"]
        return "[" + zip(starts, values).map { start, value in
            #"{"week_start": "\#(start)", "tokens": \#(value.map(String.init) ?? "null")}"#
        }.joined(separator: ", ") + "]"
    }

    static let patterns = #"""
    {"feed": "counter_pass", "week_start": "2026-10-05", "week_end": "2026-10-11", "tz": 0,
     "coverage": \#(coverage), "sessions": 37, "claude_sessions": 31, "claude_only": true,
     "long_context_threshold": 150000,
     "cards": [
      {"kind": "repeated_reads", "tokens": 2300000, "count": 41, "files": 6, "sessions": 9,
       "basis": "estimate_from_result_size", "inferred": false, "change": 180,
       "weeks": \#(patternWeeks([1_100_000, 1_600_000, nil, 1_900_000, 1_500_000, 2_300_000]))},
      {"kind": "retried_calls", "tokens": 410000, "count": 37, "sessions": 7,
       "basis": "estimate_from_result_size", "inferred": false, "change": -60,
       "weeks": \#(patternWeeks([520_000, 470_000, nil, 390_000, 440_000, 410_000]))},
      {"kind": "edit_fail_edit", "tokens": 180000, "count": 12, "sessions": 5,
       "basis": "estimate_from_result_size", "inferred": true, "change": 90,
       "weeks": \#(patternWeeks([120_000, 200_000, nil, 150_000, 165_000, 180_000]))},
      {"kind": "long_context", "tokens": 9800000, "sessions": 6,
       "basis": "from_counters", "inferred": false, "change": 310,
       "weeks": \#(patternWeeks([4_100_000, 6_300_000, nil, 5_200_000, 7_500_000, 9_800_000]))}],
     "reread_files": [
      {"letter": "A", "ext": "ts", "reads": 9, "after_shrink": 2, "tokens": 640000},
      {"letter": "B", "ext": "rs", "reads": 8, "after_shrink": 0, "tokens": 510000},
      {"letter": "C", "ext": "md", "reads": 7, "after_shrink": 3, "tokens": 420000},
      {"letter": "D", "ext": "json", "reads": 6, "after_shrink": 0, "tokens": 300000},
      {"letter": "E", "reads": 5, "after_shrink": 0, "tokens": 150000}],
     "weeks": \#(weeks)}
    """#

    static let comparisons = #"""
    {"feed": "counter_pass", "week_start": "2026-10-05",
     "goals": [
      {"id": "g1", "goal": {"kind": "cache_share_at_least", "source": "claude_code", "permille": 850}, "figure": 872,
       "marks": {"marks": ["not_met", "not_met", "no_figure", "met", "not_met", "met"],
                 "change": {"direction": "up", "from": 830}}},
      {"id": "g2", "goal": {"kind": "repeated_reads_under", "tokens": 2000000}, "figure": 2300000,
       "marks": {"marks": ["met", "met", "no_figure", "met", "met", "not_met"],
                 "change": {"direction": "up", "from": 1500000}}}],
     "lever": {"pick": {"kind": "repeated_reads", "tokens": 2300000, "count": 41, "files": 6, "ratio_permille": 1500},
               "disabled": [], "card_enabled": true}}
    """#

    static var session: String {
        var turns: [String] = []
        for turn in 1 ... 52 {
            let context = turn < 48 ? 14_000 + 3_000 * turn : 60_000 + 3_000 * (turn - 48)
            let write = turn == 31 ? 96_000 : 3_000
            let uncached = turn == 31 ? 2_000 : 1_500
            let read = context - write - uncached
            turns.append(#"{"ordinal": \#(turn), "uncached": \#(uncached), "cache_read": \#(read), "cache_write": \#(write), "output": 900, "context": \#(context)}"#)
        }
        return #"""
        {"feed": "saved", "session_ref": "snap-1", "source": "claude_code", "date": "2026-10-09", "tz": 0,
         "turns": 52, "tokens": 4400000, "span_secs": 6480, "state": "known", "reasons": [],
         "long_context_threshold": 150000, "series": [\#(turns.joined(separator: ", "))],
         "markers": [
          {"letter": "A", "turn_ordinal": 31, "kind": "cache_written_again", "basis": "inferred_from_counters",
           "pause_minutes": 7, "cache_write": 96000},
          {"letter": "B", "turn_ordinal": 37, "kind": "re_read", "basis": "from_tool_calls", "file_letter": "A", "file_ext": "ts"},
          {"letter": "C", "turn_ordinal": 46, "kind": "crossed_long_context", "basis": "from_counters",
           "context_from": 149000, "context": 152000},
          {"letter": "D", "turn_ordinal": 48, "kind": "context_shrank", "basis": "inferred_from_counters",
           "context_from": 155000, "context": 60000}]}
        """#
    }
}
