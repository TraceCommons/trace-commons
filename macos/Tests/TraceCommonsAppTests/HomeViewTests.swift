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

    func testCampaignKeepsNativeCardAndOpensItsValidatedURL() throws {
        let home = try HistoryParityTests.text("Views/Monitor/HomeViews.swift")
        let start = try XCTUnwrap(home.range(of: "    private var campaign: some View {"))
        let end = try XCTUnwrap(home.range(of: "\n    var body: some View {", range: start.upperBound..<home.endIndex))
        let campaign = home[start.lowerBound..<end.lowerBound]
        XCTAssertTrue(home.contains("@State private var campaignStore = PrivateAICampaignStore()"))
        XCTAssertTrue(home.contains(".task { await campaignStore.refresh() }"))
        XCTAssertTrue(campaign.contains("GlassHeroCard("))
        XCTAssertTrue(campaign.contains("openURL(campaign.actionURL)"))
        XCTAssertFalse(campaign.contains("action: openPrivateAI"))
        XCTAssertFalse(campaign.contains("words.campaignTitle"))
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
            "openInsights: openInsights, openMissions: { page = .missions }, inference: inference)",
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
        // Credit is History's figures (owner, 2026-10-28): Final once any
        // credit is decided, else the pending figure under the same D6 gate.
        XCTAssertTrue(home.contains(
            "let creditTile = HomeFormat.creditTile(final: freshRollup?.creditFinal, "
                + "pending: freshRollup?.creditPending, accepted: freshRollup?.allTime?.accepted, "
                + "credit: freshCredit)"))
        XCTAssertTrue(home.contains(
            "HomeStatTile(label: HomeFormat.creditPendingWord, value: creditTile.value, "
                + "detail: signIn ?? creditTile.detail, "
                + "systemImage: HomeStatIcons.credit, action: openPrivateAI)"))
        // What is not contributed is an upsell two tiles tall across both
        // columns, under Waiting and Contributed (owner, 2026-10-19), and
        // the NEAR AI balance sits beside Credit.
        XCTAssertTrue(home.contains(
            "if uncontributed.count > 0, let title = HomeFormat.cardHeading(words?.uncontributed) { "
                + "GridRow { notContributed(uncontributed, title: title) .gridCellColumns(2) } }"))
        // Slim, on the standard card fill under a solid accent border
        // (owner, 2026-10-20: it had no fill and a turning glow edge).
        XCTAssertTrue(home.contains("detail: uncontributed.detail(words), action: openTraces)"))
        XCTAssertTrue(home.contains(".glassTier(.card, hover: GlassTokens.Color.cardHover)"))
        // No border; a light turns round its edge instead (owner,
        // 2026-10-22; a shimmer crossed it from 2026-10-20).
        XCTAssertTrue(home.contains(".glassCardBeacon()"))
        XCTAssertFalse(home.contains("glassCardShimmer"), "the shimmer is back")
        XCTAssertFalse(home.contains("strokeBorder(Self.border"), "the border is back")
        XCTAssertFalse(home.contains(".overlay(GlassGlowEdge("), "the glow edge is back")
        // Its icon is the card's watermark, as the tiles' (owner, 2026-10-20).
        XCTAssertTrue(home.contains(".glassCardWatermark(HomeStatIcons.uncontributed)"))
        let card = try XCTUnwrap(home.range(of: "private struct HomeNotContributedCard: View {"))
        XCTAssertFalse(home[card.upperBound...].prefix(2500).contains("Gradient.heroMissions"), "the dark gradient is back")
        // The figure counts while credit arrives (`BalanceArrivalFlourish`),
        // and the 24-hour change sits under it in its ink (owner, 2026-10-23).
        XCTAssertTrue(home.contains(
            "HomeStatTile(label: copy.balanceTitle, value: counting ?? balance.value, "
                + "detail: balance.detail, detailInk: balance.detailInk, "
                + "supplementaryDetail: HomeFormat.balanceReceived(model.balanceStatus, calls: model.balanceCalls), "
                + "systemImage: HomeStatIcons.balance, valueAccessory: balance.beside, "
                + "action: openPrivateAI)"))
        let waiting = try XCTUnwrap(home.range(of: "HomeStatTile(label: MonitorWords.waiting"))
        let upsell = try XCTUnwrap(home.range(of: "notContributed(uncontributed, title: title)"))
        let credit = try XCTUnwrap(home.range(of: "HomeStatTile(label: HomeFormat.creditPendingWord"))
        XCTAssertLessThan(waiting.lowerBound, upsell.lowerBound)
        XCTAssertLessThan(upsell.lowerBound, credit.lowerBound, "Not contributed sits under Waiting and Contributed")
        XCTAssertTrue(home.contains(
            "HomeStatTile(label: MonitorWords.contributed, value: HomeFormat.count(freshRollup?.allTime?.accepted), "
                + "systemImage: HomeStatIcons.contributed, action: openHistory)"))
        // The credit summary after a failed read is no summary (D6 then
        // shows a dash). The condition gates the figure but is not drawn
        // (owner, 2026-10-13).
        XCTAssertTrue(home.contains("SummaryFacts.fresh(store.credit, unless: store.failures[\"commons_credit_summary\"])"))
        XCTAssertTrue(home.contains("SummaryFacts.fresh(store.rollup, unless: store.failures[\"history_rollup\"])"))
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

    /// Once #1703 reports decided credit as final, Home shows History's
    /// Final figure, with what is still pending under it (owner,
    /// 2026-10-28: Home and History must not disagree). Before anything is
    /// decided it stays the pending figure, marked Pending, under D6.
    func test_homeCreditTileShowsFinalOnceDecided() throws {
        let reply = try XCTUnwrap(SampleDaemonData.reply("commons_credit_summary", in: .normalDay))
        var object = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(reply.utf8)) as? [String: Any])
        object["posture_state"] = "known"
        object["commons_settlement"] = "pending_review"
        object["commons_settlement_explanation"] = "Scored when the commons reviews it."
        let known = try DaemonDataDecoding.decoder().decode(
            DaemonData.CommonsCreditSummary.self, from: JSONSerialization.data(withJSONObject: object))

        let undecided = HomeFormat.creditTile(final: 0, pending: 50.35, accepted: 3, credit: known)
        XCTAssertEqual(undecided.value, HomeFormat.points(50.35))
        XCTAssertEqual(undecided.detail, MonitorWords.pending)
        XCTAssertEqual(HomeFormat.creditTile(final: nil, pending: 50.35, accepted: nil, credit: nil).value, "—")

        let decided = HomeFormat.creditTile(final: 48, pending: 2.35, accepted: 3, credit: known)
        XCTAssertEqual(decided.value, HomeFormat.points(48))
        XCTAssertEqual(decided.detail, HomeFormat.creditPendingAccessory(2.35, condition: HomeFormat.pendingCondition(known)))
        XCTAssertNotEqual(decided.detail, MonitorWords.pending)

        let settled = HomeFormat.creditTile(final: 48, pending: 0, accepted: 3, credit: known)
        XCTAssertEqual(settled.value, HomeFormat.points(48))
        XCTAssertEqual(settled.detail, MonitorWords.final)
        // Every accepted trace decided with no award (#1703: 0 final, 0
        // pending) reads Final 0, never Pending.
        let nothingEarned = HomeFormat.creditTile(final: 0, pending: 0, accepted: 22, credit: known)
        XCTAssertEqual(nothingEarned.value, HomeFormat.points(0))
        XCTAssertEqual(nothingEarned.detail, MonitorWords.final)
        // With nothing accepted yet it is still the pending figure.
        XCTAssertEqual(HomeFormat.creditTile(final: 0, pending: 0, accepted: 0, credit: known).detail,
                       MonitorWords.pending)
        // Without the commons' statement, pending is not shown as a figure.
        XCTAssertEqual(HomeFormat.creditTile(final: 48, pending: 2.35, accepted: 3, credit: nil).detail, MonitorWords.final)
    }

    private func creditSummary(reason: String?, pointsState: String = "unknown") throws -> DaemonData.CommonsCreditSummary {
        let reply = try XCTUnwrap(SampleDaemonData.reply("commons_credit_summary", in: .normalDay))
        var object = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(reply.utf8)) as? [String: Any])
        object["points_state"] = pointsState
        object["points_unknown_reason"] = reason as Any? ?? NSNull()
        return try DaemonDataDecoding.decoder().decode(
            DaemonData.CommonsCreditSummary.self, from: JSONSerialization.data(withJSONObject: object))
    }

    /// A summary whose points are unknown because no account session could
    /// be used or renewed says "Sign in again" (the payout link's own
    /// constant, from the core) in place of the silent dash. Any other
    /// unknown, a known figure, and a core that sends no reason keep the dash.
    func test_creditTileSaysSignInAgainOnlyWhenTheSessionIsMissing() throws {
        let line = "Sign in again"
        XCTAssertEqual(
            HomeFormat.creditSignIn(try creditSummary(reason: "account-session-required"), line: line), line)
        XCTAssertNil(HomeFormat.creditSignIn(try creditSummary(reason: nil), line: line))
        XCTAssertNil(HomeFormat.creditSignIn(try creditSummary(reason: "something-else"), line: line))
        XCTAssertNil(HomeFormat.creditSignIn(
            try creditSummary(reason: "account-session-required", pointsState: "known"), line: line))
        XCTAssertNil(HomeFormat.creditSignIn(nil, line: line))
        XCTAssertNil(HomeFormat.creditSignIn(try creditSummary(reason: "account-session-required"), line: nil))
        XCTAssertNil(HomeFormat.creditSignIn(try creditSummary(reason: "account-session-required"), line: ""))
    }

    /// The tile reads the line from the core's payout-link export, not a
    /// string of this shell's.
    func test_creditSignInLineIsThePayoutLinkConstant() throws {
        XCTAssertEqual(HomeFormat.creditSignInLine, "Sign in again")
        let home = try HistoryParityTests.text("Views/Monitor/HomeViews.swift")
        XCTAssertTrue(home.contains("detail: signIn ?? creditTile.detail"))
        XCTAssertFalse(home.contains("\"Sign in again\""))
    }
}
#endif
