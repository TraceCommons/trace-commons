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
            // The Watching card is gone (owner, 2026-10-12); the Waiting
            // tile opens Traces.
            "static var openTracesWord: String { MonitorWords.table?.openTraces ?? \"\" }",
            "store: store, traces: traces, statusLabel: statusLabel, openTraces: openTraces, ",
            // Ron's Missions is the drafts; the catalogue and Insights sit
            // after his cards.
            // History lives on Traces (owner, 2026-10-12); Home's way in is
            // the Contributed tile.
            "openHistory: openHistory, openMissionDrafts: { page = .missionDrafts }, ",
            "openInsights: openInsights, openMissions: { page = .missions })",
        ] {
            XCTAssertTrue(home.contains(needle), "HomeViews.swift lacks \(needle)")
        }
        XCTAssertFalse(home.contains("Button(action: openTraces) { GlassCard(interactive: true) {"),
                       "the Watching card is back on Home")
        let window = try Self.flat("Views/MonitorWindowView.swift")
        XCTAssertTrue(window.contains("openTraces: { tab = .traces },"))
    }

    /// P1: #1146's status card puts the dot beside both lines, so the
    /// waiting line starts under the status words, never under the dot.
    /// Every status word sets the shared guide, and so does the waiting
    /// line; each history row keeps its own bottom rule (L14).
    func test_theStatusLinesShareALeftEdgeAfterTheDot() throws {
        let home = try HistoryParityTests.text("Views/Monitor/HomeViews.swift")
        // The status line is its own view, shared with the edge rail.
        let status = try XCTUnwrap(home.range(of: "struct HomeStatusLine: View {"))
        let statusEnd = try XCTUnwrap(home.range(of: "/// Rows inside a Home card", range: status.upperBound..<home.endIndex))
        let body = String(home[status.upperBound..<statusEnd.lowerBound])
        let texts = body.components(separatedBy: "\n").filter { $0.trimmingCharacters(in: .whitespaces).hasPrefix("Text(") }
        XCTAssertEqual(texts.count, 6)
        XCTAssertEqual(body.components(separatedBy: ".alignmentGuide(.homeStatusText) { $0[.leading] }").count - 1,
                       texts.count, "a status line starts at the dot")
        XCTAssertTrue(home.contains(".overlay(alignment: .bottom) {\n            GlassHairline(GlassColor.hairline)"),
                      "a history row lost its rule")
    }

    /// One waiting number on Home: the stat tile reads the Traces store,
    /// the count the Traces badge shows.
    func test_homeReadsWaitingFromOneStore() throws {
        let home = try Self.flat("Views/Monitor/HomeViews.swift")
        XCTAssertTrue(home.contains(
            "HomeStatTile(label: MonitorWords.waiting, value: HomeFormat.count(traces.decisionsOwed), "
                + "systemImage: HomeStatIcons.waiting, action: openTraces)"))
        XCTAssertFalse(home.contains("store.status?.decisionsOwed"), "Home reads waiting from a second store")
    }

    /// Ron's three stat cards; pending credit is a figure only beside the
    /// commons' statement of what it waits on (D6), a dash otherwise. Each
    /// opens where its number lives (owner, 2026-10-12).
    func test_homeStatCardsKeepD6() throws {
        let home = try Self.flat("Views/Monitor/HomeViews.swift")
        // Credit is the total with its change since the last day this Mac
        // saw (owner, 2026-10-12); the pending condition still sits under
        // the tiles.
        XCTAssertTrue(home.contains(
            "HomeStatTile(label: HomeFormat.creditPendingWord, "
                + "value: HomeFormat.creditTotal(HomeStore.totalCredit(credit: freshCredit, rollup: freshRollup)), "
                + "detail: delta?.text, detailInk: delta?.ink, systemImage: HomeStatIcons.credit, action: openPrivateAI)"))
        XCTAssertTrue(home.contains(
            "HomeStatTile(label: words?.uncontributed ?? \"\", value: String(uncontributed.count), "
                + "detail: uncontributed.detail(words), systemImage: HomeStatIcons.uncontributed, "
                + "valueAccessory: uncontributed.size(words), action: openTraces)"))
        XCTAssertTrue(home.contains(
            "HomeStatTile(label: MonitorWords.contributed, value: HomeFormat.count(freshRollup?.allTime?.accepted), "
                + "systemImage: HomeStatIcons.contributed, action: openHistory)"))
        // The credit summary after a failed read is no summary (D6 then
        // shows a dash). The condition gates the figure but is not drawn
        // (owner, 2026-10-13).
        XCTAssertTrue(home.contains("let condition = HomeFormat.pendingCondition(freshCredit)"))
        XCTAssertFalse(home.contains("Text(condition)"))
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
