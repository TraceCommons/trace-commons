import SwiftUI
import XCTest

@testable import TCDesign

/// R13: the menu-bar popover's graph and strip draw counts honestly.
final class MenuBarPanelTests: XCTestCase {
    /// A day with nothing draws no bar; any count draws at least 6pt, and
    /// more draws taller.
    func test_aZeroDrawsNothingAndAnyCountIsVisible() {
        XCTAssertEqual(GlassDayGraph.height(0, max: 5), 0)
        XCTAssertGreaterThanOrEqual(GlassDayGraph.height(1, max: 100), 6)
        XCTAssertGreaterThan(GlassDayGraph.height(5, max: 5), GlassDayGraph.height(1, max: 5))
        XCTAssertLessThanOrEqual(GlassDayGraph.height(5, max: 5), GlassDayGraph.plotHeight / 2)
    }

    /// The bars fit the item: six steps of two bars make a graph of under
    /// 18pt, centred in the item's height as the mark is, and each kind of
    /// bar's busiest step reaches the graph's edge without crossing it.
    func test_theBarsStayInsideTheItem() {
        XCTAssertLessThanOrEqual(GlassMenuBarStrip.graphWidth(steps: 6), 18)
        XCTAssertEqual(GlassMenuBarStrip.graphWidth(steps: 0), 0)
        XCTAssertEqual(
            GlassMenuBarStrip.reach * 2 + GlassMenuBarStrip.centreLine, GlassMenuBarStrip.graphHeight)
        XCTAssertEqual(
            GlassMenuBarStrip.graphTop + GlassMenuBarStrip.graphHeight / 2, GlassMenuBarStrip.height / 2,
            "the graph is centred in the item")
        XCTAssertEqual(
            (GlassMenuBarStrip.height - GlassMenuBarStrip.markSize) / 2 + GlassMenuBarStrip.markSize / 2,
            GlassMenuBarStrip.height / 2, "and so is the mark")

        let busy = (0 ..< 6).map { GlassDayColumn(id: String($0), up: $0, down: 6 - $0) }
        let steps = GlassMenuBarStrip.steps(traces: busy, privateAI: busy, condition: .live)
        XCTAssertEqual(steps.map { max($0.shared, $0.kept) }.max(), GlassMenuBarStrip.reach)
        XCTAssertEqual(steps.map { max($0.verified, $0.calls) }.max(), GlassMenuBarStrip.reach)
    }

    /// A step with nothing draws no segment; any count draws at least
    /// `least`, beside a busy step.
    func test_aZeroDrawsNoSegmentAndAnyCountShows() {
        let traces = [GlassDayColumn(id: "0", up: 0, down: 1), GlassDayColumn(id: "1", up: 1_000, down: 0)]
        let steps = GlassMenuBarStrip.steps(traces: traces, privateAI: nil, condition: .live)
        XCTAssertEqual(steps[0].shared, 0)
        XCTAssertGreaterThanOrEqual(steps[0].kept, GlassMenuBarStrip.least)
        XCTAssertEqual(steps[1].kept, 0)
        XCTAssertEqual(steps[1].shared, GlassMenuBarStrip.reach)
    }

    /// Private AI and traces are separate bars, each on its own scale:
    /// verified never outreaches the calls it is counted among, and a
    /// handful of traces reach as far as hundreds of calls.
    func test_eachKindOfBarHasItsOwnScale() {
        let traces = [GlassDayColumn(id: "0", up: 1, down: 1), GlassDayColumn(id: "1", up: 1, down: 1)]
        let ai = [GlassDayColumn(id: "0", up: 250, down: 500), GlassDayColumn(id: "1", up: 500, down: 500)]
        let steps = GlassMenuBarStrip.steps(traces: traces, privateAI: ai, condition: .live)
        XCTAssertEqual(steps[0].verified, GlassMenuBarStrip.reach / 2)
        XCTAssertEqual(steps[0].calls, GlassMenuBarStrip.reach)
        XCTAssertEqual(steps[1].verified, steps[1].calls)
        XCTAssertEqual(steps[0].shared, GlassMenuBarStrip.reach, "traces are not measured against calls")
    }

    /// The badge shows two digits at most: 99+ past 99.
    func test_theBadgeStopsAtNinetyNine() {
        XCTAssertEqual(GlassMenuBarStrip.badgeText(1), "1")
        XCTAssertEqual(GlassMenuBarStrip.badgeText(99), "99")
        XCTAssertEqual(GlassMenuBarStrip.badgeText(100), "99+")
        XCTAssertEqual(GlassMenuBarStrip.badgeText(1_200), "99+")
    }

    /// A core that is down or unread draws no bars, no Private AI, no badge
    /// and the attention mark, never the last activity; trouble keeps the
    /// bars and adds the mark; pausing greys only the trace segments; live
    /// data draws as recorded.
    func test_anUnavailableCoreIsNeverDrawnAsActivity() {
        let columns = (0 ..< 6).map { GlassDayColumn(id: String($0), up: $0, down: 6 - $0) }
        let down = GlassMenuBarStrip.steps(traces: columns, privateAI: columns, condition: .unavailable)
        XCTAssertEqual(down.count, 6)
        XCTAssertTrue(down.allSatisfy { $0 == GlassMenuBarStrip.Step() })
        XCTAssertNil(GlassMenuBarStrip.shownPrivateAI(columns, condition: .unavailable))
        XCTAssertNil(GlassMenuBarStrip.shownBadge(4, condition: .unavailable))
        XCTAssertTrue(GlassMenuBarStrip.showsAttention(.unavailable))
        XCTAssertTrue(GlassMenuBarStrip.grey(.unavailable))

        let live = GlassMenuBarStrip.steps(traces: columns, privateAI: columns, condition: .live)
        XCTAssertTrue(live.contains { $0.shared > 0 && $0.verified > 0 })
        XCTAssertEqual(GlassMenuBarStrip.shownBadge(4, condition: .live), 4)
        XCTAssertFalse(GlassMenuBarStrip.showsAttention(.live))
        XCTAssertFalse(GlassMenuBarStrip.grey(.live))

        XCTAssertEqual(GlassMenuBarStrip.steps(traces: columns, privateAI: columns, condition: .attention), live)
        XCTAssertTrue(GlassMenuBarStrip.showsAttention(.attention))
        XCTAssertTrue(GlassMenuBarStrip.grey(.paused))
        XCTAssertEqual(
            GlassMenuBarStrip.steps(traces: columns, privateAI: columns, condition: .paused), live,
            "pausing greys the trace bars; it keeps every height, Private AI's included")
        let unknown = GlassMenuBarStrip.steps(traces: columns, privateAI: nil, condition: .live)
        XCTAssertTrue(unknown.allSatisfy { $0.verified == 0 && $0.calls == 0 }, "unknown Private AI draws nothing")
    }

    /// The nudge mark: the news ring only in the badge's empty slot on a
    /// live strip; the halo only around a badge that is drawn. Never over
    /// the attention mark, never on an unavailable strip.
    func test_theMarkIsDrawnOnlyWhereItsSlotIs() {
        XCTAssertEqual(GlassMenuBarStrip.shownMark(.news, badge: nil, condition: .live), .news)
        XCTAssertEqual(GlassMenuBarStrip.shownMark(.news, badge: 3, condition: .live), .none)
        XCTAssertEqual(GlassMenuBarStrip.shownMark(.news, badge: nil, condition: .paused), .none)
        XCTAssertEqual(GlassMenuBarStrip.shownMark(.news, badge: nil, condition: .attention), .none)
        XCTAssertEqual(GlassMenuBarStrip.shownMark(.news, badge: nil, condition: .unavailable), .none)

        XCTAssertEqual(GlassMenuBarStrip.shownMark(.ready, badge: 3, condition: .live), .ready)
        XCTAssertEqual(GlassMenuBarStrip.shownMark(.ready, badge: 3, condition: .paused), .ready)
        XCTAssertEqual(GlassMenuBarStrip.shownMark(.ready, badge: nil, condition: .live), .none)
        XCTAssertEqual(GlassMenuBarStrip.shownMark(.ready, badge: 0, condition: .live), .none)
        XCTAssertEqual(GlassMenuBarStrip.shownMark(.ready, badge: 3, condition: .unavailable), .none)

        XCTAssertEqual(GlassMenuBarStrip.shownMark(.none, badge: 3, condition: .live), .none)
    }

    /// A menu row's own hit rect clears the 28pt target: the room between
    /// two selections belongs to a row, not to a gap no button owns.
    @MainActor
    func test_aMenuRowsHitRectClearsTwentyEightPoints() {
        let row = Button("Quit…") {}.buttonStyle(GlassMenuRowStyle()).frame(width: 240)
        let height = NSHostingView(rootView: row.fixedSize(horizontal: false, vertical: true)).fittingSize.height
        XCTAssertGreaterThanOrEqual(height, 28)
    }
}
