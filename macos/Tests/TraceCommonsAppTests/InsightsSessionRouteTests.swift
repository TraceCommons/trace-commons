import Foundation
import XCTest
import TCBridge
import TCShellCore
@testable import TraceCommonsApp

/// Feed T's Sessions drill-down with where each session's calls went
/// (design part B). Every routed shape here is the daemon's own recorded
/// answer, `tests/fixtures/insights-analytics/insights_week_routed.json`,
/// which the contributor crate's recorder test compares to its live answer;
/// the other shapes are that answer with fields taken away, the way a
/// switched-off feed or an older daemon sends it.
final class InsightsSessionRouteTests: XCTestCase {
    static let fixture = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent()   // TraceCommonsAppTests
        .deletingLastPathComponent()   // Tests
        .deletingLastPathComponent()   // macos
        .deletingLastPathComponent()   // <repo root>
        .appendingPathComponent("tests/fixtures/insights-analytics/insights_week_routed.json")

    static func recorded() throws -> [String: Any] {
        let object = try JSONSerialization.jsonObject(with: Data(contentsOf: fixture))
        return try XCTUnwrap(object as? [String: Any])
    }

    static func decode(_ object: [String: Any]) throws -> DaemonData.InsightsWeek {
        let data = try JSONSerialization.data(withJSONObject: object)
        return try DaemonDataDecoding.decoder().decode(DaemonData.InsightsWeek.self, from: data)
    }

    /// The recorded answer with each row's `routing` rewritten by `edit`.
    static func rerouted(_ edit: ([String: Any]) -> [String: Any]?) throws -> DaemonData.InsightsWeek {
        var object = try recorded()
        var rollup = try XCTUnwrap(object["rollup"] as? [String: Any])
        let rows = try XCTUnwrap(rollup["sessions"] as? [[String: Any]])
        rollup["sessions"] = rows.map { row -> [String: Any] in
            var row = row
            row["routing"] = (row["routing"] as? [String: Any]).flatMap(edit) ?? NSNull()
            return row
        }
        object["rollup"] = rollup
        return try decode(object)
    }

    /// The recorded `mixed` row's routing with `category` and the
    /// check-failed tokens replaced.
    static func route(_ category: String, checkFailed: Any = 0) throws -> DaemonData.InsightsWeekRouting {
        let week = try rerouted { routing in
            guard routing["category"] as? String == "mixed" else { return routing }
            var routing = routing
            routing["category"] = category
            var tokens = routing["tokens"] as? [String: Any] ?? [:]
            tokens["check_failed"] = checkFailed
            routing["tokens"] = tokens
            return routing
        }
        let rows = try XCTUnwrap(week.rollup?.sessions)
        return try XCTUnwrap(rows.compactMap(\.routing).first { $0.category == category })
    }

    static func coreCopy() throws -> [String: String] { try XCTUnwrap(TCInsights.copy()) }

    // MARK: Decode

    func testTheRecordedRoutedAnswerDecodes() throws {
        let week = try Self.decode(Self.recorded())
        XCTAssertTrue(week.showsCounterPass)
        XCTAssertEqual(week.routingAvailable, true)
        XCTAssertNil(week.routingUnavailable)
        let rows = try XCTUnwrap(week.rollup?.sessions)
        XCTAssertEqual(rows.count, 2)

        let claude = try XCTUnwrap(rows.first { $0.source == "claude_code" })
        XCTAssertEqual(claude.startedAt, "2026-09-14T09:00:00+00:00")
        XCTAssertEqual(claude.tokens, 37235)
        let none = try XCTUnwrap(claude.routing)
        XCTAssertEqual(none.category, "unobserved")
        XCTAssertEqual(none.reasons, [])
        XCTAssertNil(none.tokens, "no proxy record is not zero tokens")
        XCTAssertNil(none.calls)
        XCTAssertNil(none.callsWithoutCounts)

        let codex = try XCTUnwrap(rows.first { $0.source == "codex" })
        let mixed = try XCTUnwrap(codex.routing)
        XCTAssertEqual(mixed.category, "mixed")
        XCTAssertEqual(mixed.tokens?.verified, 110)
        XCTAssertEqual(mixed.tokens?.routedUnverified, 0)
        XCTAssertEqual(mixed.tokens?.checkFailed, 0)
        XCTAssertNil(mixed.tokens?.outside, "a bucket with an uncounted call is unknown, never 0")
        XCTAssertEqual(mixed.tokens?.unrecorded, 0)
        XCTAssertEqual(mixed.calls, 2)
        XCTAssertEqual(mixed.callsWithoutCounts, 1)
    }

    func testAnOlderDaemonsAnswerDecodesWithoutTheRouteFields() throws {
        // The feed T fixture written before part B carries none of them.
        let week = try InsightsWeekFeedTests.week(InsightsWeekFeedTests.counted)
        XCTAssertNil(week.routingAvailable)
        XCTAssertNil(week.routingUnavailable)
        let row = try XCTUnwrap(week.rollup?.sessions.first)
        XCTAssertNil(row.startedAt)
        XCTAssertNil(row.routing)
        XCTAssertEqual(InsightsRouteWords.column(week), .hidden)

        // And the recorded answer with the four fields removed.
        var object = try Self.recorded()
        object.removeValue(forKey: "routing_available")
        object.removeValue(forKey: "routing_unavailable")
        var rollup = try XCTUnwrap(object["rollup"] as? [String: Any])
        rollup["sessions"] = try XCTUnwrap(rollup["sessions"] as? [[String: Any]]).map { row -> [String: Any] in
            var row = row
            row.removeValue(forKey: "routing"); row.removeValue(forKey: "started_at")
            return row
        }
        object["rollup"] = rollup
        let older = try Self.decode(object)
        XCTAssertTrue(older.showsCounterPass)
        XCTAssertNil(older.routingAvailable)
        XCTAssertTrue(older.rollup?.sessions.allSatisfy { $0.routing == nil && $0.startedAt == nil } ?? false)
        XCTAssertEqual(InsightsRouteWords.column(older), .hidden)
    }

    // MARK: The column

    func testTheColumnShowsOnlyWhenTheDaemonSaysRoutingIsAvailable() throws {
        XCTAssertEqual(InsightsRouteWords.column(try Self.decode(Self.recorded())), .shown)

        func unavailable(_ reason: String) throws -> DaemonData.InsightsWeek {
            var object = try Self.recorded()
            object["routing_available"] = false
            object["routing_unavailable"] = reason
            return try Self.decode(object)
        }
        // Turned off in Settings: the column goes and one line says why.
        XCTAssertEqual(InsightsRouteWords.column(try unavailable("ledger_feed_off")), .feedOff)
        // No proxy ledger, or a reason this build does not know: the column
        // goes and nothing is said in the shell's own words.
        XCTAssertEqual(InsightsRouteWords.column(try unavailable("no_ledger")), .hidden)
        XCTAssertEqual(InsightsRouteWords.column(try unavailable("something_new")), .hidden)
    }

    func testTheFeedOffLineIsTheCoresAndNamesTheSwitch() throws {
        let copy = try Self.coreCopy()
        let line = try XCTUnwrap(InsightsRouteWords.feedOffLine(.feedOff, copy: copy))
        XCTAssertEqual(line, copy["analytics_route_feed_off"])
        let switchLabel = try XCTUnwrap(copy["analytics_setting_ledger_feed"])
        XCTAssertTrue(line.lowercased().contains(switchLabel.lowercased().replacingOccurrences(
            of: "count tokens", with: "counting tokens")), "the line names Settings' switch")
        XCTAssertNil(InsightsRouteWords.feedOffLine(.shown, copy: copy))
        XCTAssertNil(InsightsRouteWords.feedOffLine(.hidden, copy: copy))
    }

    // MARK: Words

    func testEachCategoryReadsItsOwnWord() throws {
        let copy = try Self.coreCopy()
        let expected = [
            "unobserved": "analytics_route_unobserved",
            "unrecorded": "analytics_route_unrecorded",
            "outside": "analytics_route_outside",
            "mixed": "analytics_route_mixed",
            "check_failed": "analytics_route_check_failed",
            "routed_verified": "analytics_route_verified",
            "routed_unverified": "analytics_route_unverified",
        ]
        XCTAssertEqual(expected.count, 7)
        var seen = Set<String>()
        for (category, key) in expected {
            let word = InsightsRouteWords.category(try Self.route(category), copy: copy)
            XCTAssertEqual(word, copy[key], category)
            XCTAssertFalse(word.isEmpty, category)
            seen.insert(word)
        }
        XCTAssertEqual(seen.count, 7, "every category reads differently")
        // Only verified proof says it, and nothing else claims it.
        for (category, _) in expected where category != "routed_verified" {
            XCTAssertNotEqual(InsightsRouteWords.category(try Self.route(category), copy: copy),
                              copy["analytics_route_verified"], category)
        }
        // A category this build does not know reads the dash, never a claim.
        XCTAssertEqual(InsightsRouteWords.category(try Self.route("from_the_future"), copy: copy), "\u{2014}")
        XCTAssertEqual(InsightsRouteWords.category(nil, copy: copy), "\u{2014}")
    }

    func testNoProxyRecordNeverReadsAsNotPrivate() throws {
        let copy = try Self.coreCopy()
        let rows = try XCTUnwrap(Self.decode(Self.recorded()).rollup?.sessions)
        let unobserved = try XCTUnwrap(rows.compactMap(\.routing).first { $0.category == "unobserved" })
        let word = InsightsRouteWords.category(unobserved, copy: copy)
        XCTAssertEqual(word, copy["analytics_route_unobserved"])
        for claim in ["not private", "not through", "outside", "verified"] {
            XCTAssertFalse(word.lowercased().contains(claim), claim)
        }
        XCTAssertNil(InsightsRouteWords.split(unobserved, copy: copy), "no record has no figures to split")
    }

    func testTheSplitLineIsInProxyTokensWithTheDashForUnknown() throws {
        let copy = try Self.coreCopy()
        let rows = try XCTUnwrap(Self.decode(Self.recorded()).rollup?.sessions)
        let mixed = try XCTUnwrap(rows.compactMap(\.routing).first { $0.category == "mixed" })
        let line = try XCTUnwrap(InsightsRouteWords.split(mixed, copy: copy))
        let template = try XCTUnwrap(copy["analytics_route_split"])
        XCTAssertEqual(line, InsightsOverviewWords.fill(template, ["v": "110", "u": "0", "o": "\u{2014}", "n": "0"]))
        XCTAssertTrue(line.hasSuffix("proxy tokens"))
    }

    func testTheSplitIsLeftOutWhileCheckFailedTokensHaveNoPlaceInIt() throws {
        // The split carries four of the five buckets; a session whose
        // failed-check calls hold tokens, or an unknown number of them,
        // draws no split rather than one that reads as nothing there.
        let copy = try Self.coreCopy()
        XCTAssertNil(InsightsRouteWords.split(try Self.route("check_failed", checkFailed: 40), copy: copy))
        XCTAssertNil(InsightsRouteWords.split(try Self.route("check_failed", checkFailed: NSNull()), copy: copy))
        XCTAssertNotNil(InsightsRouteWords.split(try Self.route("routed_verified", checkFailed: 0), copy: copy))
    }

    func testTheSomeCallsUnrecordedReasonHasItsWords() throws {
        let copy = try Self.coreCopy()
        let route = try Self.rerouted { routing in
            var routing = routing
            routing["reasons"] = ["some_calls_unrecorded", "from_the_future"]
            return routing
        }
        let routing = try XCTUnwrap(route.rollup?.sessions.compactMap(\.routing).first { $0.category == "mixed" })
        XCTAssertEqual(InsightsRouteWords.reasons(routing, copy: copy),
                       [try XCTUnwrap(copy["analytics_reason_some_calls_unrecorded"])],
                       "an unknown reason is left out, never shown as its wire label")
    }

    func testARowIsLabelledFromItsFirstEventAndHarness() throws {
        let copy = try Self.coreCopy()
        let utc = try XCTUnwrap(TimeZone(secondsFromGMT: 0))
        let rows = try XCTUnwrap(Self.decode(Self.recorded()).rollup?.sessions)
        let claude = try XCTUnwrap(rows.first { $0.source == "claude_code" })
        let label = InsightsSessionsWords.counterLabel(claude, copy: copy, timeZone: utc)
        let first = try XCTUnwrap(DateComponents(calendar: Calendar(identifier: .gregorian), timeZone: utc,
                                                 year: 2026, month: 9, day: 14, hour: 9).date)
        XCTAssertEqual(label, InsightsOverviewWords.fill(try XCTUnwrap(copy["analytics_session_label"]), [
            "date": first.formatted(Date.FormatStyle(timeZone: utc).month(.abbreviated).day()),
            "time": first.formatted(Date.FormatStyle(timeZone: utc).hour().minute()),
            "harness": InsightsOverviewWords.harness("claude_code", copy: copy),
            "t": InsightsOverviewWords.figure(37235, copy: copy),
        ]))
        // No first event: the undated label, never a guessed date.
        let older = try XCTUnwrap(InsightsWeekFeedTests.week(InsightsWeekFeedTests.counted).rollup?.sessions.first)
        XCTAssertEqual(InsightsSessionsWords.counterLabel(older, copy: copy, timeZone: utc),
                       InsightsOverviewWords.fill(try XCTUnwrap(copy["analytics_session_label_undated"]), [
                           "harness": InsightsOverviewWords.harness("claude_code", copy: copy),
                           "t": InsightsOverviewWords.figure(37235, copy: copy),
                       ]))
    }

    // MARK: The model

    @MainActor
    func testUnderFeedTOnlyTheSessionsCardDrillsAndItAsksNobody() async throws {
        let calls = CallCounter()
        let model = InsightsOverviewModel(service: { _ in
            await calls.bump(); throw InsightsError.invalidResponse
        })
        model.open()
        for _ in 0..<500 where model.busy { try await Task.sleep(for: .milliseconds(10)) }
        let before = await calls.count
        let week = try Self.decode(Self.recorded())
        model.showCounter(week.coreOverview, sessions: InsightsCounterSessions(week: week))

        XCTAssertTrue(model.drills("sessions"))
        XCTAssertFalse(model.drills("tokens"), "feed T rows carry no reference for the core's drill")
        XCTAssertFalse(model.drills("cache_share"))

        model.showInputs("tokens")
        XCTAssertNil(model.counterInputs)
        XCTAssertNil(model.openCard)
        model.showInputs("sessions")
        let open = try XCTUnwrap(model.counterInputs)
        XCTAssertEqual(open.rows.count, 2)
        XCTAssertEqual(open.column, .shown)
        XCTAssertEqual(model.openCard, "sessions")
        let after = await calls.count
        XCTAssertEqual(after, before, "feed T's rows are the daemon's; the core is not asked")

        model.hideInputs()
        XCTAssertNil(model.counterInputs)
        model.showInputs("sessions")
        model.showCounter(nil)
        XCTAssertNil(model.counterInputs, "going back to the saved week closes feed T's drill")
        XCTAssertFalse(model.drills("sessions") && model.counter != nil)
        model.close()
    }

    @MainActor
    func testUnderFeedSEveryCardStillDrills() {
        let model = InsightsOverviewModel(service: { _ in throw InsightsError.invalidResponse })
        for card in ["tokens", "cache_share", "sessions"] { XCTAssertTrue(model.drills(card), card) }
    }

    @MainActor
    func testAClosedOverviewOpensNoFeedTDrill() throws {
        // As under the saved week: nothing opens while the tab is closed.
        let model = InsightsOverviewModel(service: { _ in throw InsightsError.invalidResponse })
        let week = try Self.decode(Self.recorded())
        model.showCounter(week.coreOverview, sessions: InsightsCounterSessions(week: week))
        model.showInputs("sessions")
        XCTAssertNil(model.counterInputs)
        XCTAssertNil(model.openCard)
    }

    @MainActor
    func testAFeedTWeekWithoutRowsDoesNotDrill() throws {
        let model = InsightsOverviewModel(service: { _ in throw InsightsError.invalidResponse })
        let week = try Self.decode(Self.recorded())
        model.showCounter(week.coreOverview, sessions: nil)
        XCTAssertFalse(model.drills("sessions"))
    }

    // MARK: The view's source

    func testTheOverviewDrawsTheColumnFromTheCoresWords() throws {
        let source = try InsightsGlassConventionsTests.text("Views/InsightsOverviewTab.swift")
        XCTAssertFalse(source.contains(".disabled(model.counter != nil)"))
        XCTAssertTrue(source.contains(".disabled(!model.drills(inputs))"))
        XCTAssertTrue(source.contains("InsightsCounterSessionsView("))
        for key in ["analytics_drill_private", "analytics_route_measure_note"] {
            XCTAssertTrue(source.contains("\"\(key)\""), key)
        }
        XCTAssertTrue(source.contains("InsightsRouteWords.feedOffLine("))
        XCTAssertTrue(source.contains("InsightsRouteWords.category("))
        XCTAssertTrue(source.contains("InsightsRouteWords.split("))
        // The two measures are drawn side by side, never added together.
        XCTAssertFalse(source.contains("routing?.tokens?.verified ?? 0) +"))
        XCTAssertFalse(source.contains(".tokens ?? 0) +"))
    }

    func testTheWindowPassesFeedTsRowsToTheOverview() throws {
        let source = try InsightsGlassConventionsTests.text("Views/InsightsView.swift")
        XCTAssertTrue(source.contains(
            "overviewModel.showCounter(week?.coreOverview, sessions: week.flatMap(InsightsCounterSessions.init(week:)))"))
    }
}

private actor CallCounter {
    private(set) var count = 0
    func bump() { count += 1 }
}
