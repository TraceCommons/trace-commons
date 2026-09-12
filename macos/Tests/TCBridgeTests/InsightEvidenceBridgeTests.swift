import Foundation
import XCTest
@testable import TCBridge

final class InsightEvidenceBridgeTests: XCTestCase {
    private let legacy = """
    {"id":"snapshot","source_format":"codex","boundary":"local","analyzed_at":"2026-01-01T00:00:00Z","cost_unavailable_reason":"unknown","report":{"schema_version":1,"provider":{"id":"local","version":"1","rubric_version":"1","execution_mode":"local"},"metrics":[],"evidence":[]}}
    """
    func testLegacySnapshotDoesNotInventDeclarationsOrLinks() throws {
        let value = try JSONDecoder().decode(LocalInsight.self, from: Data(legacy.utf8))
        XCTAssertNil(value.model_observations)
        XCTAssertNil(value.outcome_links)
        try value.validateSupportedEvidence()
    }
    func testMixedDeclarationsAndMissingnessRemainTyped() throws {
        let json = """
        {"schema_version":1,"scope":"declared_metadata_only","source_format":"codex","source_digest":"abc","coordinates":"jsonl_physical_lines_one_based","record_count":8,"candidate_records":5,"valid_declarations":2,"missing_declarations":2,"invalid_declarations":1,"omitted_declarations":0,"model_labels_omitted":false,"mixed_declared_models":true,"declared_models":["fixture-a","fixture-b"],"declarations":[{"model":"fixture-a","record_index":1,"kind":"codex_session_metadata"},{"model":"fixture-b","record_index":8,"kind":"codex_turn_context"}]}
        """
        let value = try JSONDecoder().decode(InsightModelObservations.self, from: Data(json.utf8))
        XCTAssertTrue(value.mixed_declared_models)
        XCTAssertEqual(value.declarations.map(\.record_index), [1, 8])
        XCTAssertEqual(value.coordinates, .jsonlPhysicalLinesOneBased)
        XCTAssertEqual(value.missing_declarations, 2)
        XCTAssertEqual(value.invalid_declarations, 1)
        XCTAssertThrowsError(try JSONDecoder().decode(InsightModelObservations.self,
            from: Data(json.replacingOccurrences(of: "declared_metadata_only", with: "verified_identity").utf8)))
        var snapshot = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(legacy.utf8)) as? [String: Any])
        var observations = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any])
        observations["schema_version"] = 2
        snapshot["model_observations"] = observations
        let unsupported = try JSONDecoder().decode(LocalInsight.self, from: JSONSerialization.data(withJSONObject: snapshot))
        XCTAssertThrowsError(try unsupported.validateSupportedEvidence())
    }
    func testLinkRequestPreservesExplicitInputsAndSnapshotID() throws {
        let request = InsightsRequest(operation: .init("link_git", id: "snapshot", repository: "/selected/repo", commit: String(repeating: "a", count: 40)))
        let json = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(request)) as? [String: Any])
        let operation = try XCTUnwrap(json["operation"] as? [String: Any])
        XCTAssertEqual(operation["repository"] as? String, "/selected/repo")
        XCTAssertEqual(operation["id"] as? String, "snapshot")
        XCTAssertNil(operation["file"])
    }
}
