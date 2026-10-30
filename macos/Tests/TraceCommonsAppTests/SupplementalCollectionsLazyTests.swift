import AppKit
import SwiftUI
import TCBridge
@testable import TCShellCore
import XCTest
@testable import TraceCommonsApp

/// Opt-in row probes measure viewport-bounded construction and final-row
/// reachability independently of OS-specific accessibility text hosts.
/// Native view bounds remain a separate regression check; this does not
/// measure focus latency or depend on accessibility text values.
@MainActor
final class SupplementalCollectionsLazyTests: XCTestCase {
    func testCertificateRowsAreViewportBounded() throws {
        func view(_ count: Int) throws -> some View {
            let entries = try (0..<count).map { index in
                try JSONDecoder().decode(QueueEntry.self, from: JSONSerialization.data(withJSONObject: [
                    "entry_id": "e-\(index)", "session_hash": "h-\(index)", "source": "claude-code",
                    "project_id": "p", "project_label": "Certificate \(index)", "size_bytes": 100,
                    "discovered_at": 0, "state": "pending", "attempts": 0, "holds_certificate": true
                ]))
            }
            return ScrollView { CertificateSection(entries: entries) }.environmentObject(AppModel())
        }
        // Fill the viewport and prefetch region before comparing scales.
        let small = measure(try view(50), collection: "supplementalCertificates")
        let large = measure(try view(500), collection: "supplementalCertificates", last: "e-499")
        XCTAssertLessThanOrEqual(large.rows, small.rows + 25)
        XCTAssertLessThanOrEqual(large.views, small.views + 100)
    }

    func testHistoryRowsInOneGroupAreViewportBounded() async throws { try await history(manyGroups: false) }
    func testHistoryGroupsAreViewportBounded() async throws { try await history(manyGroups: true) }

    private func history(manyGroups: Bool) async throws {
        func view(_ count: Int) async throws -> some View {
            let rows: [[String: Any]] = (0..<count).map { index in
                ["submission_id": "row-\(index)", "project_id": manyGroups ? "p-\(index)" : "p",
                 "project_label": "History \(index)", "status": "accepted", "source": "claude-code"]
            }
            let transport = SupplementalCollectionTransport(replies: ["list_history": try json(["history": rows])])
            let store = HomeStore(client: LiveDaemonClient(transport: transport))
            await store.load()
            XCTAssertEqual(store.history?.count, count)
            return TracesHistoryPage(store: store, statusLabel: { _ in "Accepted" }, selection: .constant(""), back: {})
                .environmentObject(AppModel())
        }
        let collection = manyGroups ? "supplementalHistoryGroups" : "supplementalHistoryRows"
        let small = measure(try await view(10), collection: collection)
        let large = measure(try await view(100), collection: collection, last: manyGroups ? "p-99" : "row-99")
        XCTAssertLessThanOrEqual(large.rows, small.rows + 25)
        XCTAssertLessThanOrEqual(large.views, small.views + 150)
    }

    func testInferenceCallsAreViewportBounded() async throws {
        func view(_ count: Int) async throws -> some View {
            let calls: [[String: Any]] = (0..<count).map { index in
                ["id": index, "at": "2026-09-30T09:00:00Z", "tool": "claude-code", "family": "anthropic",
                 "model": "Call \(index)", "route": "routed", "proof": "verified",
                 "cost": ["known": true, "priced_micros": 1000]]
            }
            let transport = SupplementalCollectionTransport(replies: ["inference_calls": try json([
                "readable": true, "window_hours": 24, "calls": calls, "next_cursor": NSNull()
            ])])
            let store = InferenceStore(client: LiveDaemonClient(transport: transport))
            await store.load()
            XCTAssertEqual(store.calls?.calls.count, count)
            let model = AppModel()
            model.setStartupForTesting(.running)
            return InferenceTabView(store: store).environmentObject(model)
        }
        // Saturate the viewport and SwiftUI prefetch region before comparing.
        let small = measure(try await view(25), collection: "supplementalCalls")
        let large = measure(try await view(50), collection: "supplementalCalls", last: "49")
        XCTAssertLessThanOrEqual(large.rows, small.rows + 25)
        XCTAssertLessThanOrEqual(large.views, small.views + 100)
    }

    private func json(_ object: Any) throws -> String {
        String(decoding: try JSONSerialization.data(withJSONObject: object), as: UTF8.self)
    }

    private func measure(_ content: some View, collection: String, last: String? = nil) -> (rows: Int, views: Int) {
        OtherCollectionRowBodies.startRecording()
        defer { OtherCollectionRowBodies.stopRecording() }
        _ = NSApplication.shared
        let previous = NSApp.value(forKey: "accessibilityEnhancedUserInterface")
        NSApp.setValue(true, forKey: "accessibilityEnhancedUserInterface")
        defer { NSApp.setValue(previous, forKey: "accessibilityEnhancedUserInterface") }
        let bounds = NSRect(x: 0, y: 0, width: 408, height: 760)
        let hosting = NSHostingView(rootView: content.textSelection(.enabled).frame(width: 408, height: 760))
        hosting.frame = bounds
        let window = NSWindow(contentRect: bounds, styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = hosting
        defer { window.close() }
        func descendants(_ view: NSView) -> [NSView] { [view] + view.subviews.flatMap(descendants) }
        hosting.layoutSubtreeIfNeeded()
        window.displayIfNeeded()
        _ = hosting.accessibilityChildren()
        hosting.layoutSubtreeIfNeeded()
        let views = descendants(hosting)
        let initial = OtherCollectionRowBodies.built[collection] ?? []
        print("SUPPLEMENTAL_COLLECTION \(collection) rows=\(initial.count) views=\(views.count)")
        XCTAssertFalse(initial.isEmpty, "The collection probe must observe materialized rows")
        if let last {
            XCTAssertFalse(initial.contains(last), "The final offscreen row must not be constructed before scrolling")
            let scroll = views.compactMap { $0 as? NSScrollView }.first
            XCTAssertNotNil(scroll)
            func found() -> Bool { OtherCollectionRowBodies.built[collection]?.contains(last) == true }
            let deadline = Date().addingTimeInterval(3)
            repeat {
                if let scroll, let document = scroll.documentView {
                    scroll.contentView.scroll(to: NSPoint(x: 0, y: max(0, document.bounds.height - scroll.contentView.bounds.height)))
                    scroll.reflectScrolledClipView(scroll.contentView)
                }
                RunLoop.main.run(until: Date().addingTimeInterval(0.01))
                hosting.layoutSubtreeIfNeeded()
                window.displayIfNeeded()
            } while !found() && Date() < deadline
            XCTAssertTrue(found(), "The final row must remain reachable: \(last)")
        }
        return (initial.count, views.count)
    }
}

private final class SupplementalCollectionTransport: DaemonTransport, @unchecked Sendable {
    let replies: [String: String]
    init(replies: [String: String]) { self.replies = replies }
    func call(_ method: String, params paramsJSON: String) -> String {
        if let reply = replies[method] ?? SampleDaemonData.reply(method, in: .normalDay) {
            return "{\"id\":0,\"result\":\(reply)}"
        }
        return #"{"id":0,"error":{"code":"bad_params","message":"unknown-method"}}"#
    }
}
