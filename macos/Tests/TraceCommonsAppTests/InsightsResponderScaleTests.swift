import AppKit
import SwiftUI
import XCTest
import TCBridge
import TCShellCore
@testable import TraceCommonsApp

/// A bounded viewport must not instantiate selectable platform text views
/// for an entire week's sessions. Those views participate in every focus walk.
@MainActor
final class InsightsResponderScaleTests: XCTestCase {
    func testSavedSessionsKeepPlatformViewsBounded() throws {
        let copy = try InsightsSessionRouteTests.coreCopy()
        let a = measure(InsightsCardInputsView(inputs: try savedSessions(20), snapshots: [], copy: copy), collection: "insightsSavedSessions", first: "session-0")
        let b = measure(InsightsCardInputsView(inputs: try savedSessions(1061), snapshots: [], copy: copy), collection: "insightsSavedSessions", first: "session-0",
                        scrollTargets: [(0.5, "session-530"), (1, "session-1060"), (0, "session-0")])
        print("INSIGHTS_RESPONDERS saved small=\(a) large=\(b)")
        XCTAssertGreaterThan(a, 0)
        XCTAssertLessThanOrEqual(b, a + 100, "Offscreen saved sessions must not grow the platform responder tree with the dataset")
    }

    func testCounterSessionsKeepPlatformViewsBounded() throws {
        let copy = try InsightsSessionRouteTests.coreCopy()
        let small = try counterSessions(20)
        let large = try counterSessions(1061)
        let a = measure(InsightsCounterSessionsView(sessions: small, copy: copy), collection: "insightsCounterSessions", first: "0")
        let b = measure(InsightsCounterSessionsView(sessions: large, copy: copy), collection: "insightsCounterSessions", first: "0",
                        scrollTargets: [(0.5, "530"), (1, "1060"), (0, "0")])
        print("INSIGHTS_RESPONDERS counter small=\(a) large=\(b)")
        XCTAssertGreaterThan(a, 0)
        XCTAssertLessThanOrEqual(b, a + 100, "Offscreen sessions must not grow the platform responder tree with the dataset")
    }

    /// The Sessions tab's watched feed, every row with its Task and Outcome
    /// menus and Confirm: a page of 1,061 rows builds no more than a
    /// page of 20 does, and every row stays reachable by scrolling.
    func testWatchedFeedKeepsPlatformViewsBounded() async throws {
        let copy = try InsightsSessionRouteTests.coreCopy()
        let small = try await watchedModel(20)
        let large = try await watchedModel(1061)
        defer { small.close(); large.close() }
        XCTAssertTrue(small.canLabel)
        let a = measure(InsightsSessionsTab(model: small, snapshots: [], copy: copy),
                        collection: "insightsWatchedFeed", first: "w-0", scrolls: true)
        let b = measure(InsightsSessionsTab(model: large, snapshots: [], copy: copy),
                        collection: "insightsWatchedFeed", first: "w-0", scrolls: true,
                        scrollTargets: [(0.5, "w-530"), (1, "w-1060"), (0, "w-0")])
        print("INSIGHTS_RESPONDERS watched small=\(a) large=\(b)")
        XCTAssertGreaterThan(a, 0)
        XCTAssertLessThanOrEqual(b, a + 100, "Offscreen watched rows must not grow the platform responder tree with the feed")
    }

    /// A Sessions model showing one page of `count` watched rows, newest
    /// first, each with an inferred task to confirm.
    private func watchedModel(_ count: Int) async throws -> InsightsSessionsModel {
        let rows = (0..<count).map { index in
            #"{"session_ref":"w-\#(index)","source":"claude_code","started_at":"2026-10-19T10:00:00+00:00","turns":3,"tokens":\#(100 + index),"sparkline":{"points":[10,40,20],"peak_tokens":40,"peak_turn":2},"folder":"f","inferred_task":"bug_fix","user_task":null,"user_outcome":null}"#
        }
        let json = #"{"enabled":true,"feed":"counter_pass","readable":true,"sessions_stored":\#(count),"truncated":false,"next":null,"sessions":["#
            + rows.joined(separator: ",") + "]}"
        let page = try JSONDecoder().decode(DaemonData.InsightsSessions.self, from: Data(json.utf8))
        let model = InsightsSessionsModel(
            service: { _ in throw InsightsError.invalidResponse },
            watchedReader: { _ in page },
            watchedLabel: { _, _, _ in throw DaemonDataError.unreachable })
        model.open()
        model.showWatched(true)
        for _ in 0..<300 where model.watched?.count != count { try await Task.sleep(for: .milliseconds(10)) }
        XCTAssertEqual(model.watched?.count, count)
        return model
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

    /// `scrolls`: the content brings its own scroll view (a whole tab),
    /// so it is not put in another.
    private func measure(_ content: some View, collection: String, first: String, scrolls: Bool = false,
                         scrollTargets: [(CGFloat, String)] = []) -> Int {
        OtherCollectionRowBodies.startRecording()
        defer { OtherCollectionRowBodies.stopRecording() }
        _ = NSApplication.shared
        let previous = NSApp.value(forKey: "accessibilityEnhancedUserInterface")
        NSApp.setValue(true, forKey: "accessibilityEnhancedUserInterface")
        defer { NSApp.setValue(previous, forKey: "accessibilityEnhancedUserInterface") }
        let bounds = NSRect(x: 0, y: 0, width: 408, height: 760)
        let root = scrolls ? AnyView(content) : AnyView(ScrollView { content })
        let hosting = NSHostingView(rootView: root.frame(width: 408, height: 760))
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
