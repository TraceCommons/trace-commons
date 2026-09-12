import Foundation
import XCTest
import SwiftUI
import AppKit
import TCBridge
@testable import TraceCommonsApp

final class InsightEvidenceTests: XCTestCase {
    func testCommitInputRequiresFullLowercaseObjectID() {
        XCTAssertTrue(InsightEvidenceInput.isFullCommit(String(repeating: "a", count: 40)))
        XCTAssertTrue(InsightEvidenceInput.isFullCommit(String(repeating: "0", count: 64)))
        for value in ["HEAD", "abcdef", String(repeating: "A", count: 40), String(repeating: "g", count: 40)] {
            XCTAssertFalse(InsightEvidenceInput.isFullCommit(value))
        }
    }

    @MainActor
    func testSyntheticEvidenceLifecycleAndPickerSelectionBinding() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = root.appendingPathComponent("store")
        let source = root.appendingPathComponent("fixture.jsonl")
        try Data("{\"role\":\"meta\",\"model\":\"fixture-a\",\"source\":\"claude-code\"}\n{\"role\":\"user\",\"timestamp\":\"2026-01-01T00:00:00Z\",\"content\":\"synthetic\"}\n".utf8).write(to: source)
        let report = root.appendingPathComponent("report.json")
        try Data("{\"schema_version\":1,\"runner\":\"synthetic\",\"passed\":0,\"failed\":2,\"skipped\":1,\"observed_at\":\"2026-01-01T00:00:00Z\"}".utf8).write(to: report)
        let repository = root.appendingPathComponent("repo")
        try FileManager.default.createDirectory(at: repository, withIntermediateDirectories: true)
        _ = try git(["init", "--quiet"], in: repository)
        _ = try git(["-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "commit", "--allow-empty", "--quiet", "-m", "synthetic"], in: repository)
        let commit = try git(["rev-parse", "HEAD"], in: repository).trimmingCharacters(in: .whitespacesAndNewlines)
        let recorder = EvidenceRecorder(store: store.path)
        let model = InsightsModel(service: { request in try await recorder.call(request) })
        func settle() async throws {
            for _ in 0..<1000 {
                if !model.busy { return }
                try await Task.sleep(for: .milliseconds(10))
            }
            XCTFail("Operation did not finish")
        }
        model.open(); try await settle()
        model.analyze(file: source, source: "trajectory"); try await settle()
        XCTAssertNil(model.evidenceSelection())
        model.save(); try await settle()
        let id = try XCTUnwrap(model.selected?.id)
        let oldPicker = try XCTUnwrap(model.evidenceSelection())
        model.refresh(); try await settle()
        model.linkTestReport(selection: oldPicker, file: report)
        XCTAssertEqual(model.error, model.text("link_changed_selection"))
        XCTAssertTrue(model.selected?.outcome_links?.isEmpty == true)
        let gitSelection = try XCTUnwrap(model.evidenceSelection())
        model.linkGit(selection: gitSelection, repository: repository, commit: commit)
        XCTAssertNil(model.selected)
        try await settle()
        XCTAssertNil(model.error)
        XCTAssertEqual(model.selected?.id, id)
        XCTAssertEqual(model.snapshots.first?.outcome_links?.count, 1)
        XCTAssertEqual(model.summary?.saved_snapshots, 1)
        let gitLink = try XCTUnwrap(model.selected?.outcome_links?.first)
        guard case .gitCommit(let gitEvidence) = gitLink.evidence else { return XCTFail("Expected Git object") }
        XCTAssertEqual(gitEvidence.object_id, commit)
        XCTAssertEqual(gitEvidence.provenance, .inspectedLocalObject)
        model.linkTestReport(selection: try XCTUnwrap(model.evidenceSelection()), file: report)
        try await settle()
        let snapshot = try XCTUnwrap(model.selected)
        XCTAssertEqual(snapshot.outcome_links?.count, 2)
        let testLink = try XCTUnwrap(snapshot.outcome_links?.first { if case .testReport = $0.evidence { return true }; return false })
        guard case .testReport(let testEvidence) = testLink.evidence else { return XCTFail("Expected report") }
        XCTAssertEqual(testEvidence.passed, 0)
        XCTAssertEqual(testEvidence.failed, 2)
        XCTAssertNil(testEvidence.commit_id)
        XCTAssertEqual(testEvidence.provenance, .importedReport)
        XCTAssertEqual(snapshot.model_observations?.declared_models, ["fixture-a"])
        let operations = await recorder.operations
        XCTAssertEqual(Array(operations.suffix(3)), ["link_test_report", "list", "summary"])
        let render = VStack(alignment: .leading, spacing: 16) {
            InsightModelSection(observations: snapshot.model_observations, copy: model.copy)
            if let observations = snapshot.model_observations { InsightModelDetails(observations: observations, copy: model.copy) }
            InsightOutcomeDetails(link: gitLink, copy: model.copy)
            InsightOutcomeDetails(link: testLink, copy: model.copy)
        }.padding(20).frame(width: 740).background(Color.white)
        let renderer = ImageRenderer(content: render)
        let image = try XCTUnwrap(renderer.nsImage)
        XCTAssertGreaterThan(image.size.height, 400)
        if let path = ProcessInfo.processInfo.environment["TC_EVIDENCE_RENDER_PATH"] {
            let bitmap = try XCTUnwrap(NSBitmapImageRep(data: XCTUnwrap(image.tiffRepresentation)))
            try XCTUnwrap(bitmap.representation(using: .png, properties: [:])).write(to: URL(fileURLWithPath: path))
        }
        model.unlinkEvidence(selection: try XCTUnwrap(model.evidenceSelection()), evidenceID: testLink.id)
        try await settle()
        XCTAssertEqual(model.selected?.outcome_links?.count, 1)
        let closingPicker = try XCTUnwrap(model.evidenceSelection())
        model.close()
        model.linkTestReport(selection: closingPicker, file: report)
        XCTAssertNil(model.error)
        model.open(); try await settle()
        model.linkTestReport(selection: closingPicker, file: report)
        XCTAssertEqual(model.error, model.text("link_changed_selection"))
        model.linkTestReport(selection: try XCTUnwrap(model.evidenceSelection()), file: root.appendingPathComponent("missing.json"))
        try await settle()
        XCTAssertNotNil(model.error)
        XCTAssertNil(model.selected, "Failure must not retain an earlier successful association")
        XCTAssertNil(model.summary)
        XCTAssertTrue(FileManager.default.fileExists(atPath: report.path))
        model.explain(id); try await settle()
        await recorder.pauseNextLink()
        model.linkTestReport(selection: try XCTUnwrap(model.evidenceSelection()), file: report)
        await recorder.waitForPending()
        model.close()
        await recorder.release()
        for _ in 0..<30 { await Task.yield() }
        XCTAssertNil(model.selected, "A link finishing after close cannot repopulate detail")
        XCTAssertFalse(model.busy)
        XCTAssertNil(model.error)
    }

    private func git(_ arguments: [String], in repository: URL) throws -> String {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/git")
        process.arguments = ["-c", "core.hooksPath=/dev/null"] + arguments
        process.currentDirectoryURL = repository
        process.environment = ["PATH": "/usr/bin:/bin", "GIT_CONFIG_GLOBAL": "/dev/null", "GIT_CONFIG_NOSYSTEM": "1"]
        let pipe = Pipe()
        process.standardOutput = pipe
        process.standardError = FileHandle.nullDevice
        try process.run()
        let output = pipe.fileHandleForReading.readDataToEndOfFile()
        process.waitUntilExit()
        XCTAssertEqual(process.terminationStatus, 0)
        return String(decoding: output, as: UTF8.self)
    }
}

private actor EvidenceRecorder {
    let store: String
    var operations: [String] = []
    init(store: String) { self.store = store }
    private var pauseLink = false
    private var pending: CheckedContinuation<Void, Never>?
    func pauseNextLink() { pauseLink = true }
    func waitForPending() async { while pending == nil { await Task.yield() } }
    func release() { pending?.resume(); pending = nil }
    func call(_ request: InsightsRequest) async throws -> InsightsResponse {
        operations.append(request.operation.type)
        let response = try TCInsights.call(.init(storeDirectory: store, operation: request.operation))
        if pauseLink, request.operation.type == "link_test_report" {
            pauseLink = false
            await withCheckedContinuation { pending = $0 }
        }
        return response
    }
}
