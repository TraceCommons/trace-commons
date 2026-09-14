import Foundation
import XCTest
@testable import TCBridge

final class InsightsBridgeTests: XCTestCase {
    func testLocalSnapshotLifecycleWithoutEnrollment() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = root.appendingPathComponent("insights")
        let file = root.appendingPathComponent("fixture.jsonl")
        let content = """
        {"role":"meta","source":"claude-code","model":"fixture"}
        {"role":"user","timestamp":"2026-09-11T12:00:00Z","content":"PRIVATE_BODY"}
        """
        try Data(content.utf8).write(to: file)
        func call(_ operation: InsightsRequest.Operation) throws -> InsightsResponse {
            try TCInsights.call(.init(storeDirectory: store.path, operation: operation))
        }
        XCTAssertEqual(try call(.init("copy")).copy?["title"], "Insights")
        XCTAssertEqual(try call(.init("list")).insights?.count, 0)
        XCTAssertFalse(FileManager.default.fileExists(atPath: store.path))
        let value = try XCTUnwrap(call(.init("analyze", source: "trajectory", file: file.path, save: false)).insight)
        XCTAssertFalse(FileManager.default.fileExists(atPath: store.path))
        XCTAssertNil(value.estimated_cost_usd)
        XCTAssertNil(value.report.metrics.first { $0.id == "input_tokens" }?.value)
        XCTAssertFalse(value.report.evidence.isEmpty)
        XCTAssertEqual(value.report.provider.execution_mode, "local")
        _ = try call(.init("analyze", source: "trajectory", file: file.path, save: true))
        XCTAssertEqual(try call(.init("list")).insights?.count, 1)
        XCTAssertEqual(try call(.init("explain", id: value.id)).insight?.id, value.id)
        let annotated = try call(.init("annotate", id: value.id, category: "docs", outcome: "partial"))
        XCTAssertEqual(annotated.insight?.manual_annotation?.provenance, "user_reported")
        XCTAssertNil(try call(.init("clear_annotation", id: value.id)).insight?.manual_annotation)
        XCTAssertEqual(try call(.init("delete", id: value.id)).deleted, true)
        XCTAssertEqual(try String(contentsOf: file, encoding: .utf8), content)
        XCTAssertEqual(Set(try FileManager.default.contentsOfDirectory(atPath: root.path)), ["fixture.jsonl", "insights"])
    }
    func testBridgeRejectsMalformedOperationAndOversizedRequestWithoutEcho() {
        XCTAssertThrowsError(try TCInsights.call(.init(operation: .init("secret-invalid")))) { error in
            XCTAssertTrue(error is InsightsError)
            XCTAssertFalse(String(describing: error).contains("secret-invalid"))
        }
        XCTAssertThrowsError(try TCInsights.call(.init(operation: .init(String(repeating: "x", count: 70_000)))))
    }

    func testCodexModelSchemaTwoAnalyzeSaveReimportAndListLifecycle() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = root.appendingPathComponent("insights")
        let qualified = root.appendingPathComponent("qualified.jsonl")
        let absent = root.appendingPathComponent("absent.jsonl")
        try Data("""
        {"type":"session_meta","timestamp":"2026-09-11T00:00:00Z","payload":{"id":"fixture","model_provider":"openai"}}
        {"type":"turn_context","timestamp":"2026-09-11T00:00:01Z","payload":{"model":"fixture-model"}}
        {"type":"response_item","timestamp":"2026-09-11T00:00:02Z","payload":{"type":"message","role":"assistant","content":[]}}
        """.utf8).write(to: qualified)
        try Data("""
        {"type":"session_meta","timestamp":"2026-09-11T00:00:00Z","payload":{"id":"fixture-absent","model_provider":"openai","model":"ignored"}}
        {"type":"response_item","timestamp":"2026-09-11T00:00:01Z","payload":{"type":"message","role":"assistant","model":"ignored","content":[]}}
        """.utf8).write(to: absent)
        func call(_ operation: InsightsRequest.Operation) throws -> InsightsResponse {
            try TCInsights.call(.init(storeDirectory: store.path, operation: operation))
        }

        let preview = try XCTUnwrap(call(.init("analyze", source: "codex", file: qualified.path, save: false)).insight)
        XCTAssertEqual(preview.model_observations?.schema_version, 2)
        XCTAssertEqual(preview.model_observations?.candidate_records, 1)
        XCTAssertEqual(preview.model_observations?.declared_models, ["fixture-model"])
        let saved = try XCTUnwrap(call(.init("analyze", source: "codex", file: qualified.path, save: true)).insight)
        XCTAssertEqual(try call(.init("analyze", source: "codex", file: qualified.path, save: true)).insight?.id, saved.id)
        let absentSaved = try XCTUnwrap(call(.init("analyze", source: "codex", file: absent.path, save: true)).insight)
        XCTAssertEqual(absentSaved.model_observations?.schema_version, 2)
        XCTAssertEqual(absentSaved.model_observations?.candidate_records, 0)
        XCTAssertEqual(absentSaved.model_observations?.valid_declarations, 0)
        XCTAssertEqual(absentSaved.model_observations?.missing_declarations, 0)
        XCTAssertEqual(absentSaved.model_observations?.declared_models, [])
        let listed = try XCTUnwrap(call(.init("list")).insights)
        XCTAssertEqual(Set(listed.map(\.id)), Set([saved.id, absentSaved.id]))
        XCTAssertTrue(listed.allSatisfy { $0.model_observations?.schema_version == 2 })
    }

    func testClaudeAttributionSurvivesNativeAnalyzeSaveAndList() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = root.appendingPathComponent("insights")
        let repository = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent()
            .deletingLastPathComponent().deletingLastPathComponent()
        let file = repository.appendingPathComponent(
            "crates/trace-commons-contributor/fixtures/insights/claude-task-attribution/agent-alpha.jsonl")
        let original = try Data(contentsOf: file)
        func call(_ operation: InsightsRequest.Operation) throws -> InsightsResponse {
            try TCInsights.call(.init(storeDirectory: store.path, operation: operation))
        }
        let preview = try XCTUnwrap(call(.init("analyze", source: "claude_code", file: file.path, save: false)).insight)
        XCTAssertFalse(FileManager.default.fileExists(atPath: store.path))
        XCTAssertEqual(preview.claude_task_attribution?.state.status, "attributed")
        XCTAssertEqual(preview.claude_task_attribution?.declared_model, "claude-opus-5")
        XCTAssertEqual(preview.claude_task_attribution?.context_window_selector, "1m")
        let saved = try XCTUnwrap(call(.init("analyze", source: "claude_code", file: file.path, save: true)).insight)
        XCTAssertEqual(saved.id, preview.id)
        let listed = try XCTUnwrap(call(.init("list")).insights)
        XCTAssertEqual(listed.count, 1)
        XCTAssertEqual(listed.first?.claude_task_attribution?.source_digest,
                       preview.claude_task_attribution?.source_digest)
        XCTAssertEqual(listed.first?.claude_task_attribution?.state.status, "attributed")
        XCTAssertEqual(try call(.init("delete", id: saved.id)).deleted, true)
        XCTAssertEqual(try Data(contentsOf: file), original)
    }

}
