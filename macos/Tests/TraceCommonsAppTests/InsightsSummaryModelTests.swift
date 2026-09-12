import Foundation
import XCTest
import SwiftUI
import AppKit
import TCBridge
@testable import TraceCommonsApp

final class InsightsSummaryModelTests: XCTestCase {
    @MainActor
    func testFailedRefreshClearsPreviouslySuccessfulSummary() async throws {
        let service = SummaryService(failSecond: true)
        let model = InsightsModel(service: { try await service.call($0) })
        model.open()
        try await settle(model)
        XCTAssertNotNil(model.summary)
        model.refresh()
        XCTAssertNil(model.summary, "A pending refresh must not display the previous summary as current")
        try await settle(model)
        XCTAssertNil(model.summary)
        XCTAssertNotNil(model.summaryError)
        XCTAssertFalse(model.loadingSummary)
        model.close()
    }

    @MainActor
    func testClosingDiscardsSummaryThatFinishesAfterClose() async throws {
        let service = SummaryService(holdSummary: true)
        let model = InsightsModel(service: { try await service.call($0) })
        model.open()
        await service.waitForSummary()
        XCTAssertTrue(model.loadingSummary)
        model.close()
        try await service.finishSummary()
        for _ in 0..<20 { await Task.yield() }
        XCTAssertNil(model.summary)
        XCTAssertNil(model.summaryError)
        XCTAssertFalse(model.loadingSummary)
        XCTAssertFalse(model.busy)
    }

    func testDisplayFormattingKeepsZeroDistinctFromUnknown() {
        XCTAssertEqual(InsightsSummaryFormatting.value(0, unknown: "Unknown fixture"), 0.formatted())
        XCTAssertEqual(InsightsSummaryFormatting.value(nil, unknown: "Unknown fixture"), "Unknown fixture")
    }

    @MainActor
    func testPartialSummaryRendersAndEvidenceNavigationClearsMissingSelection() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = root.appendingPathComponent("insights")
        let first = root.appendingPathComponent("first.jsonl")
        let second = root.appendingPathComponent("second.jsonl")
        let fixture = """
        {"type":"session_meta","payload":{}}
        {"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"fixture-one"}]}}
        {"type":"event_msg","payload":{"type":"future_event"}}
        """
        try Data(fixture.utf8).write(to: first)
        try Data(fixture.replacingOccurrences(of: "fixture-one", with: "fixture-two").utf8).write(to: second)
        func local(_ operation: InsightsRequest.Operation) throws -> InsightsResponse {
            try TCInsights.call(.init(storeDirectory: store.path, operation: operation))
        }
        let one = try XCTUnwrap(local(.init("analyze", source: "codex", file: first.path, save: true)).insight)
        let two = try XCTUnwrap(local(.init("analyze", source: "codex", file: second.path, save: true)).insight)
        _ = try local(.init("annotate", id: one.id, category: "unknown", outcome: "unknown"))
        let model = InsightsModel(service: { request in
            try await Task.detached {
                try TCInsights.call(.init(storeDirectory: store.path, operation: request.operation))
            }.value
        })
        model.open(); try await settle(model)
        let summary = try XCTUnwrap(model.summary)
        XCTAssertEqual(summary.user_reported.unassessed_snapshots, 1)
        XCTAssertEqual(summary.user_reported.outcomes.first { $0.outcome == "unknown" }?.snapshots, 1)
        let calls = try XCTUnwrap(summary.metrics.first { $0.id == "tool_calls" })
        XCTAssertEqual(calls.observed_value_sum, 0)
        XCTAssertEqual(calls.available_snapshots, 2)
        XCTAssertLessThan(calls.record_coverage.observed, calls.record_coverage.total)
        XCTAssertNil(summary.metrics.first { $0.id == "input_tokens" }?.observed_value_sum)
        let renderer = ImageRenderer(content: InsightsSummaryView(summary: summary, copy: model.copy, openSnapshot: model.explain)
            .padding(20).frame(width: 740).background(Color.white))
        let image = try XCTUnwrap(renderer.nsImage)
        XCTAssertGreaterThan(image.size.height, 100)
        if let path = ProcessInfo.processInfo.environment["TC_INSIGHTS_SUMMARY_RENDER_PATH"] {
            let bitmap = try XCTUnwrap(NSBitmapImageRep(data: XCTUnwrap(image.tiffRepresentation)))
            try XCTUnwrap(bitmap.representation(using: .png, properties: [:])).write(to: URL(fileURLWithPath: path))
        }
        XCTAssertTrue(summary.snapshots.contains { $0.id == two.id })
        model.explain(two.id); try await settle(model)
        XCTAssertEqual(model.selected?.id, two.id)
        _ = try local(.init("delete", id: one.id))
        model.explain(one.id)
        XCTAssertNil(model.selected, "Evidence lookup cannot keep a previous snapshot selected")
        try await settle(model)
        XCTAssertNil(model.selected)
        XCTAssertNotNil(model.error)
        model.close()
    }

    @MainActor
    private func settle(_ model: InsightsModel) async throws {
        for _ in 0..<500 {
            if !model.busy { return }
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTFail("Summary operation did not settle")
    }
}

private actor SummaryService {
    let failSecond: Bool
    let holdSummary: Bool
    private var summaries = 0
    private var continuation: CheckedContinuation<InsightsResponse, Error>?
    private let store = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString).path
    init(failSecond: Bool = false, holdSummary: Bool = false) {
        self.failSecond = failSecond; self.holdSummary = holdSummary
    }
    func call(_ request: InsightsRequest) async throws -> InsightsResponse {
        if request.operation.type == "summary" {
            summaries += 1
            if failSecond && summaries > 1 { throw InsightsError.operationFailed }
            if holdSummary {
                return try await withCheckedThrowingContinuation { continuation = $0 }
            }
        }
        return try TCInsights.call(.init(storeDirectory: store, operation: request.operation))
    }
    func waitForSummary() async {
        while continuation == nil { await Task.yield() }
    }
    func finishSummary() throws {
        let response = try TCInsights.call(.init(storeDirectory: store, operation: .init("summary")))
        continuation?.resume(returning: response)
        continuation = nil
    }
}
