import Foundation
import XCTest
import SwiftUI
import AppKit
import TCBridge
@testable import TraceCommonsApp

final class InsightsModelTests: XCTestCase {
    @MainActor
    func testFreshNavigationDoesNotAdvanceContributionGates() {
        let trace = AppModel()
        let navigation = MainWindowNavigation()
        XCTAssertEqual(navigation.section, .insights)
        var starts = 0
        navigation.activateServicesIfNeeded { starts += 1 }
        XCTAssertEqual(starts, 0)
        XCTAssertTrue(navigation.displaysInsights)
        XCTAssertEqual(trace.startup, .starting)
        XCTAssertFalse(trace.status.loggedIn)
        XCTAssertFalse(trace.isOnboardingComplete)
        XCTAssertFalse(trace.traceNavigationReady)
        XCTAssertEqual(MainWindowView.Section.allCases.last, .settings)
        navigation.section = .queue
        navigation.activateServicesIfNeeded { starts += 1 }
        navigation.activateServicesIfNeeded { starts += 1 }
        XCTAssertEqual(starts, 1)
    }

    @MainActor
    func testClosingDiscardsLateResponseEvenWhenOperationIgnoresCancellation() async throws {
        let gate = ResponseGate()
        let model = InsightsModel(service: { request in try await gate.call(request) })
        model.open()
        await gate.waitForCall()
        XCTAssertTrue(model.busy)
        model.close()
        try await gate.resolve("{\"type\":\"copy\",\"copy\":{\"title\":\"late\"}}")
        for _ in 0..<20 { await Task.yield() }
        XCTAssertFalse(model.busy)
        XCTAssertTrue(model.copy.isEmpty)
        XCTAssertNil(model.error)
        XCTAssertTrue(model.snapshots.isEmpty)
    }

    @MainActor
    func testInitialCopyThenListUsesOnlyLocalService() async throws {
        let recorder = Recorder()
        let model = InsightsModel(service: { request in try await recorder.call(request) })
        model.open()
        for _ in 0..<500 where model.busy || model.episodeBusy { try await Task.sleep(for: .milliseconds(10)) }
        XCTAssertEqual(model.text("title"), "Insights")
        let operations = await recorder.operations
        XCTAssertEqual(Set(operations), ["copy", "episode_list", "list", "summary"])
        XCTAssertEqual(model.summary?.saved_snapshots, 0)
        XCTAssertTrue(model.snapshots.isEmpty)
        model.close()
    }
}

private actor ResponseGate {
    private var continuation: CheckedContinuation<InsightsResponse, Error>?
    func call(_ request: InsightsRequest) async throws -> InsightsResponse {
        try await withCheckedThrowingContinuation { continuation = $0 }
    }
    func waitForCall() async {
        while continuation == nil { await Task.yield() }
    }
    func resolve(_ json: String) throws {
        continuation?.resume(returning: try JSONDecoder().decode(InsightsResponse.self, from: Data(json.utf8)))
        continuation = nil
    }
}
private actor Recorder {
    var operations: [String] = []
    func call(_ request: InsightsRequest) throws -> InsightsResponse {
        operations.append(request.operation.type)
        if request.operation.type == "summary" {
            return try TCInsights.call(.init(
                storeDirectory: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString).path,
                operation: .init("summary")))
        }
        let json = switch request.operation.type {
        case "copy": "{\"type\":\"copy\",\"copy\":{\"title\":\"Insights\"}}"
        case "episode_list": "{\"type\":\"episode_list\",\"episodes\":[]}"
        default: "{\"type\":\"list\",\"insights\":[]}"
        }
        return try JSONDecoder().decode(InsightsResponse.self, from: Data(json.utf8))
    }
}

extension InsightsModelTests {
    @MainActor
    func testActualModelKeepsAnalysisEphemeralAndSavesOnlyOnAction() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = root.appendingPathComponent("insights")
        let file = root.appendingPathComponent("fixture.jsonl")
        try Data("{\"role\":\"meta\",\"source\":\"claude-code\",\"model\":\"fixture\"}\n{\"role\":\"user\",\"timestamp\":\"2026-09-11T12:00:00Z\",\"content\":\"hello\"}\n".utf8).write(to: file)
        let model = InsightsModel(service: { request in
            try await Task.detached {
                try TCInsights.call(.init(storeDirectory: store.path, operation: request.operation))
            }.value
        })
        func settle() async throws {
            for _ in 0..<500 {
                if !model.busy && !model.episodeBusy { return }
                try await Task.sleep(for: .milliseconds(10))
            }
            XCTFail("Local operation did not finish")
        }
        model.open(); try await settle()
        XCTAssertEqual(model.summary?.saved_snapshots, 0)
        XCTAssertFalse(FileManager.default.fileExists(atPath: store.path))
        model.analyze(file: file, source: "trajectory"); try await settle()
        XCTAssertNotNil(model.selected)
        XCTAssertFalse(model.selectedIsSaved)
        XCTAssertFalse(FileManager.default.fileExists(atPath: store.path))
        model.save(); try await settle()
        XCTAssertTrue(model.selectedIsSaved)
        XCTAssertEqual(model.snapshots.count, 1)
        XCTAssertEqual(model.summary?.saved_snapshots, 1)
        XCTAssertEqual(model.summary?.user_reported.unassessed_snapshots, 1)
        model.annotate(category: "docs", outcome: "accepted"); try await settle()
        XCTAssertEqual(model.selected?.manual_annotation?.provenance, "user_reported")
        XCTAssertEqual(model.assessmentCategory, "docs")
        XCTAssertEqual(model.assessmentOutcome, "accepted")
        XCTAssertEqual(model.summary?.user_reported.assessed_snapshots, 1)
        XCTAssertEqual(model.summary?.user_reported.unassessed_snapshots, 0)
        XCTAssertEqual(model.summary?.user_reported.outcomes.first { $0.outcome == "accepted" }?.snapshots, 1)
        let savedID = try XCTUnwrap(model.selected?.id)
        model.analyze(file: file, source: "trajectory"); try await settle()
        XCTAssertEqual(model.assessmentCategory, "unknown")
        XCTAssertEqual(model.assessmentOutcome, "unknown")
        model.explain(savedID); try await settle()
        XCTAssertEqual(model.assessmentCategory, "docs", "Explain must populate the editable assessment")
        XCTAssertEqual(model.assessmentOutcome, "accepted")
        let detail = InsightDetail(insight: try XCTUnwrap(model.selected), copy: model.copy)
        let renderer = ImageRenderer(content: detail.padding(20).frame(width: 700).background(Color.white))
        let image = try XCTUnwrap(renderer.nsImage)
        XCTAssertGreaterThan(image.size.height, 100)
        if let path = ProcessInfo.processInfo.environment["TC_INSIGHTS_RENDER_PATH"] {
            let bitmap = try XCTUnwrap(NSBitmapImageRep(data: XCTUnwrap(image.tiffRepresentation)))
            try XCTUnwrap(bitmap.representation(using: .png, properties: [:])).write(to: URL(fileURLWithPath: path))
        }
        _ = try TCInsights.call(.init(storeDirectory: store.path,
            operation: .init("clear_annotation", id: try XCTUnwrap(model.selected?.id))))
        model.refresh(); try await settle()
        XCTAssertNil(model.selected?.manual_annotation, "Refresh must replace the selected saved detail after external edits")
        XCTAssertEqual(model.assessmentCategory, "unknown")
        XCTAssertEqual(model.assessmentOutcome, "unknown")
        model.annotate(category: "tests", outcome: "partial"); try await settle()
        model.clearAnnotation(); try await settle()
        XCTAssertEqual(model.summary?.user_reported.unassessed_snapshots, 1)
        XCTAssertEqual(model.assessmentCategory, "unknown")
        XCTAssertEqual(model.assessmentOutcome, "unknown")
        let oldID = model.selected?.id
        let original = try String(contentsOf: file, encoding: .utf8)
        try Data(original.replacingOccurrences(of: "hello", with: "changed").utf8).write(to: file)
        model.analyze(file: file, source: "trajectory"); try await settle()
        model.save(); try await settle()
        XCTAssertEqual(model.snapshots.count, 1, "Reimport must remove the replaced cached snapshot")
        XCTAssertNotEqual(model.selected?.id, oldID)
        XCTAssertNil(model.selected?.manual_annotation)
        model.delete(); try await settle()
        XCTAssertNil(model.selected)
        XCTAssertTrue(model.snapshots.isEmpty)
        XCTAssertEqual(model.summary?.saved_snapshots, 0)
        XCTAssertNil(model.summary?.snapshot_analysis_range)
        XCTAssertTrue(FileManager.default.fileExists(atPath: file.path))
        model.close()
    }
}
