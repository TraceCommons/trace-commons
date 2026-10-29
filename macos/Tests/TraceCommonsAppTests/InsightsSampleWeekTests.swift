import TCBridge
import XCTest
@testable import TraceCommonsApp

/// The sample week (`TRACE_COMMONS_INSIGHTS_SAMPLE=1` in a dev build) holds
/// no saved traces, and no operation the window sends reaches the store but
/// the sample's reads.
final class InsightsSampleWeekTests: XCTestCase {
    func test_theSampleListsNoSavedTraces() throws {
        let response = try XCTUnwrap(try InsightsSampleWeek.response(.init("list")))
        XCTAssertEqual(response.insights?.count, 0)
    }

    func test_noSampleWriteReachesTheStore() async throws {
        XCTAssertEqual(InsightsSampleWeek.storeReads, ["copy", "card_inputs", "pattern_sessions"])
        let store = StoreRecorder()
        let service = InsightsSampleWeek.service(fallback: { request in try await store.call(request) })
        let writes: [InsightsRequest.Operation] = [
            .init("analyze", source: "claude_code", file: "/tmp/trace.jsonl", save: true),
            .init("analyze", source: "claude_code", file: "/tmp/trace.jsonl", save: false),
            .init("delete", id: "unknown"), .init("episode_create", snapshotIDs: ["x"]),
            .init("question_cards", snapshotIDs: [], questions: InsightQuestion.allCases),
            .init("goal_set"), .init("lever_feedback"), .init("lever_card", enabled: false),
        ]
        for operation in writes { _ = try? await service(.init(operation: operation)) }
        let reached = await store.operations
        XCTAssertEqual(reached, [], "no write reaches the store")
        do {
            _ = try await service(.init(operation: .init("episode_create", snapshotIDs: ["x"])))
            XCTFail("an operation the sample does not answer is refused")
        } catch { XCTAssertEqual(error as? InsightsError, .service(InsightsSampleWeek.refusal)) }
    }
}

private actor StoreRecorder {
    private(set) var operations: [String] = []
    private let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString).path
    func call(_ request: InsightsRequest) throws -> InsightsResponse {
        operations.append(request.operation.type)
        return try TCInsights.call(.init(storeDirectory: directory, operation: request.operation))
    }
}
