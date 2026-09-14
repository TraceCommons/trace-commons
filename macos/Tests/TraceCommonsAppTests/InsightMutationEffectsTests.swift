import Foundation
import XCTest
import TCBridge
@testable import TraceCommonsApp

final class InsightMutationEffectsTests: XCTestCase {
    func testOlderResponsesDefaultToNoEffects() throws {
        let response = try JSONDecoder().decode(InsightsResponse.self, from: Data("{\"type\":\"delete\",\"deleted\":true}".utf8))
        XCTAssertTrue(response.invalidatedEpisodeIDs.isEmpty)
        let explicit = try JSONDecoder().decode(InsightsResponse.self, from: Data("{\"type\":\"analyze\",\"mutation_effects\":{\"invalidated_episode_ids\":[]}}".utf8))
        XCTAssertTrue(explicit.invalidatedEpisodeIDs.isEmpty)
    }

    @MainActor
    func testActualEpisodeDeletionEffectsFromLocalService() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = root.appendingPathComponent("store").path
        let source = root.appendingPathComponent("synthetic.json")
        try Data("[{\"role\":\"meta\",\"source\":\"fixture\",\"model\":\"fixture\"},{\"role\":\"user\",\"timestamp\":\"2026-01-01T00:00:00Z\",\"content\":\"synthetic\"}]".utf8).write(to: source)
        let model = InsightsModel(service: { request in
            try await Task.detached {
                try TCInsights.call(.init(storeDirectory: store, operation: request.operation))
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
        model.analyze(file: source, source: "trajectory"); try await settle()
        model.save(); try await settle()
        let id = try XCTUnwrap(model.selected?.id)
        let created = try TCInsights.call(.init(storeDirectory: store,
            operation: .init("episode_create", snapshotIDs: [id])))
        XCTAssertEqual(created.type, "episode_create")
        model.delete(); try await settle()
        XCTAssertNil(model.error)
        XCTAssertEqual(model.invalidatedEpisodeIDs.count, 1)
        XCTAssertNotNil(UUID(uuidString: try XCTUnwrap(model.invalidatedEpisodeIDs.first)))
        XCTAssertFalse(model.text("episode_invalidated_notice").isEmpty)
        XCTAssertTrue(model.snapshots.isEmpty)
        XCTAssertEqual(model.summary?.saved_snapshots, 0)
        XCTAssertTrue(FileManager.default.fileExists(atPath: source.path))
        model.close()
    }

    @MainActor
    func testEffectsSurviveAutomaticRefreshButClearOnNextActionFailureAndClose() async throws {
        let service = MutationEffectsService()
        let model = InsightsModel(service: { request in try await service.call(request) })
        func settle() async throws {
            for _ in 0..<500 {
                if !model.busy && !model.episodeBusy { return }
                try await Task.sleep(for: .milliseconds(10))
            }
            XCTFail("Operation did not finish")
        }
        model.open(); try await settle()
        model.analyze(file: URL(fileURLWithPath: "/synthetic"), source: "codex"); try await settle()
        XCTAssertTrue(model.invalidatedEpisodeIDs.isEmpty)
        model.save(); try await settle()
        XCTAssertEqual(model.invalidatedEpisodeIDs, ["removed-group"])
        XCTAssertNotNil(model.summary)
        let operations = await service.operations
        XCTAssertEqual(Array(operations.suffix(4)), ["analyze", "list", "summary", "episode_list"])
        model.refresh()
        XCTAssertTrue(model.invalidatedEpisodeIDs.isEmpty)
        try await settle()
        model.delete(); try await settle()
        XCTAssertEqual(model.invalidatedEpisodeIDs, ["removed-group"])
        model.close()
        XCTAssertTrue(model.invalidatedEpisodeIDs.isEmpty)
        model.open(); try await settle()
        model.explain("snapshot"); try await settle()
        await service.failNextSummary()
        model.delete(); try await settle()
        XCTAssertNotNil(model.error)
        XCTAssertEqual(model.invalidatedEpisodeIDs, ["removed-group"], "Confirmed removal remains visible beside a refresh failure")
        model.explain("snapshot"); try await settle()
        await service.failNextDelete()
        model.delete(); try await settle()
        XCTAssertNotNil(model.error)
        XCTAssertTrue(model.invalidatedEpisodeIDs.isEmpty, "A failed mutation must not report prior effects")
        model.close()
    }
}

private actor MutationEffectsService {
    var operations: [String] = []
    private var failSummary = false
    private var failDelete = false
    private let absentStore = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString).path
    func failNextSummary() { failSummary = true }
    func failNextDelete() { failDelete = true }
    func call(_ request: InsightsRequest) throws -> InsightsResponse {
        let operation = request.operation
        operations.append(operation.type)
        if operation.type == "delete", failDelete { failDelete = false; throw InsightsError.invalidResponse }
        if operation.type == "summary" {
            if failSummary { failSummary = false; throw InsightsError.invalidResponse }
            return try TCInsights.call(.init(storeDirectory: absentStore, operation: .init("summary")))
        }
        let snapshot: [String: Any] = [
            "id": "snapshot", "source_format": "codex", "boundary": "session_proxy",
            "analyzed_at": "2026-01-01T00:00:00Z", "cost_unavailable_reason": "unknown",
            "report": ["schema_version": 1, "provider": ["id": "local", "version": "1", "rubric_version": "1", "execution_mode": "local"], "metrics": [], "evidence": []] as [String: Any]
        ]
        var response: [String: Any] = ["type": operation.type]
        switch operation.type {
        case "copy": response["copy"] = ["error": "operation-failed", "episode_invalidated_notice": "groups removed"]
        case "list": response["insights"] = [snapshot]
        case "episode_list": response["episodes"] = []
        case "delete": response["deleted"] = true
        default: response["insight"] = snapshot
        }
        if operation.type == "delete" || operation.save == true {
            response["mutation_effects"] = ["invalidated_episode_ids": ["removed-group"]]
        }
        return try JSONDecoder().decode(InsightsResponse.self, from: JSONSerialization.data(withJSONObject: response))
    }
}
