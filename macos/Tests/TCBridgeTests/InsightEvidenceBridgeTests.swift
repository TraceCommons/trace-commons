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
        XCTAssertNil(value.claude_task_attribution)
        try value.validateSupportedEvidence()
    }
    func testClaudeAttributionIsAdditiveAndSourceSpecific() throws {
        // Shared Rust-generated fixture: the attributed branch must contain real
        // evidence, even though this decoder only consumes its display envelope.
        let repository = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent()
            .deletingLastPathComponent().deletingLastPathComponent()
        let fixture = repository.appendingPathComponent(
            "crates/trace-commons-contributor/fixtures/insights/claude-task-attribution/native-agent-alpha-snapshot.json")
        var snapshot = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(contentsOf: fixture)) as? [String: Any])
        let current = try JSONDecoder().decode(LocalInsight.self, from: JSONSerialization.data(withJSONObject: snapshot))
        XCTAssertEqual(current.claude_task_attribution?.state.status, "attributed")
        try current.validateSupportedEvidence()
        XCTAssertEqual(current.claude_task_attribution?.declared_model, "claude-opus-5")
        XCTAssertEqual(current.claude_task_attribution?.record_count, 3)
        snapshot["source_format"] = "codex"
        let mismatched = try JSONDecoder().decode(LocalInsight.self, from: JSONSerialization.data(withJSONObject: snapshot))
        XCTAssertThrowsError(try mismatched.validateSupportedEvidence())
    }
    func testMixedDeclarationsAndMissingnessRemainTyped() throws {
        let json = """
        {"schema_version":1,"scope":"declared_metadata_only","source_format":"codex","source_digest":"abc","coordinates":"jsonl_physical_lines_one_based","record_count":8,"candidate_records":5,"valid_declarations":2,"missing_declarations":2,"invalid_declarations":1,"omitted_declarations":0,"model_labels_omitted":false,"mixed_declared_models":true,"declared_models":["fixture-a","fixture-b"],"declarations":[{"model":"fixture-a","record_index":1,"kind":"codex_session_metadata"},{"model":"fixture-b","record_index":8,"kind":"codex_turn_context"}],"contract":"legacy_declared_metadata_v1"}
        """
        let value = try JSONDecoder().decode(InsightModelObservations.self, from: Data(json.utf8))
        XCTAssertTrue(value.mixed_declared_models)
        XCTAssertEqual(value.declarations.map(\.record_index), [1, 8])
        XCTAssertEqual(value.coordinates, .jsonlPhysicalLinesOneBased)
        XCTAssertEqual(value.missing_declarations, 2)
        XCTAssertEqual(value.invalid_declarations, 1)
        XCTAssertThrowsError(try JSONDecoder().decode(InsightModelObservations.self,
            from: Data(json.replacingOccurrences(of: "declared_metadata_only", with: "verified_identity").utf8)))
    }

    /// Rust is the single validator of the coverage contract. This shell renders
    /// the verdicts it knows, refuses the one Rust rejected, and fails to decode
    /// a verdict from a newer build rather than guessing what it means. It
    /// deliberately no longer re-derives counters, ordering, kinds or labels.
    func testTheShellConsumesRustsCoverageVerdictInsteadOfRederivingIt() throws {
        let body = """
        {"schema_version":2,"scope":"declared_metadata_only","source_format":"codex","source_digest":"abc","coordinates":"jsonl_physical_lines_one_based","record_count":2,"candidate_records":0,"valid_declarations":0,"missing_declarations":0,"invalid_declarations":0,"omitted_declarations":0,"model_labels_omitted":false,"mixed_declared_models":false,"declared_models":[],"declarations":[],"contract":"CONTRACT"}
        """
        var snapshot = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(legacy.utf8)) as? [String: Any])
        for accepted in ["legacy_declared_metadata_v1", "codex_turn_context_v2", "claude_assistant_message_v3"] {
            let text = body.replacingOccurrences(of: "CONTRACT", with: accepted)
            snapshot["model_observations"] = try JSONSerialization.jsonObject(with: Data(text.utf8))
            let value = try JSONDecoder().decode(LocalInsight.self, from: JSONSerialization.data(withJSONObject: snapshot))
            try value.validateSupportedEvidence()
        }
        let refused = body.replacingOccurrences(of: "CONTRACT", with: "unsupported")
        snapshot["model_observations"] = try JSONSerialization.jsonObject(with: Data(refused.utf8))
        let rejected = try JSONDecoder().decode(LocalInsight.self, from: JSONSerialization.data(withJSONObject: snapshot))
        XCTAssertThrowsError(try rejected.validateSupportedEvidence())

        let newer = body.replacingOccurrences(of: "CONTRACT", with: "codex_turn_context_v4")
        snapshot["model_observations"] = try JSONSerialization.jsonObject(with: Data(newer.utf8))
        XCTAssertThrowsError(try JSONDecoder().decode(
            LocalInsight.self, from: JSONSerialization.data(withJSONObject: snapshot)))
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
