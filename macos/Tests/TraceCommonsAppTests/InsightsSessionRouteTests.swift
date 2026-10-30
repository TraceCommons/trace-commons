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
        // No proxy ledger: the column goes and the core's line says no
        // Private AI calls are recorded yet (Ron's review of #1287).
        XCTAssertEqual(InsightsRouteWords.column(try unavailable("no_ledger")), .noLedger)
        // A reason this build does not know: the column goes and nothing is
        // said in the shell's own words.
        XCTAssertEqual(InsightsRouteWords.column(try unavailable("something_new")), .hidden)
    }

    func testTheFeedOffLineIsTheCoresAndNamesTheSwitch() throws {
        let copy = try Self.coreCopy()
        let line = try XCTUnwrap(InsightsRouteWords.unavailableLine(.feedOff, copy: copy))
        XCTAssertEqual(line, copy["analytics_route_feed_off"])
        let switchLabel = try XCTUnwrap(copy["analytics_setting_ledger_feed"])
        XCTAssertTrue(line.contains("\"\(switchLabel)\""),
                      "the line names Settings' switch: \(line)")
        XCTAssertNil(InsightsRouteWords.unavailableLine(.shown, copy: copy))
        XCTAssertNil(InsightsRouteWords.unavailableLine(.hidden, copy: copy))
    }

    func testNoLedgerShowsTheCoresNoCallsLine() throws {
        let copy = try Self.coreCopy()
        let line = try XCTUnwrap(InsightsRouteWords.unavailableLine(.noLedger, copy: copy))
        XCTAssertEqual(line, copy["analytics_route_no_ledger"])
        XCTAssertFalse(line.isEmpty)
        XCTAssertNotEqual(line, copy["analytics_route_feed_off"])
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

    /// A session the proxy saw none of (no record, or no route at all) draws
    /// no Private AI line: on a week of direct calls every row said "No
    /// record" (owner, 2026-10-29). Every other category still draws it.
    func testNoProxyRecordDrawsNoPrivateAILine() throws {
        XCTAssertFalse(InsightsRouteWords.drawsLine(nil))
        XCTAssertFalse(InsightsRouteWords.drawsLine(try Self.route("unobserved")))
        for category in ["unrecorded", "outside", "mixed", "check_failed", "routed_verified",
                         "routed_unverified", "from_the_future"] {
            XCTAssertTrue(InsightsRouteWords.drawsLine(try Self.route(category)), category)
        }
        let source = try InsightsGlassConventionsTests.text("Views/InsightsOverviewTab.swift")
        XCTAssertTrue(source.contains("if routed, InsightsRouteWords.drawsLine(row.session.routing) {"))
        XCTAssertTrue(source.contains("if routed, rows.contains(where: { InsightsRouteWords.drawsLine($0.session.routing) }) {"),
                      "the per-call note goes with the lines")
    }

    func testTheSplitLineIsInProxyTokensWithTheDashForUnknown() throws {
        let copy = try Self.coreCopy()
        let rows = try XCTUnwrap(Self.decode(Self.recorded()).rollup?.sessions)
        let mixed = try XCTUnwrap(rows.compactMap(\.routing).first { $0.category == "mixed" })
        let line = try XCTUnwrap(InsightsRouteWords.split(mixed, copy: copy))
        let template = try XCTUnwrap(copy["analytics_route_split"])
        XCTAssertEqual(line, InsightsOverviewWords.fill(template,
                                                         ["v": "110", "u": "0", "f": "0", "o": "\u{2014}", "n": "0"]))
        XCTAssertFalse(line.contains("{"), "every slot filled: \(line)")
    }

    func testTheSplitIsShownWhateverTheCheckFailedTokens() throws {
        // The split carries all five buckets (Ron's review of #1287), so a
        // session whose failed-check calls hold tokens still draws it, and
        // an unknown number of them reads as the dash, never as zero.
        let copy = try Self.coreCopy()
        let template = try XCTUnwrap(copy["analytics_route_split"])
        let fill = { (f: String) in
            InsightsOverviewWords.fill(template, ["v": "110", "u": "0", "f": f, "o": "\u{2014}", "n": "0"])
        }
        XCTAssertEqual(InsightsRouteWords.split(try Self.route("check_failed", checkFailed: 40), copy: copy),
                       fill("40"))
        XCTAssertEqual(InsightsRouteWords.split(try Self.route("check_failed", checkFailed: NSNull()), copy: copy),
                       fill("\u{2014}"))
        XCTAssertEqual(InsightsRouteWords.split(try Self.route("routed_verified", checkFailed: 0), copy: copy),
                       fill("0"))
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

    func testARowWithoutLastActivityIsLabelledFromItsFirstEventAndHarness() throws {
        let copy = try Self.coreCopy()
        let utc = try XCTUnwrap(TimeZone(secondsFromGMT: 0))
        var object = try Self.recorded()
        var rollup = try XCTUnwrap(object["rollup"] as? [String: Any])
        var recorded = try XCTUnwrap(rollup["sessions"] as? [[String: Any]])
        for index in recorded.indices { recorded[index].removeValue(forKey: "last_event_at") }
        rollup["sessions"] = recorded
        object["rollup"] = rollup
        let rows = try XCTUnwrap(Self.decode(object).rollup?.sessions)
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

    func testBreakdownUsesFolderAndSessionReference() throws {
        let copy = try Self.coreCopy()
        var object = try Self.recorded()
        var rollup = try XCTUnwrap(object["rollup"] as? [String: Any])
        var rows = try XCTUnwrap(rollup["sessions"] as? [[String: Any]])
        rows[0]["folder"] = "trace-commons-server"
        rows[0]["session_ref"] = "t42"
        rows[1]["folder"] = "trace-commons-server"
        rows[1]["session_ref"] = "t43"
        rollup["sessions"] = rows
        object["rollup"] = rollup
        let sessions = try XCTUnwrap(Self.decode(object).rollup?.sessions)
        XCTAssertEqual(InsightsSessionsWords.counterTitle(sessions[0], copy: copy),
                       "trace-commons-server · t42")
        XCTAssertEqual(InsightsSessionsWords.counterTitle(sessions[1], copy: copy),
                       "trace-commons-server · t43")
        XCTAssertEqual(sessions[0].sessionRef, "t42")
    }

    func testOlderBreakdownPayloadUsesUnknownFolderWithoutInventingAnIdentifier() throws {
        let copy = try Self.coreCopy()
        var object = try Self.recorded()
        var rollup = try XCTUnwrap(object["rollup"] as? [String: Any])
        var rows = try XCTUnwrap(rollup["sessions"] as? [[String: Any]])
        for index in rows.indices {
            rows[index].removeValue(forKey: "folder")
            rows[index].removeValue(forKey: "session_ref")
        }
        rollup["sessions"] = rows
        object["rollup"] = rollup
        let session = try XCTUnwrap(Self.decode(object).rollup?.sessions.first)
        XCTAssertEqual(InsightsSessionsWords.counterTitle(session, copy: copy),
                       try XCTUnwrap(copy["analytics_folder_unknown"]))
        XCTAssertNil(session.sessionRef)
    }

    func testBreakdownShowsLastRecordedActivityInItsTimeRange() throws {
        let copy = try Self.coreCopy()
        _ = try XCTUnwrap(copy["analytics_session_label_range"])
        let utc = try XCTUnwrap(TimeZone(secondsFromGMT: 0))
        var object = try Self.recorded()
        var rollup = try XCTUnwrap(object["rollup"] as? [String: Any])
        var rows = try XCTUnwrap(rollup["sessions"] as? [[String: Any]])
        rows[0]["started_at"] = "2026-10-22T13:53:00Z"
        for end in ["2026-10-22T15:05:00Z", "2026-10-23T01:05:00Z"] {
            rows[0]["last_event_at"] = end
            rollup["sessions"] = rows
            object["rollup"] = rollup
            let session = try XCTUnwrap(Self.decode(object).rollup?.sessions.first)
            let label = InsightsSessionsWords.counterLabel(session, copy: copy, timeZone: utc)
            let last = try Date(end, strategy: .iso8601)
            XCTAssertTrue(label.contains(last.formatted(Date.FormatStyle(timeZone: utc).hour().minute())),
                          "Last recorded activity must distinguish sessions: \(label)")
            if end.contains("10-23") {
                XCTAssertTrue(label.contains(last.formatted(Date.FormatStyle(timeZone: utc)
                    .month(.abbreviated).day())), "An overnight session must show the end date")
            }
        }
    }

    func testBreakdownTimeRangeUsesEachEventsLocalOffsetAcrossDaylightSaving() throws {
        let copy = try Self.coreCopy()
        let zone = try XCTUnwrap(TimeZone(identifier: "America/Los_Angeles"))
        var object = try Self.recorded()
        var rollup = try XCTUnwrap(object["rollup"] as? [String: Any])
        var rows = try XCTUnwrap(rollup["sessions"] as? [[String: Any]])
        rows[0]["started_at"] = "2026-11-01T08:30:00Z"
        rows[0]["last_event_at"] = "2026-11-01T10:30:00Z"
        rollup["sessions"] = rows
        object["rollup"] = rollup
        let row = try XCTUnwrap(Self.decode(object).rollup?.sessions.first)
        let last = try Date("2026-11-01T10:30:00Z", strategy: .iso8601)
        let label = InsightsSessionsWords.counterLabel(row, copy: copy, timeZone: zone)
        XCTAssertTrue(label.contains(last.formatted(Date.FormatStyle(timeZone: zone).hour().minute())),
                      "End time must use its own daylight-saving offset: \(label)")
    }

    func testBreakdownDoesNotInventAnEndTimeForOlderOrInvalidPayloads() throws {
        let copy = try Self.coreCopy()
        let utc = try XCTUnwrap(TimeZone(secondsFromGMT: 0))
        var object = try Self.recorded()
        var rollup = try XCTUnwrap(object["rollup"] as? [String: Any])
        var rows = try XCTUnwrap(rollup["sessions"] as? [[String: Any]])
        rows[0]["started_at"] = "2026-10-22T13:53:00Z"
        rows[0].removeValue(forKey: "last_event_at")
        rollup["sessions"] = rows
        object["rollup"] = rollup
        let older = try XCTUnwrap(Self.decode(object).rollup?.sessions.first)
        let expected = InsightsSessionsWords.counterLabel(older, copy: copy, timeZone: utc)
        for end in ["invalid", "2026-10-21T12:00:00Z"] {
            rows[0]["last_event_at"] = end
            rollup["sessions"] = rows
            object["rollup"] = rollup
            let session = try XCTUnwrap(Self.decode(object).rollup?.sessions.first)
            XCTAssertEqual(InsightsSessionsWords.counterLabel(session, copy: copy, timeZone: utc), expected)
        }
    }

    // MARK: The model

    /// Under feed T the Sessions figure's breakdown is the daemon's counted
    /// sessions, always there while the week shows (owner, 2026-10-22: no
    /// chevron), and the core is never asked; no figure drills.
    @MainActor
    func testUnderFeedTTheSessionsBreakdownIsTheDaemonsAndAsksNobody() async throws {
        let calls = CallCounter()
        let model = InsightsOverviewModel(service: { _ in
            await calls.bump(); throw InsightsError.invalidResponse
        })
        model.open()
        for _ in 0..<500 where model.busy { try await Task.sleep(for: .milliseconds(10)) }
        let before = await calls.count
        let week = try Self.decode(Self.recorded())
        model.showCounter(week.coreOverview, sessions: InsightsCounterSessions(week: week))

        for card in ["sessions", "tokens", "cache_share"] {
            XCTAssertFalse(model.drills(card), "feed T rows carry no reference for the core's drill: \(card)")
        }
        let breakdown = try XCTUnwrap(model.counterSessions)
        XCTAssertEqual(breakdown.rows.count, 2)
        XCTAssertEqual(breakdown.column, .shown)

        model.showInputs("tokens")
        model.showInputs("sessions")
        XCTAssertNil(model.openCard)
        let after = await calls.count
        XCTAssertEqual(after, before, "feed T's rows are the daemon's; the core is not asked")

        model.showCounter(nil)
        XCTAssertNil(model.counterSessions, "going back to the saved week drops feed T's rows")
        model.close()
    }

    /// Under feed S the tokens and cache share figures still drill; the
    /// Sessions figure does not, since its breakdown is always shown.
    @MainActor
    func testUnderFeedSTheFiguresStillDrill() {
        let model = InsightsOverviewModel(service: { _ in throw InsightsError.invalidResponse })
        for card in ["tokens", "cache_share"] { XCTAssertTrue(model.drills(card), card) }
        XCTAssertFalse(model.drills("sessions"))
    }

    @MainActor
    func testAClosedOverviewOpensNoDrill() throws {
        // Nothing opens while the tab is closed.
        let model = InsightsOverviewModel(service: { _ in throw InsightsError.invalidResponse })
        model.showInputs("tokens")
        XCTAssertNil(model.openCard)
        XCTAssertNil(model.sessionsInputs)
    }

    @MainActor
    func testAFeedTWeekWithoutRowsHasNoBreakdown() throws {
        let model = InsightsOverviewModel(service: { _ in throw InsightsError.invalidResponse })
        let week = try Self.decode(Self.recorded())
        model.showCounter(week.coreOverview, sessions: nil)
        XCTAssertNil(model.counterSessions)
        XCTAssertFalse(model.drills("sessions"))
    }

    // MARK: The view's source

    func testTheOverviewDrawsTheColumnFromTheCoresWords() throws {
        let source = try InsightsGlassConventionsTests.text("Views/InsightsOverviewTab.swift")
        XCTAssertFalse(source.contains(".disabled(model.counter != nil)"))
        // A figure drills only when `drills` says so; one that does not is
        // drawn plain, not as a disabled button.
        XCTAssertTrue(source.contains("if model.drills(inputs) {"))
        // A picked day's sessions are drawn in the window's inspector
        // (owner, 2026-10-29).
        XCTAssertTrue(try InsightsGlassConventionsTests.text("Views/InsightsBreakdownInspector.swift")
            .contains("InsightsCounterSessionsView("))
        for key in ["analytics_drill_private", "analytics_route_measure_note"] {
            XCTAssertTrue(source.contains("\"\(key)\""), key)
        }
        XCTAssertTrue(source.contains("InsightsRouteWords.unavailableLine("))
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
