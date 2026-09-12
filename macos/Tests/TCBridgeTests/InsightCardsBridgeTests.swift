import Foundation
import XCTest
@testable import TCBridge

final class InsightCardsBridgeTests: XCTestCase {
    func testActualServiceDecodesAllDeterministicCardsAndSharedCopy() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = root.appendingPathComponent("insights")
        let response = try TCInsights.call(.init(storeDirectory: store.path, operation: .init(
            "question_cards", snapshotIDs: [], episodeIDs: [], questions: InsightQuestion.allCases)))
        XCTAssertEqual(response.type, "question_cards")
        let result = try XCTUnwrap(response.result)
        try result.validateSupportedSchema(expectedQuestions: InsightQuestion.allCases)
        XCTAssertEqual(result.cards.map(\.question), InsightQuestion.allCases)
        XCTAssertFalse(try XCTUnwrap(response.text).isEmpty)
        let copy = try TCInsights.call(.init(storeDirectory: store.path, operation: .init("copy")))
        XCTAssertEqual(copy.copy?["card_question_recorded_activity"], "Recorded activity")
        XCTAssertEqual(copy.copy?["card_state_partial"], "Partial coverage")
    }

    func testTypedValidationRejectsWrongProviderAndMalformedRows() throws {
        let invalid = cardResponseJSONForBridge()
            .replacingOccurrences(of: "trace-commons-local", with: "untrusted")
        let response = try JSONDecoder().decode(InsightsResponse.self, from: Data(invalid.utf8))
        XCTAssertThrowsError(try XCTUnwrap(response.result).validateSupportedSchema(expectedQuestions: InsightQuestion.allCases))
    }
}

private func cardResponseJSONForBridge() -> String {
    let cards = InsightQuestion.allCases.map {
        "{\"question\":\"\($0.rawValue)\",\"metric_version\":\"\(bridgeMetricVersion($0))\",\"state\":\"unavailable\",\"rows\":[],\"coverage\":[],\"episode_denominator\":null,\"evidence_ids\":[],\"episode_ids\":[],\"limitations\":[],\"rows_omitted\":false}"
    }.joined(separator: ",")
    return "{\"type\":\"question_cards\",\"result\":{\"schema_version\":1,\"provider\":{\"id\":\"trace-commons-local\",\"version\":\"1\",\"rubric_version\":\"deterministic-question-cards-v1\",\"execution_mode\":\"local\",\"schema_version\":1},\"input_digest\":\"\(String(repeating: "a", count: 64))\",\"cards\":[\(cards)]},\"text\":\"text\"}"
}
private func bridgeMetricVersion(_ question: InsightQuestion) -> String {
    switch question {
    case .recordedActivity: "recorded-activity-v1"
    case .episodeOutcomes: "episode-outcomes-v1"
    case .observedModels: "observed-models-v1"
    case .estimatedCost: "estimated-cost-v1"
    }
}
