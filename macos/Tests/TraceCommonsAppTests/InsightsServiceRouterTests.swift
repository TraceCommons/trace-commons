import Foundation
import XCTest
import TCBridge
import TCShellCore
@testable import TraceCommonsApp

final class InsightsServiceRouterTests: XCTestCase {
    func testStatelessInsightsCopySuppliesStoreRoutingWording() throws {
        let copy = try XCTUnwrap(TCInsights.copy())
        XCTAssertEqual(copy["insights_store_title"], "Insights store")
        XCTAssertEqual(copy["insights_store_unavailable"], "Insights store unavailable")
        XCTAssertEqual(copy["insights_store_not_directory"],
                       "The selected Insights store path is not a directory.")
    }

    func testCustomStoreRoutesEveryInsightsOperationAndBalancesSourceScope() async throws {
        let record = RouterRecord()
        let router = InsightsServiceRouter(selection: .custom("/pilot/store"), nativeCall: { request in
            record.capture(request)
            return try Self.response(type: request.operation.type)
        }, startScope: { url in record.start(url); return true }, stopScope: { record.stop($0) })

        for operation in [InsightsRequest.Operation("list"), .init("episode_explain", id: "episode"),
                          .init("comparison_task_list"), .init("comparison_get_spec", id: "spec"),
                          .init("analyze", source: "codex", file: "/source/session.jsonl")] {
            _ = try await router.call(.init(operation: operation))
        }

        XCTAssertEqual(record.storeDirectories, Array(repeating: "/pilot/store", count: 5))
        XCTAssertEqual(record.operationTypes, ["list", "episode_explain", "comparison_task_list",
                                               "comparison_get_spec", "analyze"])
        XCTAssertEqual(record.startedPaths, ["/source/session.jsonl"])
        XCTAssertEqual(record.stoppedPaths, ["/source/session.jsonl"])
    }

    func testRefusedSelectionNeverCallsNativeService() async {
        let record = RouterRecord()
        let router = InsightsServiceRouter(selection: .refused(.relativePath), nativeCall: { request in
            record.capture(request)
            return try Self.response(type: request.operation.type)
        })
        do {
            _ = try await router.call(.init(operation: .init("list")))
            XCTFail("refused store selection reached the native service")
        } catch {
            XCTAssertEqual(error as? InsightsError, .invalidResponse)
        }
        XCTAssertTrue(record.operationTypes.isEmpty)
    }

    private static func response(type: String) throws -> InsightsResponse {
        try JSONDecoder().decode(InsightsResponse.self,
                                 from: JSONSerialization.data(withJSONObject: ["type": type]))
    }
}

private final class RouterRecord: @unchecked Sendable {
    private let lock = NSLock()
    private(set) var storeDirectories: [String] = []
    private(set) var operationTypes: [String] = []
    private(set) var startedPaths: [String] = []
    private(set) var stoppedPaths: [String] = []

    func capture(_ request: InsightsRequest) {
        lock.withLock {
            storeDirectories.append(request.store_dir ?? "")
            operationTypes.append(request.operation.type)
        }
    }
    func start(_ url: URL) { lock.withLock { startedPaths.append(url.path) } }
    func stop(_ url: URL) { lock.withLock { stoppedPaths.append(url.path) } }
}
