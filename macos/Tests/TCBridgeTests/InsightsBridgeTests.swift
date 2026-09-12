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
}
