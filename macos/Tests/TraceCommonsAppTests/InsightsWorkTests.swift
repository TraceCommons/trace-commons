import XCTest

import TCBridge
@testable import TCShellCore
@testable import TraceCommonsApp

/// Overview's week at a glance and Submitted's kinds of work (product owner, 2026-10-29),
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

    func testEveryKindOfWorkHasAName() throws {
        let copy = try words()
        for key in InsightsSubmittedWords.tasks {
            XCTAssertFalse(InsightsSubmittedWords.taskName(key, copy: copy).isEmpty, key)
        }
        XCTAssertEqual(InsightsSubmittedWords.taskName("bug_fix", copy: copy), "Fix a bug")
        XCTAssertEqual(InsightsSubmittedWords.taskName(nil, copy: copy), "Kind not recorded")
    }

    /// Breakdown's By model lists NEAR AI's models beside Claude's and
    /// Codex's (product owner, 2026-10-29): NEAR AI's figure is the ledger's
    /// charge by served model, "at least" while partly known, the unknown
    /// model in the core's word; and the view is offered with NEAR AI alone.
    func testByModelAddsNearAiModelsAtTheLedgersCharge() throws {
        let copy = try words()
        var object = try InsightsSessionRouteTests.recorded()
        object["work"] = nil
        let nearAiOnly = try InsightsSessionRouteTests.decode(object)
        XCTAssertNil(nearAiOnly.work)
        XCTAssertTrue(InsightsCreditWords.showsModels(nearAiOnly))
        XCTAssertEqual(InsightsCreditWords.modelLines(nearAiOnly, copy: copy).map(\.label), ["zai-org/glm-4.6"])

        var credit = try XCTUnwrap(object["near_ai_credit"] as? [String: Any])
        var week = try XCTUnwrap(credit["week"] as? [String: Any])
        var groups = try XCTUnwrap(week["by_model"] as? [[String: Any]])
        groups.append(["key": NSNull(), "usd_micros": NSNull(), "sessions": 1, "calls": 2,
                       "calls_without_charge": 1, "tokens": 50, "usd_micros_known": 300_000,
                       "usd_partial": true, "tokens_known": 50, "tokens_partial": false])
        week["by_model"] = groups
        credit["week"] = week
        object["near_ai_credit"] = credit
        let empty: [String: Any] = ["key": NSNull(), "sessions": 0, "priced_sessions": 0, "unpriced_sessions": 0,
                                    "usd_micros_known": 0, "usd_partial": false, "cache_savings_usd_micros_known": 0,
                                    "lines_added_known": 0, "lines_removed_known": 0, "lines_partial": false,
                                    "edits_known": 0, "edits_undone_known": 0, "edits_denied_known": 0,
                                    "edits_partial": false]
        object["work"] = ["basis": "api_list", "week": empty, "by_tool": [Any](), "by_folder": [Any](),
                          "by_day": [Any](),
                          "by_model": [["key": "claude-opus-4-7", "tool": "claude_code", "sessions": 1,
                                        "tokens": 1_000, "usd_micros_known": 2_000_000, "usd_partial": false]]]
        let lines = InsightsCreditWords.modelLines(try InsightsSessionRouteTests.decode(object), copy: copy)
        XCTAssertEqual(lines.map(\.label), ["claude-opus-4-7", "zai-org/glm-4.6", copy["analytics_unknown_label"]])
        XCTAssertEqual(lines.map(\.tool), ["Claude", "NEAR AI", "NEAR AI"])
        XCTAssertEqual(lines.map(\.amount), ["$2.00", "$0.0125", "at least $0.30"])
        XCTAssertEqual(Set(lines.map(\.id)).count, 3)
    }

    /// Breakdown's By project carries Claude and Codex spend at API prices,
    /// and the largest spend leads.
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
