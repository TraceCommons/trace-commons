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
        let small = measure(InsightsPatternSessionsView(found: try patterns(20), copy: copy), first: "101")
        let large = measure(InsightsPatternSessionsView(found: try patterns(200), copy: copy), first: "101",
                            targets: [(0.5, "202"), (1, "303"), (0, "101")])
        XCTAssertGreaterThan(small, 0)
        XCTAssertLessThanOrEqual(large, small + 100, "Offscreen pattern sessions must not grow the responder tree")
    }

    func testSummarySnapshotRowsStayBoundedAndRemainReachable() throws {
        let copy = try InsightsSessionRouteTests.coreCopy()
        let small = measure(InsightsSummaryView(summary: try summary(20), copy: copy, openSnapshot: { _ in }),
                            first: "snapshot-0", collection: "insightsSummarySnapshots")
        let large = measure(InsightsSummaryView(summary: try summary(200), copy: copy, openSnapshot: { _ in }),
                            first: "snapshot-0", targets: [(0.5, "snapshot-100"), (1, "snapshot-199"), (0, "snapshot-0")],
                            collection: "insightsSummarySnapshots")
        XCTAssertGreaterThan(small, 0)
        XCTAssertLessThanOrEqual(large, small + 100, "Offscreen summary evidence must not grow the responder tree")
    }

    func testFrozenComparisonMembersStayBounded() async throws {
        let copy = try InsightsSessionRouteTests.coreCopy()
        let smallModel = try await taskModel(episodes: 1, members: 4)
        let largeModel = try await taskModel(episodes: 4, members: 64)
        defer { smallModel.close(); largeModel.close() }
        let first = "comparison-frozen-" + String(format: "d0c18c96-6093-49f5-bb6f-%012d", 0)
            + "-" + String(format: "%064x", 0)
        let last = "comparison-frozen-" + String(format: "d0c18c96-6093-49f5-bb6f-%012d", 3)
            + "-" + String(format: "%064x", 255)
        let small = measure(ComparisonTasksView(model: smallModel, episodes: [], copy: copy,
                                                openEpisode: { _ in }, openSnapshot: { _ in }), first: first,
                            collection: "insightsFrozenMembers")
        let large = measure(ComparisonTasksView(model: largeModel, episodes: [], copy: copy,
                                                openEpisode: { _ in }, openSnapshot: { _ in }), first: first,
                            scanFor: last, collection: "insightsFrozenMembers")
        XCTAssertLessThanOrEqual(large, small + 100, "Frozen member buttons must be lazy within their episode rows")
    }

    private func taskModel(episodes: Int, members: Int) async throws -> ComparisonTasksModel {
        let taskID = "20c18c96-6093-49f5-bb6f-6092ef0630b9"
        let bindings: [[String: Any]] = (0..<episodes).map { episode in
            ["episode_id": String(format: "d0c18c96-6093-49f5-bb6f-%012d", episode),
             "revision": 1, "membership_revision": 1, "members_digest": String(repeating: "b", count: 64),
             "members": (0..<members).map { member in
                 ["snapshot_id": String(format: "%064x", episode * members + member),
                  "source_digest": String(repeating: "d", count: 64)]
             }]
        }
        let task: [String: Any] = [
            "schema_version": 1, "id": taskID, "revision": 1, "material_revision": 1,
            "material_digest": String(repeating: "a", count: 64), "created_at": "2026-09-12T00:00:00Z",
            "updated_at": "2026-09-12T00:00:00Z", "episodes": bindings,
        ]
        let list = try JSONSerialization.data(withJSONObject: ["type": "comparison_task_list", "tasks": []])
        let detail = try JSONSerialization.data(withJSONObject: [
            "type": "comparison_task_explain", "detail": ["task": task,
                "stale_reasons": ["context_incomplete", "attribution_pending_qualification"],
                "overlapping_task_ids": [], "resolved_at": "2026-09-12T00:00:00Z"],
        ])
        let model = ComparisonTasksModel(service: { request in
            try JSONDecoder().decode(InsightsResponse.self,
                                     from: request.operation.type == "comparison_task_list" ? list : detail)
        })
        model.open()
        for _ in 0..<300 where model.busy { try await Task.sleep(for: .milliseconds(10)) }
        XCTAssertFalse(model.busy)
        model.select(taskID)
        for _ in 0..<300 where model.busy { try await Task.sleep(for: .milliseconds(10)) }
        XCTAssertFalse(model.busy)
        XCTAssertNotNil(model.detail)
        return model
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

    private func summary(_ count: Int) throws -> SavedInsightsSummary {
        let rows: [[String: Any]] = (0..<count).map { index in
            ["id": "snapshot-\(index)", "source_format": "claude_code", "analyzed_at": "2026-09-07T00:00:00Z", "evidence": []]
        }
        return try decode([
            "schema_version": 1, "scope": "all_saved_selected_session_snapshots", "limitations": [],
            "provider": ["id": "fixture", "version": "1", "rubric_version": "1", "execution_mode": "local"],
            "saved_snapshots": count, "user_reported": ["assessed_snapshots": 0, "unassessed_snapshots": count,
                                                       "categories": [], "outcomes": []],
            "metrics": [], "snapshots": rows,
        ])
    }

    private func decode<T: Decodable>(_ object: [String: Any]) throws -> T {
        try JSONDecoder().decode(T.self, from: JSONSerialization.data(withJSONObject: object))
    }

    private func measure(_ content: some View, first: String, targets: [(CGFloat, String)] = [], scanFor: String? = nil, collection: String? = nil) -> Int {
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
        func readableValues() -> [String] {
            descendants(hosting).flatMap { view in
                [view.accessibilityValue() as? String, view.accessibilityLabel(), (view as? NSButton)?.title].compactMap { $0 }
            }
        }
        func contains(_ expected: String) -> Bool {
            if let collection, OtherCollectionRowBodies.built[collection]?.contains(expected) == true { return true }
            return readableValues().contains { $0.contains(expected) }
        }
        XCTAssertTrue(contains(first), "The first row must be materialized: \(readableValues().prefix(20))")
        if let collection {
            let offscreen = scanFor ?? targets.first(where: { $0.0 == 1 })?.1
            if let offscreen {
                XCTAssertFalse(OtherCollectionRowBodies.built[collection]?.contains(offscreen) == true,
                               "The last row must stay unbuilt before scrolling")
            }
        }
        let initialCount = descendants(hosting).count
        print("INSIGHTS_COLLECTION views=\(initialCount)")
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
        }
        if let scanFor {
            // The form follows the frozen evidence, so the last member is
            // above the document's bottom. Walk bounded viewport steps rather
            // than querying a virtual accessibility tree for offscreen rows.
            for _ in 0..<100 {
                if contains(scanFor) { break }
                if let scroll, let document = scroll.documentView {
                    let maximum = max(0, document.bounds.height - scroll.contentView.bounds.height)
                    let next = min(maximum, scroll.contentView.bounds.minY + scroll.contentView.bounds.height * 0.75)
                    scroll.contentView.scroll(to: NSPoint(x: 0, y: next))
                    scroll.reflectScrolledClipView(scroll.contentView)
                }
                RunLoop.main.run(until: Date().addingTimeInterval(0.02))
                hosting.layoutSubtreeIfNeeded()
                window.displayIfNeeded()
            }
            XCTAssertTrue(contains(scanFor), "The final frozen member button must remain reachable and materialize")
            if let scroll {
                scroll.contentView.scroll(to: .zero)
                scroll.reflectScrolledClipView(scroll.contentView)
            }
            RunLoop.main.run(until: Date().addingTimeInterval(0.02))
            hosting.layoutSubtreeIfNeeded()
            if let scroll { XCTAssertEqual(scroll.contentView.bounds.minY, 0, accuracy: 1) }
            XCTAssertTrue(contains(first), "The first task member remains represented after returning to the top")
        }
        return initialCount
    }
}
