import Foundation
import XCTest
import TCBridge
@testable import TraceCommonsApp

final class InsightsStoreRoutingIntegrationTests: XCTestCase {
    @MainActor
    func testActualServiceKeepsTheSavedListInSelectedStore() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let selectedStore = root.appendingPathComponent("selected")
        let otherStore = root.appendingPathComponent("other")
        let selected = try seedStore(selectedStore, marker: "selected")
        let other = try seedStore(otherStore, marker: "other")

        let router = InsightsServiceRouter(selection: .custom(selectedStore.path))
        let insights = InsightsModel(service: { try await router.call($0) })
        insights.open()
        for _ in 0..<500 where insights.snapshots.isEmpty { try await Task.sleep(for: .milliseconds(10)) }
        XCTAssertEqual(insights.snapshots.map(\.id), [selected])

        let otherList = try TCInsights.call(.init(storeDirectory: otherStore.path, operation: .init("list")))
        XCTAssertEqual(try XCTUnwrap(otherList.insights).map(\.id), [other])
        XCTAssertNotEqual(other, selected)
        insights.close()
    }

    func testActualReadOnlyCallsLeaveEmptySelectedStoreUntouched() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        let emptyStore = root.appendingPathComponent("empty")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let router = InsightsServiceRouter(selection: .custom(emptyStore.path))
        for type in ["list", "episode_list", "comparison_task_list", "comparison_list_specs"] {
            _ = try await router.call(.init(operation: .init(type)))
        }
        XCTAssertFalse(FileManager.default.fileExists(atPath: emptyStore.path))
    }

    /// Saves one analyzed trace to `store`; its snapshot ID.
    private func seedStore(_ store: URL, marker: String) throws -> String {
        let source = store.deletingLastPathComponent().appendingPathComponent("\(marker).json")
        let fixture = "[{\"role\":\"meta\",\"source\":\"fixture\",\"model\":\"\(marker)\"}," +
            "{\"role\":\"user\",\"timestamp\":\"2026-01-01T00:00:00Z\",\"content\":\"\(marker)\"}]"
        try Data(fixture.utf8).write(to: source)
        let analyzed = try TCInsights.call(.init(storeDirectory: store.path,
            operation: .init("analyze", source: "trajectory", file: source.path, save: true)))
        return try XCTUnwrap(analyzed.insight?.id)
    }
}
