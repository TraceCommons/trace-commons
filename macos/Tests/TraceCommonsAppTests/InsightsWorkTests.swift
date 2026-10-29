import XCTest

import TCBridge
@testable import TCShellCore
@testable import TraceCommonsApp

/// Overview's week at a glance and Benchmarks (product owner, 2026-10-29),
/// from the core's own words.
@MainActor
final class InsightsWorkTests: XCTestCase {
    private typealias Totals = DaemonData.InsightsWorkTotals

    private func words() throws -> [String: String] { try XCTUnwrap(TCInsights.copy()) }

    func testSpendIsItsKnownSumToTheCent() throws {
        let copy = try words()
        XCTAssertEqual(InsightsWorkWords.spend(Totals(sessions: 2, usdMicrosKnown: 63_400_000), copy: copy), "$63.40")
        XCTAssertEqual(InsightsWorkWords.spend(Totals(sessions: 2, unpricedSessions: 1, usdMicrosKnown: 5_000_000,
                                                      usdPartial: true), copy: copy), "$5.00",
                       "a partly known sum shows plain; the coverage line says how many traces it covers")
        XCTAssertEqual(InsightsWorkWords.spend(Totals(sessions: 1, pricedSessions: 0, usdMicrosKnown: 0, usdPartial: true),
                                               copy: copy), "—", "nothing priced is the dash, never $0.00")
        XCTAssertEqual(InsightsWorkWords.spend(Totals(sessions: 2, usdMicrosKnown: 896_212_500), copy: copy), "$896.21")
        XCTAssertEqual(InsightsWorkWords.spend(Totals(sessions: 1, usdMicrosKnown: 40), copy: copy), "$0.00004",
                       "a charge under a cent keeps a significant digit")
    }

    func testChangeVsLastWeekNeedsLastWeeksFigure() throws {
        let copy = try words()
        XCTAssertEqual(InsightsWorkWords.change(118, last: 100, copy: copy)?.words, "▲ 18% vs last week")
        XCTAssertEqual(InsightsWorkWords.change(88, last: 100, copy: copy)?.words, "▼ 12% vs last week")
        XCTAssertNil(InsightsWorkWords.change(5, last: nil, copy: copy))
        XCTAssertNil(InsightsWorkWords.change(5, last: 0, copy: copy))
        XCTAssertNil(InsightsWorkWords.change(12, last: 30, partial: true, copy: copy),
                     "a minimum against an exact sum says nothing about the change")
        XCTAssertEqual(InsightsWorkWords.spend(Totals(sessions: 1, pricedSessions: 0, usdMicrosKnown: 40_000_000,
                                                      usdPartial: true), copy: copy),
                       "$40.00", "a known part shows even when no trace priced in full")
    }

    func testModelRowsShareTheKnownSpend() throws {
        let copy = try words()
        let work = DaemonData.InsightsWork(week: Totals(sessions: 2, usdMicrosKnown: 4_000_000), byModel: [
            .init(key: "gpt-5.5", tool: "codex", sessions: 1, tokens: 10, usdMicrosKnown: 3_000_000),
            .init(key: "claude-opus", tool: "claude_code", sessions: 1, tokens: 10, usdMicrosKnown: 1_000_000),
            .init(key: "unpriced", tool: "codex", sessions: 1, tokens: 10, usdMicrosKnown: 0, usdPartial: true),
        ])
        let rows = InsightsWorkWords.modelRows(work, copy: copy)
        XCTAssertEqual(rows.map(\.label), ["gpt-5.5", "claude-opus"])
        XCTAssertEqual(rows.map(\.percent), ["75%", "25%"])
        XCTAssertEqual(rows.first?.tool, "Codex")
    }

    func testWorkDecodesFromTheWire() throws {
        let json = #"""
        {"basis":"api_list","week":{"sessions":2,"priced_sessions":1,"unpriced_sessions":1,"usd_micros_known":5,
         "usd_partial":true,"cache_savings_usd_micros_known":1,
         "edits_known":4,"edits_undone_known":1,"edits_denied_known":0,"edits_partial":false},
         "last_week":null,"by_tool":[],"by_model":[],"by_folder":[],"by_day":[{"date":"2026-10-23","usd_micros_known":5,
         "usd_partial":false}],"last_week_by_day":null}
        """#
        let work = try JSONDecoder().decode(DaemonData.InsightsWork.self, from: Data(json.utf8))
        XCTAssertEqual(work.week.editsKeptKnown, 3)
        XCTAssertTrue(work.week.usdPartial)
        XCTAssertEqual(work.byDay.first?.date, "2026-10-23")
    }

    func testBenchmarkPicksAndTipsReadInTheCoresWords() throws {
        let copy = try words()
        var tip = DaemonData.InsightsBenchmarkTip(kind: "test_first")
        tip.fasterPermille = 350; tip.sessionsWithout = 12; tip.sessions = 34
        let category = DaemonData.InsightsBenchmarkCategory(
            key: "bug_fix", sessions: 34,
            quality: .init(leader: .init(label: "gpt-5.5", tool: "codex", figure: 930),
                           next: .init(label: "opus", tool: "claude_code", figure: 880)),
            value: .init(leader: .init(label: "sonnet", tool: "claude_code", figure: 400_000),
                         next: .init(label: "gpt-5.5", tool: "codex", figure: 950_000)),
            speed: nil, tip: tip)
        let picks = InsightsBenchmarkWords.picks(category, copy: copy)
        XCTAssertEqual(picks.map(\.id), ["quality", "value"], "a measure without two qualifying models is left out")
        XCTAssertEqual(picks[0].figure, "93% kept")
        XCTAssertEqual(picks[0].next, "Next: opus, 88% kept")
        XCTAssertEqual(picks[1].figure, "$0.40 per trace")
        XCTAssertEqual(InsightsBenchmarkWords.name("bug_fix", copy: copy), "Fix a bug")
        XCTAssertEqual(InsightsBenchmarkWords.tip(tip, copy: copy),
                       "Fixes where a failing test ran first finished 35% faster. 12 of your 34 fixes did not start that way.")
        XCTAssertNil(InsightsBenchmarkWords.tip(.init(kind: "from_a_newer_daemon"), copy: copy))
    }

    func testEveryBenchmarkCategoryHasAName() throws {
        let copy = try words()
        for key in InsightsBenchmarkWords.categories {
            XCTAssertFalse(InsightsBenchmarkWords.name(key, copy: copy).isEmpty, key)
        }
    }

    /// Breakdown's By project carries Claude and Codex spend at API prices
    /// at API prices, and the largest spend leads.
    func testByProjectAddsSpendFromWork() throws {
        let copy = try words()
        var object = try InsightsSessionRouteTests.recorded()
        var rollup = try XCTUnwrap(object["rollup"] as? [String: Any])
        var rows = try XCTUnwrap(rollup["sessions"] as? [[String: Any]])
        rows[0]["folder"] = "app"
        rows[1]["folder"] = "site"
        rollup["sessions"] = rows
        object["rollup"] = rollup
        func totals(_ key: String, usd: Int) -> [String: Any] {
            ["key": key, "sessions": 1, "priced_sessions": 1, "unpriced_sessions": 0, "usd_micros_known": usd,
             "usd_partial": false, "cache_savings_usd_micros_known": 0, "edits_known": 0, "edits_undone_known": 0,
             "edits_denied_known": 0, "edits_partial": false]
        }
        var week = totals("week", usd: 2_100_000)
        week["key"] = NSNull()
        object["work"] = ["basis": "api_list", "week": week, "by_tool": [Any](), "by_model": [Any](),
                          "by_folder": [totals("site", usd: 2_000_000), totals("app", usd: 100_000)],
                          "by_day": [Any]()]
        let decoded = try InsightsSessionRouteTests.decode(object)
        let table = InsightsProjectRows.rows(decoded, copy: copy)
        XCTAssertEqual(table.first?.label, "site", "the largest spend leads")
        XCTAssertEqual(table.first?.spend, "$2.00")
    }
}
