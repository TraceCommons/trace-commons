#if DEBUG
import XCTest
@testable import TCShellCore
@testable import TraceCommonsApp

/// Home in Ron's #1146 shape (Task 8 of the #1241 port): the status card
/// with its link to Traces, the stat cards with pending credit under D6,
/// Missions, and History with two recent rows.
final class HomeViewTests: XCTestCase {
    static func flat(_ rel: String) throws -> String {
        try HistoryParityTests.text(rel).split(whereSeparator: \.isWhitespace).joined(separator: " ")
    }

    /// Ron's "Open Traces": the status card links to the Traces tab, in the
    /// tab's own word, and the window switches the tab.
    func test_homeLinksToTraces() throws {
        let home = try Self.flat("Views/Monitor/HomeViews.swift")
        for needle in [
            "Button(action: openTraces) { HStack(spacing: GlassTokens.Space.inlineGap) "
                + "{ Text(MonitorWindowView.Tab.traces.title)",
            // The waiting line under the status, the Traces badge's own words.
            "MonitorWindowView.tracesDescription( traces.decisionsOwed, shield: traces.shield, "
                + "secondLook: traces.words?.secondLookWaiting)",
            "openHistory: { page = .history }, openMissions: { page = .missions }, openTraces: openTraces)",
        ] {
            XCTAssertTrue(home.contains(needle), "HomeViews.swift lacks \(needle)")
        }
        let window = try Self.flat("Views/MonitorWindowView.swift")
        XCTAssertTrue(window.contains("openTraces: { tab = .traces })"))
    }

    /// One waiting number on Home: the stat card and the status card's line
    /// both read the Traces store, the count the Traces badge shows.
    func test_homeReadsWaitingFromOneStore() throws {
        let home = try Self.flat("Views/Monitor/HomeViews.swift")
        XCTAssertTrue(home.contains(
            "GlassLegendCell(MonitorWords.waiting, value: HomeFormat.count(traces.decisionsOwed), status: .ask)"))
        XCTAssertFalse(home.contains("store.status?.decisionsOwed"), "Home reads waiting from a second store")
    }

    /// Ron's three stat cards; pending credit is a figure only beside the
    /// commons' statement of what it waits on (D6), a dash otherwise.
    func test_homeStatCardsKeepD6() throws {
        let home = try Self.flat("Views/Monitor/HomeViews.swift")
        XCTAssertTrue(home.contains(
            "GlassLegendCell(MonitorWords.pending, value: HomeFormat.pendingFigure(freshRollup?.creditPending, credit: freshCredit), status: .ask)"))
        // The credit summary after a failed read is no summary (D6 then
        // shows a dash and no condition), never the earlier sentence.
        XCTAssertTrue(home.contains("if let condition = HomeFormat.pendingCondition(freshCredit) { Text(condition)"))
        XCTAssertEqual(HomeFormat.pendingFigure(3, credit: nil), "—")
        XCTAssertEqual(HomeFormat.pendingFigure(nil, credit: nil), "—")
        let reply = try XCTUnwrap(SampleDaemonData.reply("commons_credit_summary", in: .normalDay))
        var object = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(reply.utf8)) as? [String: Any])
        object["posture_state"] = "known"
        object["commons_settlement"] = "pending_review"
        object["commons_settlement_explanation"] = "Scored when the commons reviews it."
        let known = try DaemonDataDecoding.decoder().decode(
            DaemonData.CommonsCreditSummary.self, from: JSONSerialization.data(withJSONObject: object))
        XCTAssertEqual(HomeFormat.pendingFigure(3, credit: known), HomeFormat.points(3))
        XCTAssertEqual(HomeFormat.pendingFigure(nil, credit: known), "—")
    }
}
#endif
