import AppKit
import SwiftUI
import XCTest
import TCBridge
@testable import TraceCommonsApp

/// A bounded viewport must not instantiate selectable platform text views
/// for an entire week's sessions. Those views participate in every focus walk.
///
/// The two Breakdown lists open folded to five rows (#1648); they are
/// measured with Show all on, where the whole week is in the list.
@MainActor
final class InsightsResponderScaleTests: XCTestCase {
    func testSavedSessionsKeepPlatformViewsBounded() throws {
        let copy = try InsightsSessionRouteTests.coreCopy()
        let a = measure(InsightsCardInputsView(inputs: try savedSessions(20), snapshots: [], copy: copy, showAll: true), collection: "insightsSavedSessions", first: "session-0")
        let b = measure(InsightsCardInputsView(inputs: try savedSessions(1061), snapshots: [], copy: copy, showAll: true), collection: "insightsSavedSessions", first: "session-0",
                        scrollTargets: [(0.5, "session-530"), (1, "session-1060"), (0, "session-0")])
        print("INSIGHTS_RESPONDERS saved small=\(a) large=\(b)")
        XCTAssertGreaterThan(a, 0)
        XCTAssertLessThanOrEqual(b, a + 100, "Offscreen saved sessions must not grow the platform responder tree with the dataset")
    }

    func testCounterSessionsKeepPlatformViewsBounded() throws {
        let copy = try InsightsSessionRouteTests.coreCopy()
        let small = try counterSessions(20)
        let large = try counterSessions(1061)
        let a = measure(InsightsCounterSessionsView(sessions: small, copy: copy, showAll: true), collection: "insightsCounterSessions", first: "0")
        let b = measure(InsightsCounterSessionsView(sessions: large, copy: copy, showAll: true), collection: "insightsCounterSessions", first: "0",
                        scrollTargets: [(0.5, "530"), (1, "1060"), (0, "0")])
        print("INSIGHTS_RESPONDERS counter small=\(a) large=\(b)")
        XCTAssertGreaterThan(a, 0)
        XCTAssertLessThanOrEqual(b, a + 100, "Offscreen sessions must not grow the platform responder tree with the dataset")
    }

    private func counterSessions(_ count: Int) throws -> InsightsCounterSessions {
        var object = try InsightsSessionRouteTests.recorded()
        var rollup = try XCTUnwrap(object["rollup"] as? [String: Any])
        let row = try XCTUnwrap((rollup["sessions"] as? [[String: Any]])?.first)
        rollup["sessions"] = (0..<count).map { index in
            var row = row
            row["tokens"] = index == 0 ? 101 : index == count / 2 ? 202 : index == count - 1 ? 303 : 123
            return row
        }
        object["rollup"] = rollup
        return try XCTUnwrap(InsightsCounterSessions(week: InsightsSessionRouteTests.decode(object)))
    }

    private func savedSessions(_ count: Int) throws -> InsightsCardInputs {
        let sessions: [[String: Any]] = (0..<count).map { index in
            ["session_ref": "session-\(index)", "source": "claude_code",
             "tokens": index == 0 ? 101 : index == count / 2 ? 202 : index == count - 1 ? 303 : 123,
             "state": "known", "reasons": []]
        }
        let object: [String: Any] = [
            "card": "sessions", "feed": "saved", "week_start": "2026-09-07", "tz": 0,
            "sources": [], "coverage": ["known": count, "partial": 0, "unknown": 0, "reasons": [:]],
            "sessions": sessions
        ]
        return try JSONDecoder().decode(InsightsCardInputs.self, from: JSONSerialization.data(withJSONObject: object))
    }

    private func measure(_ content: some View, collection: String, first: String,
                         scrollTargets: [(CGFloat, String)] = []) -> Int {
        OtherCollectionRowBodies.startRecording()
        defer { OtherCollectionRowBodies.stopRecording() }
        _ = NSApplication.shared
        let previous = NSApp.value(forKey: "accessibilityEnhancedUserInterface")
        NSApp.setValue(true, forKey: "accessibilityEnhancedUserInterface")
        defer { NSApp.setValue(previous, forKey: "accessibilityEnhancedUserInterface") }
        let bounds = NSRect(x: 0, y: 0, width: 408, height: 760)
        let hosting = NSHostingView(rootView: ScrollView { content }.frame(width: 408, height: 760))
        hosting.frame = bounds
        let window = NSWindow(contentRect: bounds, styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = hosting
        defer { window.close() }
        let start = Date()
        hosting.layoutSubtreeIfNeeded()
        window.displayIfNeeded()
        _ = hosting.accessibilityChildren()
        hosting.layoutSubtreeIfNeeded()
        func descendants(_ view: NSView) -> [NSView] {
            [view] + view.subviews.flatMap(descendants)
        }
        let views = descendants(hosting)
        // SwiftUI text is not backed by an NSView accessibilityValue on
        // every macOS version. Measure construction independently of that
        // platform detail, while retaining native-view bounds as well.
        func built(_ id: String) -> Bool { OtherCollectionRowBodies.built[collection]?.contains(id) == true }
        XCTAssertTrue(built(first), "The first session must be constructed")
        let realized = OtherCollectionRowBodies.built[collection]?.count ?? 0
        XCTAssertLessThan(realized, 80, "The viewport must not construct the whole week")
        if let last = scrollTargets.first(where: { $0.0 == 1 })?.1 {
            XCTAssertFalse(built(last), "The last session must stay unbuilt until scrolling")
        }
        let count = views.count
        print("INSIGHTS_LAYOUT views=\(count) seconds=\(Date().timeIntervalSince(start))")
        // Read only the materialized native views, not the entire AX tree:
        // a whole-tree query can itself force offscreen row realization.
        let scroll = views.compactMap { $0 as? NSScrollView }.first
        XCTAssertNotNil(scroll)
        for (fraction, expected) in scrollTargets {
            let deadline = Date().addingTimeInterval(2)
            repeat {
                if let scroll, let document = scroll.documentView {
                    let maximum = max(0, document.bounds.height - scroll.contentView.bounds.height)
                    scroll.contentView.scroll(to: NSPoint(x: 0, y: maximum * fraction))
                    scroll.reflectScrolledClipView(scroll.contentView)
                }
                RunLoop.main.run(until: Date().addingTimeInterval(0.01))
                hosting.layoutSubtreeIfNeeded()
                window.displayIfNeeded()
            } while !built(expected) && Date() < deadline
            XCTAssertTrue(built(expected), "Session at scroll fraction \(fraction) must remain reachable and construct its row")
            if fraction == 0, let scroll {
                XCTAssertEqual(scroll.contentView.bounds.minY, 0, accuracy: 1, "Returning to the first row must restore the scroll position")
            }
        }
        return count
    }
}
