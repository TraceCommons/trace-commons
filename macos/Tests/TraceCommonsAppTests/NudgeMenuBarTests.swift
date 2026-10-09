import TCDesign
@testable import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// The nudge in the menu bar: the panel row, what tapping it does, and the
/// mark on the strip with its words.
@MainActor
final class NudgeMenuBarTests: XCTestCase {
    func test_thePanelRowIsTheDaemonsRowAndOpensTheIdleSessions() async throws {
        let client = SampleDaemonClient(.normalDay)
        let store = MenuPanelStore(client: client)
        await store.load()
        let row = store.nudgeRow
        XCTAssertEqual(row?.text, "Some sessions have been idle for 3 days or more")
        let destination = await store.open(try XCTUnwrap(row))
        XCTAssertEqual(client.nudgeCalls, ["nudge_opened idle_sessions"])
        XCTAssertEqual(destination, .idleSessions)
    }

    /// A stale panel draws no row: what it holds is no longer current.
    func test_aStalePanelDrawsNoRow() async {
        let store = MenuPanelStore(client: SampleDaemonClient(.normalDay))
        XCTAssertNil(store.nudgeRow)
        let down = MenuPanelStore(client: SampleDaemonClient(.coreDown))
        await down.load()
        XCTAssertNil(down.nudgeRow)
    }

    /// The opened stamp is best effort for a row tap: the row still opens
    /// its place, since looking changes nothing.
    func test_aRefusedOpenStampStillOpensThePlace() async {
        let store = MenuPanelStore(client: SampleDaemonClient(.coreDown))
        let row = NudgeSurface.PanelRow(kind: .verdictsLanded, text: "t", intent: .seeHistory)
        let destination = await store.open(row)
        XCTAssertEqual(destination, .home(.history))
    }

    func test_eachNudgeDestinationIsAMonitorDestination() {
        XCTAssertEqual(MonitorDestination(nudge: .traces(idleOnly: true)), .idleSessions)
        XCTAssertEqual(MonitorDestination(nudge: .traces(idleOnly: false)), .traces(entryId: nil))
        XCTAssertEqual(MonitorDestination(nudge: .history), .home(.history))
    }

    /// The mark follows the daemon only while the strip can vouch for it.
    func test_theStripsMarkIsTheDaemonsWhileAvailable() async {
        let store = MenuPanelStore(client: SampleDaemonClient(.normalDay))
        await store.load()
        XCTAssertEqual(MenuPanelStatus.mark(store.status?.nudge, available: true), .ready)
        XCTAssertEqual(MenuPanelStatus.mark(store.status?.nudge, available: false), GlassMenuBarStrip.Mark.none)
        let drawn = MenuPanelStatus.markWords(store.status?.nudge, available: true, badge: 2, condition: .live)
        XCTAssertEqual(
            MenuPanelStatus.markAccessibility(base: "Trace Commons. 2 traces waiting for your decision.", words: drawn),
            "Trace Commons. 2 traces waiting for your decision. 2 of them have been idle for 3 days or more.")
        let unavailable = MenuPanelStatus.markWords(store.status?.nudge, available: false, badge: 2, condition: .live)
        XCTAssertEqual(MenuPanelStatus.markAccessibility(base: "Base.", words: unavailable), "Base.")
        let quiet = MenuPanelStore(client: SampleDaemonClient(.empty))
        await quiet.load()
        XCTAssertEqual(MenuPanelStatus.mark(quiet.status?.nudge, available: true), GlassMenuBarStrip.Mark.none)
    }

    private static let newsStatus = #"""
        {"nudge":{"state":"armed","lead":"verdicts_landed","count":3,"accepted":2,"held":1,"mark":"news",
         "mark_kinds":["verdicts_landed"],
         "mark_text":{"accessibility":"New: 2 accepted and 1 held for privacy review.","tooltip":"Verdicts are in."}}}
        """#

    /// The spoken label and the tooltip say what the strip draws: the
    /// core's mark sentence only while the ring or halo is on screen, never
    /// for a mark the badge or the condition hid.
    func test_theMarksWordsFollowTheMarkTheStripDraws() throws {
        let news = try DaemonDataDecoding.decoder().decode(DaemonData.Status.self, from: Data(Self.newsStatus.utf8)).nudge
        // The ring, drawn: no badge, a live strip.
        let ring = MenuPanelStatus.markWords(news, available: true, badge: nil, condition: .live)
        XCTAssertEqual(MenuPanelStatus.markAccessibility(base: "Base.", words: ring),
                       "Base. New: 2 accepted and 1 held for privacy review.")
        XCTAssertEqual(MenuPanelStatus.markTooltip(ring), "Verdicts are in.")
        // A badge holds the ring's slot, so no ring is drawn and none is said.
        for words in [
            MenuPanelStatus.markWords(news, available: true, badge: 1, condition: .live),
            MenuPanelStatus.markWords(news, available: true, badge: nil, condition: .attention),
            MenuPanelStatus.markWords(news, available: true, badge: nil, condition: .paused),
        ] {
            XCTAssertEqual(MenuPanelStatus.markAccessibility(base: "Base.", words: words), "Base.")
            XCTAssertEqual(MenuPanelStatus.markTooltip(words), "")
        }
    }

    /// The halo rings a drawn badge only; without one it is not said.
    func test_aHaloWithoutABadgeIsNotSaid() async {
        let store = MenuPanelStore(client: SampleDaemonClient(.normalDay))
        await store.load()
        let words = MenuPanelStatus.markWords(store.status?.nudge, available: true, badge: nil, condition: .live)
        XCTAssertEqual(MenuPanelStatus.markAccessibility(base: "Base.", words: words), "Base.")
        XCTAssertEqual(MenuPanelStatus.markTooltip(words), "")
    }
}
