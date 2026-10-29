import AppKit
import SwiftUI
import XCTest
import TCBridge
@testable import TraceCommonsApp

/// Exercise production collections in a bounded native scroll viewport.
/// Scroll checks ensure that bounding responders never drops later rows.
@MainActor
final class InsightsCollectionsLazyTests: XCTestCase {
    func testPatternSessionRowsStayBoundedAndRemainReachable() throws {
        let copy = try InsightsSessionRouteTests.coreCopy()
        let small = measure(InsightsPatternSessionsView(found: try patterns(20), copy: copy), first: "session-0",
                            collection: "insightsPatternSessions")
        let large = measure(InsightsPatternSessionsView(found: try patterns(200), copy: copy), first: "session-0",
                            targets: [(0.5, "session-100"), (1, "session-199"), (0, "session-0")],
                            collection: "insightsPatternSessions")
        XCTAssertGreaterThan(small, 0)
        XCTAssertLessThanOrEqual(large, small + 100, "Offscreen pattern sessions must not grow the responder tree")
    }

    private func patterns(_ count: Int) throws -> InsightsPatternSessions {
        let rows: [[String: Any]] = (0..<count).map { index in
            ["session_ref": "session-\(index)", "count": 1,
             "tokens": index == 0 ? 101 : index == count / 2 ? 202 : index == count - 1 ? 303 : 123,
             "state": "known", "reasons": []]
        }
        return try decode(["pattern": "repeated_reads", "feed": "saved", "week_start": "2026-09-07", "tz": 0,
                           "sessions": rows])
    }

    private func decode<T: Decodable>(_ object: [String: Any]) throws -> T {
        try JSONDecoder().decode(T.self, from: JSONSerialization.data(withJSONObject: object))
    }

    private func measure(_ content: some View, first: String, targets: [(CGFloat, String)] = [], collection: String) -> Int {
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
        hosting.layoutSubtreeIfNeeded()
        window.displayIfNeeded()
        _ = hosting.accessibilityChildren()
        hosting.layoutSubtreeIfNeeded()
        func descendants(_ view: NSView) -> [NSView] { [view] + view.subviews.flatMap(descendants) }
        // Native text backing differs between macOS releases. Row-body
        // construction proves laziness even when AX creates no NSText views.
        func contains(_ expected: String) -> Bool {
            OtherCollectionRowBodies.built[collection]?.contains(expected) == true
        }
        let initialRows = OtherCollectionRowBodies.built[collection]?.count ?? 0
        XCTAssertGreaterThan(initialRows, 0, "The viewport must construct rows")
        XCTAssertLessThan(initialRows, 80, "Offscreen rows must not be constructed with the whole dataset")
        XCTAssertTrue(contains(first), "The first row must be materialized")
        let offscreen = targets.first(where: { $0.0 == 1 })?.1
        if let offscreen {
            XCTAssertFalse(contains(offscreen), "The last row must stay unbuilt before scrolling")
        }
        let initialCount = descendants(hosting).count
        print("INSIGHTS_COLLECTION collection=\(collection) rows=\(initialRows) views=\(initialCount)")
        let scroll = descendants(hosting).compactMap { $0 as? NSScrollView }.first
        XCTAssertNotNil(scroll)
        for (fraction, expected) in targets {
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
            } while !contains(expected) && Date() < deadline
            XCTAssertTrue(contains(expected), "Row at scroll fraction \(fraction) must remain reachable and materialize")
            if fraction == 0, let scroll {
                XCTAssertEqual(scroll.contentView.bounds.minY, 0, accuracy: 1, "Returning to the first row must restore the scroll position")
            }
        }
        return initialCount
    }
}
