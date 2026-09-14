import Foundation
import XCTest
@testable import TCBridge

final class InsightsSummaryBridgeTests: XCTestCase {
    func testEmptyNativeSummaryDoesNotCreateLocalState() throws {
        let store = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        let response = try TCInsights.call(.init(storeDirectory: store.path, operation: .init("summary")))
        let summary = try XCTUnwrap(response.summary)
        try summary.validateSupportedSchema()
        XCTAssertEqual(response.type, "summary")
        XCTAssertEqual(summary.saved_snapshots, 0)
        XCTAssertEqual(summary.user_reported.unassessed_snapshots, 0)
        XCTAssertNil(summary.snapshot_analysis_range)
        XCTAssertEqual(summary.metrics.count, 7)
        XCTAssertTrue(summary.metrics.allSatisfy { $0.observed_value_sum == nil })
        XCTAssertEqual(Set(summary.limitations), Set(SavedInsightsSummary.Limitation.allCases))
        XCTAssertFalse(FileManager.default.fileExists(atPath: store.path))
        let copy = try XCTUnwrap(TCInsights.call(.init(operation: .init("copy"))).copy)
        for limitation in summary.limitations {
            XCTAssertNotNil(copy["summary_limitation_" + limitation.rawValue])
        }
        for metric in summary.metrics {
            XCTAssertNotNil(copy["summary_unit_" + metric.coverage_unit.rawValue])
        }
    }

    func testMetricDecodingPreservesKnownZeroUnknownAndPartialDenominators() throws {
        let json = """
        {"id":"tool_calls","observed_value_sum":0,"available_snapshots":1,
        "missing_snapshots":2,"record_coverage":{"observed":1,"total":9},
        "coverage_unit":"normalized_events","evidence_snapshot_ids":["snapshot-1"]}
        """
        let metric = try JSONDecoder().decode(SavedInsightsSummary.Metric.self, from: Data(json.utf8))
        XCTAssertEqual(metric.observed_value_sum, 0)
        XCTAssertEqual(metric.available_snapshots, 1)
        XCTAssertEqual(metric.missing_snapshots, 2)
        XCTAssertEqual(metric.record_coverage.observed, 1)
        XCTAssertEqual(metric.record_coverage.total, 9)
        XCTAssertEqual(metric.coverage_unit, .normalizedEvents)
        XCTAssertEqual(metric.evidence_snapshot_ids, ["snapshot-1"])
        let unknown = json.replacingOccurrences(of: "\"observed_value_sum\":0", with: "\"observed_value_sum\":null")
        XCTAssertNil(try JSONDecoder().decode(SavedInsightsSummary.Metric.self, from: Data(unknown.utf8)).observed_value_sum)
        XCTAssertThrowsError(try JSONDecoder().decode(SavedInsightsSummary.Metric.self,
            from: Data(json.replacingOccurrences(of: "normalized_events", with: "unknown_unit").utf8)))
    }
}
