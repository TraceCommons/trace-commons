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
    /// core's words (Ron's status card, which has no waiting line of its
    /// own since the merge of Ron's glass rebuilds), and the window
    /// switches the tab.
    func test_homeLinksToTraces() throws {
        let home = try Self.flat("Views/Monitor/HomeViews.swift")
        for needle in [
            "Button(action: openTraces) { HStack(spacing: GlassTokens.Space.s1) { Text(HomeFormat.openTracesWord)",
            "static var openTracesWord: String { MonitorWords.table?.openTraces ?? \"\" }",
            "store: store, traces: traces, statusLabel: statusLabel, openTraces: openTraces, ",
            // Ron's Missions is the drafts; the catalogue and Insights sit
            // after his cards.
            "openHistory: { page = .history }, openMissionDrafts: { page = .missionDrafts }, ",
            "openInsights: { page = .insights }, openMissions: { page = .missions })",
        ] {
            XCTAssertTrue(home.contains(needle), "HomeViews.swift lacks \(needle)")
        }
        let window = try Self.flat("Views/MonitorWindowView.swift")
        XCTAssertTrue(window.contains("openTraces: { tab = .traces },"))
    }

    /// One waiting number on Home: the stat tile reads the Traces store,
    /// the count the Traces badge shows.
    func test_homeReadsWaitingFromOneStore() throws {
        let home = try Self.flat("Views/Monitor/HomeViews.swift")
        XCTAssertTrue(home.contains(
            "HomeStatTile(label: MonitorWords.waiting, value: HomeFormat.count(traces.decisionsOwed))"))
        XCTAssertFalse(home.contains("store.status?.decisionsOwed"), "Home reads waiting from a second store")
    }

    /// Ron's three stat cards; pending credit is a figure only beside the
    /// commons' statement of what it waits on (D6), a dash otherwise.
    func test_homeStatCardsKeepD6() throws {
        let home = try Self.flat("Views/Monitor/HomeViews.swift")
        XCTAssertTrue(home.contains(
            "HomeStatTile(label: HomeFormat.creditPendingWord, "
                + "value: HomeFormat.pendingFigure(freshRollup?.creditPending, condition: condition))"))
        XCTAssertTrue(home.contains(
            "HomeStatTile(label: MonitorWords.contributed, value: HomeFormat.count(freshRollup?.allTime?.accepted))"))
        // The credit summary after a failed read is no summary (D6 then
        // shows a dash and no condition), never the earlier sentence.
        XCTAssertTrue(home.contains("let condition = HomeFormat.pendingCondition(freshCredit)"))
        XCTAssertTrue(home.contains("if let condition { Text(condition)"))
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
