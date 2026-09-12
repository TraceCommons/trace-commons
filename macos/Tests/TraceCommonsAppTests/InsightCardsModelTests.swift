import Foundation
import XCTest
import TCBridge
@testable import TraceCommonsApp

final class InsightCardsModelTests: XCTestCase {
    @MainActor
    func testEmptySelectionRequestsNoEvidenceRatherThanFallingBackToAll() async throws {
        let service = CardResponseGate()
        let model = InsightsModel(service: { try await service.call($0) })
        model.open(); try await settle(model)
        model.generateCards()
        let operation = await service.waitForCardRequest()
        XCTAssertEqual(operation.snapshot_ids, [])
        XCTAssertEqual(operation.episode_ids, [])
        await service.resolveCard(cardResponseJSON())
        try await settle(model)
        XCTAssertNotNil(model.cardResult)
    }

    @MainActor
    func testCardRequestFreezesSortedExplicitSelectionsAndPublishesValidatedResult() async throws {
        let service = CardResponseGate()
        let model = InsightsModel(service: { try await service.call($0) })
        model.open(); try await settle(model)
        model.setCardSnapshot(String(repeating: "b", count: 64), selected: true)
        model.setCardSnapshot(String(repeating: "a", count: 64), selected: true)
        model.setCardEpisode("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb", selected: true)
        model.generateCards()
        let operation = await service.waitForCardRequest()
        XCTAssertEqual(operation.questions, InsightQuestion.allCases)
        XCTAssertEqual(operation.snapshot_ids, [String(repeating: "a", count: 64), String(repeating: "b", count: 64)])
        XCTAssertEqual(operation.episode_ids, ["bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"])
        await service.resolveCard(cardResponseJSON())
        try await settle(model)
        XCTAssertEqual(model.cardResult?.cards.map(\.question), InsightQuestion.allCases)
        XCTAssertEqual(model.cardText, "authoritative rendered text")
    }

    @MainActor
    func testSelectionChangeInvalidatesAndLateResponseCannotRestoreCards() async throws {
        let service = CardResponseGate()
        let model = InsightsModel(service: { try await service.call($0) })
        model.open(); try await settle(model)
        let id = String(repeating: "a", count: 64)
        model.setCardSnapshot(id, selected: true)
        model.generateCards(); _ = await service.waitForCardRequest()
        model.setCardSnapshot(id, selected: false)
        await service.resolveCard(cardResponseJSON())
        for _ in 0..<20 { await Task.yield() }
        XCTAssertNil(model.cardResult)
        XCTAssertNil(model.cardText)
        XCTAssertFalse(model.cardBusy)
    }

    @MainActor
    func testListRefreshRemovesDeletedSelectionAndInvalidatesPresentedCards() async throws {
        let service = CardResponseGate()
        let model = InsightsModel(service: { try await service.call($0) })
        model.open(); try await settle(model)
        let id = String(repeating: "a", count: 64)
        model.setCardSnapshot(id, selected: true)
        model.generateCards(); _ = await service.waitForCardRequest()
        await service.resolveCard(cardResponseJSON()); try await settle(model)
        XCTAssertNotNil(model.cardResult)
        model.refresh(); try await settle(model)
        XCTAssertTrue(model.cardSnapshotSelection.isEmpty)
        XCTAssertNil(model.cardResult)
    }

    @MainActor private func settle(_ model: InsightsModel) async throws {
        for _ in 0..<500 {
            if !model.busy && !model.episodeBusy && !model.cardBusy { return }
            try await Task.sleep(for: .milliseconds(5))
        }
        XCTFail("Insights operation did not settle")
    }
}

private actor CardResponseGate {
    private var request: InsightsRequest.Operation?
    private var continuation: CheckedContinuation<InsightsResponse, Error>?
    func call(_ request: InsightsRequest) async throws -> InsightsResponse {
        switch request.operation.type {
        case "copy": return try decode("{\"type\":\"copy\",\"copy\":{\"error\":\"Unavailable\"}}")
        case "list": return try decode("{\"type\":\"list\",\"insights\":[]}")
        case "summary": return try decode(emptySummaryJSON)
        case "episode_list": return try decode("{\"type\":\"episode_list\",\"episodes\":[]}")
        case "question_cards":
            self.request = request.operation
            return try await withCheckedThrowingContinuation { continuation = $0 }
        default: throw InsightsError.operationFailed
        }
    }
    func waitForCardRequest() async -> InsightsRequest.Operation {
        while request == nil { await Task.yield() }
        return request!
    }
    func resolveCard(_ json: String) {
        continuation?.resume(with: Result { try decode(json) }); continuation = nil
    }
    private func decode(_ json: String) throws -> InsightsResponse {
        try JSONDecoder().decode(InsightsResponse.self, from: Data(json.utf8))
    }
}

private let emptySummaryJSON = """
{"type":"summary","summary":{"schema_version":1,"scope":"all_saved_selected_session_snapshots","limitations":["selected_saved_sessions_are_not_verified_tasks","assessments_are_user_reported","observed_sums_require_both_coverages","analysis_dates_are_not_activity_time","source_formats_are_not_model_identity","no_model_rankings_time_savings_or_cost"],"provider":{"id":"trace-commons-local","version":"1","rubric_version":"v","execution_mode":"local"},"saved_snapshots":0,"snapshot_analysis_range":null,"user_reported":{"assessed_snapshots":0,"unassessed_snapshots":0,"categories":[],"outcomes":[]},"metrics":[],"snapshots":[]}}
"""
private func cardResponseJSON() -> String {
    let cards = InsightQuestion.allCases.map {
        "{\"question\":\"\($0.rawValue)\",\"metric_version\":\"\(metricVersion($0))\",\"state\":\"unavailable\",\"rows\":[],\"coverage\":[],\"episode_denominator\":null,\"evidence_ids\":[],\"episode_ids\":[],\"limitations\":[],\"rows_omitted\":false}"
    }.joined(separator: ",")
    return "{\"type\":\"question_cards\",\"result\":{\"schema_version\":1,\"provider\":{\"id\":\"trace-commons-local\",\"version\":\"1\",\"rubric_version\":\"deterministic-question-cards-v1\",\"execution_mode\":\"local\",\"schema_version\":1},\"input_digest\":\"\(String(repeating: "a", count: 64))\",\"cards\":[\(cards)]},\"text\":\"authoritative rendered text\"}"
}
private func metricVersion(_ question: InsightQuestion) -> String {
    switch question {
    case .recordedActivity: "recorded-activity-v1"
    case .episodeOutcomes: "episode-outcomes-v1"
    case .observedModels: "observed-models-v1"
    case .estimatedCost: "estimated-cost-v1"
    }
}
