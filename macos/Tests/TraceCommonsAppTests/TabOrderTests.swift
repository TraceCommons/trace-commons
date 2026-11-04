import XCTest

@testable import TCDesign
@testable import TraceCommonsApp

/// The window's tabs, in the owner's default order (2026-11-02): Home,
/// Private AI, Traces, Insights, with Cmd-1..4 following the strip. Every
/// tab but Home can be dragged into another order (owner, 2026-10-30), once
/// a held press has lifted it (owner, 2026-11-02), and the shortcuts follow
/// it.
@MainActor
final class TabOrderTests: XCTestCase {
    typealias Tab = MonitorWindowView.Tab

    func test_privateAIIsSecondAndTracesThird() {
        XCTAssertEqual(Tab.allCases, [.home, .inference, .traces, .insights])
        XCTAssertEqual(Tab.ordered(""), [.home, .inference, .traces, .insights])
        XCTAssertEqual(MonitorCommands.shortcut(.inference), "2")
        XCTAssertEqual(MonitorCommands.shortcut(.traces), "3")
    }

    /// An order the person dragged into before the default changed is
    /// theirs, and is kept.
    func test_aStoredOrderOutlivesTheNewDefault() {
        XCTAssertEqual(Tab.ordered("Traces,Inference,Insights"), [.home, .traces, .inference, .insights])
    }

    func test_homeIsTheOnlyTabThatCannotMove() {
        XCTAssertEqual(Tab.allCases.filter(\.movable), [.inference, .traces, .insights])
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
        XCTAssertEqual(Tab.ordered("Home,Insights,Bogus,Insights"), [.home, .insights, .inference, .traces])
        XCTAssertEqual(Tab.ordered("Inference"), [.home, .inference, .traces, .insights])
    }

    func test_draggingATabTakesTheTargetsPlaceButNeverHomes() {
        let start = Tab.ordered("")
        XCTAssertEqual(Tab.moving(.inference, to: .traces, in: start), [.home, .traces, .inference, .insights])
        XCTAssertEqual(Tab.moving(.traces, to: .insights, in: start), [.home, .inference, .insights, .traces])
        XCTAssertEqual(Tab.moving(.insights, to: .home, in: start), start)
        XCTAssertEqual(Tab.moving(.home, to: .traces, in: start), start)
    }

    /// While onboarding is required the strip shows Private AI alone,
    /// whatever order is stored.
    func test_onboardingShowsPrivateAIAloneWhateverTheOrder() {
        XCTAssertEqual(Tab.shown(requiresOnboarding: true, order: "Insights,Traces"), [.inference])
        XCTAssertEqual(Tab.shown(requiresOnboarding: false, order: "Insights"), [.home, .insights, .inference, .traces])
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

    /// A held press lifts a tab; it pops out a little larger and raised;
    /// once lifted, a few points of travel make letting go a move, not a
    /// click (owner, 2026-11-02).
    func test_aHeldPressLiftsTheTabAndPopsItOut() throws {
        typealias Strip = GlassRuleTabs<Int>
        XCTAssertGreaterThanOrEqual(Strip.liftDelay, 0.25, "a click that lingers is not a lift")
        XCTAssertLessThanOrEqual(Strip.liftDelay, 0.5)
        XCTAssertGreaterThan(Strip.liftScale, 1)
        XCTAssertLessThan(Strip.liftScale, 1.1, "slightly")
        XCTAssertGreaterThan(Strip.liftRise, 0)
        XCTAssertFalse(Strip.hasMoved(0))
        XCTAssertFalse(Strip.hasMoved(Strip.dragThreshold - 1))
        XCTAssertTrue(Strip.hasMoved(Strip.dragThreshold))
        XCTAssertTrue(Strip.hasMoved(-Strip.dragThreshold - 4))
        let navigation = try String(
            contentsOf: GlassSurfaceRulesTests.root.deletingLastPathComponent()
                .appendingPathComponent("TCDesign/Components/Navigation.swift"), encoding: .utf8)
        // A quick drag moves nothing: the drag only follows a held press.
        XCTAssertTrue(navigation.contains(
            "LongPressGesture(minimumDuration: Self.liftDelay, maximumDistance: Self.liftSlop)\n"
                + "            .sequenced(before: DragGesture(minimumDistance: 0, coordinateSpace: .global))"))
        XCTAssertFalse(navigation.contains("DragGesture(minimumDistance: Self.dragThreshold"))
        // The lift is cleared however the press ends, so a tab always drops.
        XCTAssertTrue(navigation.contains("@GestureState private var lifted: Value?"))
        XCTAssertTrue(navigation.contains("if now == nil, dragging != nil { drop() }"))
        XCTAssertTrue(navigation.contains(".scaleEffect(up ? Self.liftScale : 1)"))
        XCTAssertTrue(navigation.contains(".glassSurface(.control, radius: GlassTokens.Radius.control)"))
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
