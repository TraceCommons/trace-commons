import XCTest

@testable import TCDesign
@testable import TraceCommonsApp

/// The window's tabs, in the owner's default order (2026-10-28): Home,
/// Traces, Private AI, Insights, with Cmd-1..4 following the strip. Every
/// tab but Home can be dragged into another order (owner, 2026-10-30), and
/// the shortcuts follow it.
@MainActor
final class TabOrderTests: XCTestCase {
    typealias Tab = MonitorWindowView.Tab

    func test_tracesIsSecondAndPrivateAIThird() {
        XCTAssertEqual(Tab.allCases, [.home, .traces, .inference, .insights])
        XCTAssertEqual(Tab.ordered(""), [.home, .traces, .inference, .insights])
        XCTAssertEqual(MonitorCommands.shortcut(.traces), "2")
        XCTAssertEqual(MonitorCommands.shortcut(.inference), "3")
    }

    func test_homeIsTheOnlyTabThatCannotMove() {
        XCTAssertEqual(Tab.allCases.filter(\.movable), [.traces, .inference, .insights])
    }

    func test_aStoredOrderIsReadBackAfterHome() {
        let order = Tab.ordered("Insights,Inference,Traces")
        XCTAssertEqual(order, [.home, .insights, .inference, .traces])
        XCTAssertEqual(Tab.stored(order), "Insights,Inference,Traces")
        XCTAssertEqual(MonitorCommands.shortcut(.home, order: "Insights,Inference,Traces"), "1")
        XCTAssertEqual(MonitorCommands.shortcut(.insights, order: "Insights,Inference,Traces"), "2")
        XCTAssertEqual(MonitorCommands.shortcut(.traces, order: "Insights,Inference,Traces"), "4")
    }

    /// Home stays first even if a stored order names it; an unknown or
    /// repeated name is skipped; a tab the order does not name (one added
    /// since) joins at the end.
    func test_aDamagedOrStaleStoredOrderStillShowsEveryTabOnce() {
        XCTAssertEqual(Tab.ordered("Home,Insights,Bogus,Insights"), [.home, .insights, .traces, .inference])
        XCTAssertEqual(Tab.ordered("Inference"), [.home, .inference, .traces, .insights])
    }

    func test_draggingATabTakesTheTargetsPlaceButNeverHomes() {
        let start = Tab.ordered("")
        XCTAssertEqual(Tab.moving(.inference, to: .traces, in: start), [.home, .inference, .traces, .insights])
        XCTAssertEqual(Tab.moving(.traces, to: .insights, in: start), [.home, .inference, .insights, .traces])
        XCTAssertEqual(Tab.moving(.insights, to: .home, in: start), start)
        XCTAssertEqual(Tab.moving(.home, to: .traces, in: start), start)
    }

    /// While onboarding is required the strip shows Private AI alone,
    /// whatever order is stored.
    func test_onboardingShowsPrivateAIAloneWhateverTheOrder() {
        XCTAssertEqual(Tab.shown(requiresOnboarding: true, order: "Insights,Traces"), [.inference])
        XCTAssertEqual(Tab.shown(requiresOnboarding: false, order: "Insights"), [.home, .insights, .traces, .inference])
    }

    /// A drag lands on the slot whose middle the tab has passed, and never
    /// on Home's slot or past the strip's end.
    func test_aDragLandsPastAMiddleAndStaysAmongTheMovableTabs() {
        let landing = GlassRuleTabs<Int>.landing
        XCTAssertEqual(landing(1, 40, 100, 1...3), 1)
        XCTAssertEqual(landing(1, 60, 100, 1...3), 2)
        XCTAssertEqual(landing(1, 260, 100, 1...3), 3)
        XCTAssertEqual(landing(2, -400, 100, 1...3), 1)
        XCTAssertEqual(landing(3, 500, 100, 1...3), 3)
    }

    /// The main window's strip moves tabs by drag alone, Home excepted, and
    /// saves the order where every window and the View menu read it.
    func test_theStripMovesTabsByDragAndSavesTheOrder() throws {
        let window = try String(
            contentsOf: GlassSurfaceRulesTests.root.appendingPathComponent("Views/MonitorWindowView.swift"), encoding: .utf8)
        XCTAssertTrue(window.contains("@AppStorage(MonitorWindowView.Tab.orderKey) private var tabOrder"))
        XCTAssertTrue(window.contains("movable: { $0.movable },"))
        XCTAssertTrue(window.contains("tabOrder = Tab.stored(Tab.moving(item, to: target, in: Tab.ordered(tabOrder)))"))
        let main = try String(
            contentsOf: GlassSurfaceRulesTests.root.appendingPathComponent("TraceCommonsAppMain.swift"), encoding: .utf8)
        XCTAssertTrue(main.contains("ForEach(MonitorWindowView.Tab.ordered(tabOrder))"))
    }
}
