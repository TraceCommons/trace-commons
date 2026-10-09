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
        XCTAssertEqual(
            MenuPanelStatus.markAccessibility(base: "Trace Commons. 2 sessions waiting for your decision.",
                                              nudge: store.status?.nudge, available: true),
            "Trace Commons. 2 sessions waiting for your decision. 2 of them have been idle for 3 days or more.")
        XCTAssertEqual(
            MenuPanelStatus.markAccessibility(base: "Base.", nudge: store.status?.nudge, available: false), "Base.")
        let quiet = MenuPanelStore(client: SampleDaemonClient(.empty))
        await quiet.load()
        XCTAssertEqual(MenuPanelStatus.mark(quiet.status?.nudge, available: true), GlassMenuBarStrip.Mark.none)
    }
}
