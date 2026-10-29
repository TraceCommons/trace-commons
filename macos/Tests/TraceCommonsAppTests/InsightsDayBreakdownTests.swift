import TCBridge
import TCShellCore
import XCTest
@testable import TraceCommonsApp

/// A day clicked on "Tokens by day" narrows the Sessions breakdown to that
/// day's sessions, by their tokens that day (Zaki's demo, 2026-10-26).
@MainActor
final class InsightsDayBreakdownTests: XCTestCase {
    private struct Session: Identifiable, Equatable {
        let id: String
        let days: [UInt64]?
    }

    private func days(_ dates: [String]) throws -> [InsightsDayTokens] {
        let object = dates.map { ["date": $0, "uncached": 0, "cache_read": 0, "cache_write": 0, "output": 0] }
        return try JSONDecoder().decode([InsightsDayTokens].self, from: JSONSerialization.data(withJSONObject: object))
    }

    func testAPickedDateIsItsPlaceInTheWeekAndItsWeekday() throws {
        let week = try days((5...11).map { String(format: "2026-10-%02d", $0) })
        let english = Locale(identifier: "en_US_POSIX")
        XCTAssertEqual(InsightsDayPick.pick("2026-10-06", days: week, locale: english),
                       InsightsDayPick(index: 1, weekday: "Tuesday"))
        XCTAssertEqual(InsightsDayPick.pick("2026-10-06", days: week, locale: Locale(identifier: "de_DE"))?.weekday,
                       "Dienstag")
        XCTAssertEqual(InsightsDayPick.pick("2026-10-11", days: week)?.index, 6)
        // Another week's day, no day, or no days: the whole week.
        XCTAssertNil(InsightsDayPick.pick("2026-10-12", days: week))
        XCTAssertNil(InsightsDayPick.pick(nil, days: week))
        XCTAssertNil(InsightsDayPick.pick("2026-10-06", days: nil))
    }

    func testADayListsItsSessionsMostFirstAndLeavesOutTheRest() throws {
        let sessions = [
            Session(id: "a", days: [5, 0, 0, 0, 0, 0, 0]),
            Session(id: "b", days: [0, 3, 0, 0, 0, 0, 0]),
            Session(id: "codex", days: nil),
            Session(id: "c", days: [0, 9, 1, 0, 0, 0, 0]),
            Session(id: "d", days: [0, 3, 0, 0, 0, 0, 0]),
        ]
        let tuesday = try XCTUnwrap(InsightsDayBreakdown.rows(sessions, day: 1) { $0.days })
        XCTAssertEqual(tuesday.map(\.id), ["c", "b", "d"])
        XCTAssertEqual(tuesday.map(\.tokens), [9, 3, 3])
        XCTAssertEqual(InsightsDayBreakdown.rows(sessions, day: 5) { $0.days }?.count, 0)
    }

    func testRowsWithNoSplitAtAllLeaveTheWeekShowing() {
        let older = [Session(id: "a", days: nil), Session(id: "b", days: nil)]
        XCTAssertNil(InsightsDayBreakdown.rows(older, day: 1) { $0.days })
    }

    func testBothFeedsDecodeTheSplitAndAnOlderAnswerWithout() throws {
        let row: [String: Any] = ["source": "claude_code", "tokens": 7, "state": "known", "reasons": [String](),
                                  "by_day": [0, 3, 4, 0, 0, 0, 0]]
        var older = row
        older["by_day"] = nil
        let counter = try DaemonDataDecoding.decoder().decode(
            [DaemonData.InsightsWeekSession].self, from: JSONSerialization.data(withJSONObject: [row, older]))
        XCTAssertEqual(counter.map(InsightsDayBreakdown.split), [[0, 3, 4, 0, 0, 0, 0], nil])

        var saved = row
        saved["session_ref"] = "s"
        var savedOlder = older
        savedOlder["session_ref"] = "o"
        let card = try JSONDecoder().decode(
            [InsightsCardSession].self, from: JSONSerialization.data(withJSONObject: [saved, savedOlder]))
        XCTAssertEqual(card.map(\.by_day), [[0, 3, 4, 0, 0, 0, 0], nil])
    }

    /// The sample week is used in demos: its sessions sum to its chart day
    /// by day, and every day lists some.
    func testTheSampleSessionsSumToTheSampleDays() throws {
        let overview = try InsightsSampleWeek.weekOverview
        let byDay = try XCTUnwrap(overview.by_day)
        let sessions = try InsightsSampleWeek.counterSessions
        for (index, day) in byDay.enumerated() {
            let split = sessions.rows.compactMap { InsightsDayBreakdown.split($0.session)?[index] }.reduce(0, +)
            XCTAssertEqual(split, day.uncached + day.cache_read + day.cache_write + day.output, day.date)
            XCTAssertFalse(try XCTUnwrap(InsightsDayBreakdown.rows(sessions.rows, day: index) {
                InsightsDayBreakdown.split($0.session)
            }).isEmpty)
        }
        for row in sessions.rows {
            if let split = InsightsDayBreakdown.split(row.session) {
                XCTAssertEqual(row.session.tokens, Int64(split.reduce(0, +)))
            }
        }
        let states = Dictionary(grouping: sessions.rows, by: \.session.state).mapValues(\.count)
        XCTAssertEqual(states, ["known": Int(overview.coverage.known), "partial": Int(overview.coverage.partial),
                                "unknown": Int(overview.coverage.unknown)])
        let largest = sessions.rows.filter { $0.session.source == "claude_code" }.compactMap(\.session.tokens).max()
        XCTAssertEqual(largest.map(UInt64.init), overview.sources.first?.largest_session?.tokens)
        let codex = sessions.rows.filter { $0.session.source == "codex" }.compactMap(\.session.tokens)
        XCTAssertEqual(UInt64(codex.reduce(0, +)), overview.codex_interval_tokens)
    }

    func testTheKeyboardStepsThroughTheDaysAndPastEitherEndToTheWeek() {
        let ids = ["mon", "tue", "wed"]
        XCTAssertEqual(InsightsTokensByDay.step(ids, from: nil, by: 1), "mon")
        XCTAssertEqual(InsightsTokensByDay.step(ids, from: nil, by: -1), "wed")
        XCTAssertEqual(InsightsTokensByDay.step(ids, from: "mon", by: 1), "tue")
        XCTAssertNil(InsightsTokensByDay.step(ids, from: "wed", by: 1))
        XCTAssertNil(InsightsTokensByDay.step(ids, from: "mon", by: -1))
        XCTAssertNil(InsightsTokensByDay.step([], from: nil, by: 1))
    }

    func testADayOpensFoldedAndItsRowsSayThatDaysTokens() throws {
        let overview = try GlassSurfaceRulesTests.text("Views/InsightsOverviewTab.swift")
        XCTAssertEqual(overview.components(separatedBy: ".onChange(of: day) { _, _ in showAll = false }").count, 3)
        XCTAssertTrue(overview.contains("counterLabel(row.session, tokens: figure, copy: copy)"))
    }

    func testAPickedDaysTokensStandInTheCountedLabel() throws {
        let row: [String: Any] = ["source": "claude_code", "tokens": 7_000, "state": "known", "reasons": [String]()]
        let session = try DaemonDataDecoding.decoder().decode(
            DaemonData.InsightsWeekSession.self, from: JSONSerialization.data(withJSONObject: row))
        let copy = ["analytics_session_label_undated": "{harness} \u{b7} {t}"]
        let week = InsightsSessionsWords.counterLabel(session, copy: copy)
        let day = InsightsSessionsWords.counterLabel(session, tokens: 3_000, copy: copy)
        XCTAssertNotEqual(week, day)
        XCTAssertEqual(InsightsSessionsWords.counterLabel(session, tokens: nil, copy: copy), week)
    }

    /// Tokens by day moved to Details (product owner, 2026-10-28).
    func testAClickOnTheChartPicksTheDayAndASecondPutsTheWeekBack() throws {
        let overview = try GlassSurfaceRulesTests.text("Views/InsightsTokensByDay.swift")
        XCTAssertTrue(overview.contains("pick(pickedDay == day.id ? nil : day.id)"))
        let detail = try GlassSurfaceRulesTests.text("Views/InsightsBreakdownInspector.swift")
        XCTAssertTrue(detail.contains("InsightsDayPick.pick(inspector.pickedDay, days: overview?.shown?.by_day)"))
    }
}
