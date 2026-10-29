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

    /// Benchmarks' population picker (product owner, 2026-10-29): the
    /// person's own traces, or Claude's sub-agent transcripts, in the core's
    /// words; the intro names which.
    func testBenchmarkPopulationsReadInTheCoresWords() throws {
        let copy = try words()
        XCTAssertEqual(InsightsBenchmarkWords.populations, ["own", "spawned"])
        XCTAssertEqual(InsightsBenchmarkWords.population("own", copy: copy), "Your traces")
        XCTAssertEqual(InsightsBenchmarkWords.population("spawned", copy: copy), "Sub-agents")
        XCTAssertEqual(InsightsBenchmarkWords.intro(12, population: "own", copy: copy), "Compared on 12 of your traces")
        XCTAssertEqual(InsightsBenchmarkWords.intro(40, population: "spawned", copy: copy),
                       "Compared on 40 Claude sub-agent traces")
    }

    func testBenchmarksDecodeTheirPopulationAndDelegatedCount() throws {
        let json = #"""
        {"enabled":true,"readable":true,"weeks":4,"population":"spawned","sessions":3,"min_sessions":5,
         "categories":[],"delegated":0}
        """#
        let answer = try JSONDecoder().decode(DaemonData.InsightsBenchmarks.self, from: Data(json.utf8))
        XCTAssertEqual(answer.population, "spawned")
        XCTAssertEqual(answer.delegated, 0)
        let older = try JSONDecoder().decode(DaemonData.InsightsBenchmarks.self,
                                             from: Data(#"{"enabled":true,"categories":[]}"#.utf8))
        XCTAssertNil(older.population, "an older daemon answers the person's own only")
    }

    /// The model asks for the population chosen, and a sample model (the
    /// get-started card's) has no reader to ask.
    func testTheBenchmarksModelAsksForTheChosenPopulation() async throws {
        let asked = AskedLog()
        let model = InsightsBenchmarksModel(reader: { weeks, population in
            await asked.add("\(weeks) \(population)")
            return DaemonData.InsightsBenchmarks(categories: [])
        })
        XCTAssertEqual(model.population, "own", "the person's own by default")
        await model.load()
        model.population = "spawned"
        model.weeks = 13
        await model.load()
        let log = await asked.items
        XCTAssertEqual(log, ["4 own", "13 spawned"])
        let sample = InsightsBenchmarksModel(sample: DaemonData.InsightsBenchmarks(categories: []))
        sample.population = "spawned"
        await sample.load()
        XCTAssertNotNil(sample.answer, "the sample stays drawn")
    }

    /// The header counts the traces compared, not the window: 1,382 in the
    /// window with 1,360 left out (automated and the rest) is 22 compared.
    func testBenchmarksHeaderCountsTheTracesActuallyCompared() throws {
        let copy = try words()
        let categories = [
            DaemonData.InsightsBenchmarkCategory(key: "bug_fix", sessions: 14),
            DaemonData.InsightsBenchmarkCategory(key: "feature", sessions: 8),
            DaemonData.InsightsBenchmarkCategory(key: "tests", sessions: 0),
        ]
        let answer = DaemonData.InsightsBenchmarks(sessions: 1_382, categories: categories)
        XCTAssertEqual(InsightsBenchmarkWords.compared(answer), 22)
        XCTAssertEqual(InsightsOverviewWords.fill(copy["analytics_bench_intro"] ?? "",
                                                  ["n": String(InsightsBenchmarkWords.compared(answer))]),
                       "Compared on 22 of your traces")
        XCTAssertEqual(InsightsBenchmarkWords.compared(.init(sessions: 5, categories: nil)), 0)
    }

    func testBenchmarkLabelsAreShortAndTheirDetailsAreBehindAnI() throws {
        let copy = try words()
        XCTAssertEqual(copy["analytics_bench_quality"], "Quality")
        XCTAssertEqual(copy["analytics_bench_value"], "Value")
        XCTAssertEqual(copy["analytics_bench_speed"], "Speed")
        XCTAssertEqual(copy["analytics_bench_quality_detail"], "Share of edits you kept")
        XCTAssertEqual(copy["analytics_bench_value_detail"], "Cost per trace at API prices")
        XCTAssertEqual(copy["analytics_bench_speed_detail"], "Time per trace")
        XCTAssertEqual(copy["analytics_bench_not_enough"], "Waiting for more data")
        XCTAssertNil(copy["analytics_bench_min"])
        XCTAssertNil(copy["analytics_bench_empty"])
        let category = DaemonData.InsightsBenchmarkCategory(
            key: "bug_fix", sessions: 9,
            quality: .init(leader: .init(label: "a", tool: "codex", figure: 900),
                           next: .init(label: "b", tool: "codex", figure: 800)))
        XCTAssertEqual(InsightsBenchmarkWords.picks(category, copy: copy).map(\.info), ["Share of edits you kept"])
    }

    /// The header is the title and the picker alone; each card carries its
    /// own labels with an "i".
    func testBenchmarksHeaderHasNoLegend() throws {
        let source = try InsightsGlassConventionsTests.text("Views/InsightsBenchmarksTab.swift")
        XCTAssertFalse(source.contains("analytics_bench_min"))
        XCTAssertFalse(source.contains("insightsCaption()\n                }\n            }\n            Text(InsightsOverviewWords.fill(text(\"analytics_bench_min"), "the legend left the header")
        XCTAssertFalse(source.contains(" · "), "no joined title and detail")
        XCTAssertTrue(source.contains("InsightsInfo(pick.title, [pick.info])"))
    }

    func testEveryBenchmarkCategoryHasAName() throws {
        let copy = try words()
        for key in InsightsBenchmarkWords.categories {
            XCTAssertFalse(InsightsBenchmarkWords.name(key, copy: copy).isEmpty, key)
        }
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
        XCTAssertEqual(lines.map(\.amount), ["$2.00", "$0.01", "$0.30"], "to the cent, no at least")
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

private actor AskedLog {
    private(set) var items: [String] = []
    func add(_ item: String) { items.append(item) }
}
