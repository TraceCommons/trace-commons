import Foundation
import XCTest
@testable import TCBridge

final class ComparisonTaskBridgeTests: XCTestCase {
    func testTypedContextEncodesUnknownAndKnownValues() throws {
        let context = ComparisonTaskContextInput(
            projectID: "20c18c96-6093-49f5-bb6f-6092ef0630b9", taskDate: "2026-09-12",
            language: .known("swift"),
            configuration: .init(harnessID: .known("codex"), harnessVersion: .unknown,
                                 reasoningEffort: .high, toolPolicyID: .unknown,
                                 toolPolicyVersion: .unknown, promptTemplateDigest: .unknown))
        let request = InsightsRequest(operation: .init("comparison_task_set_context", id: "id",
                                                        expectedRevision: 2, context: context))
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(request)) as? [String: Any])
        let operation = try XCTUnwrap(object["operation"] as? [String: Any])
        let encoded = try XCTUnwrap(operation["context"] as? [String: Any])
        XCTAssertEqual((encoded["language"] as? [String: Any])?["state"] as? String, "known")
        XCTAssertEqual((encoded["checkout_provenance"] as? [String: Any])?["state"] as? String, "unavailable")
    }

    func testSharedCopyCoversStructuredComparisonReview() throws {
        let copy = try XCTUnwrap(TCInsights.call(.init(operation: .init("copy"))).copy)
        let fixed = ["comparison_task_revision", "comparison_task_resolved",
                     "comparison_task_frozen_evidence", "comparison_task_stale_none",
                     "comparison_task_advanced_evidence", "comparison_task_project",
                     "comparison_task_new_project", "comparison_task_current_missing",
                     "comparison_task_current_matches_frozen", "comparison_task_current_changed",
                     "comparison_task_open_current_episode", "comparison_task_open_frozen_snapshot"]
        XCTAssertTrue(fixed.allSatisfy { copy[$0]?.isEmpty == false })
        XCTAssertTrue(ComparisonReasoningEffort.allCases.allSatisfy {
            copy["comparison_task_reasoning_\($0.rawValue)"]?.isEmpty == false
        })
        XCTAssertTrue(ComparisonTaskStaleReason.allCases.allSatisfy {
            copy["comparison_task_stale_\($0.rawValue)"]?.isEmpty == false
        })
    }

    func testMutationEffectsExposeStaleComparisonTasks() throws {
        let response = try JSONDecoder().decode(InsightsResponse.self, from: Data("""
        {"type":"episode_delete","mutation_effects":{"invalidated_episode_ids":[],
        "stale_comparison_task_ids":["20c18c96-6093-49f5-bb6f-6092ef0630b9"],
        "stale_comparison_tasks":[]}}
        """.utf8))
        XCTAssertEqual(response.staleComparisonTaskIDs,
                       ["20c18c96-6093-49f5-bb6f-6092ef0630b9"])
    }

    func testMalformedTaskDataIsRejectedByValidation() throws {
        let response = try JSONDecoder().decode(InsightsResponse.self, from: Data(Self.response(
            type: "comparison_task", task: Self.task(id: "not-a-uuid")).utf8))
        XCTAssertThrowsError(try XCTUnwrap(response.task).validateSupportedSchema())

        var duplicate = Self.detail()
        duplicate["stale_reasons"] = ["context_incomplete", "context_incomplete"]
        let list = try JSONDecoder().decode(InsightsResponse.self, from: try JSONSerialization.data(
            withJSONObject: ["type": "comparison_task_list", "tasks": [duplicate]]))
        XCTAssertThrowsError(try XCTUnwrap(list.tasks?.first).validateSupportedSchema())
    }

    func testActualServiceComparisonTaskRoundTrip() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let source = root.appendingPathComponent("source.json")
        try Data("[{\"role\":\"meta\",\"source\":\"fixture\",\"model\":\"fixture\"},{\"role\":\"user\",\"timestamp\":\"2026-09-12T00:00:00Z\",\"content\":\"refactor task\"}]".utf8).write(to: source)
        let store = root.appendingPathComponent("store").path
        let saved = try TCInsights.call(.init(storeDirectory: store,
            operation: .init("analyze", source: "trajectory", file: source.path, save: true)))
        let snapshotID = try XCTUnwrap(saved.insight?.id)
        let episodeResponse = try TCInsights.call(.init(storeDirectory: store,
            operation: .init("episode_create", snapshotIDs: [snapshotID])))
        let episode = try XCTUnwrap(episodeResponse.episode)
        let createdResponse = try TCInsights.call(.init(storeDirectory: store,
            operation: .init("comparison_task_create", episodeIDs: [episode.id])))
        let created = try XCTUnwrap(createdResponse.task)
        try created.validateSupportedSchema()
        let explained = try TCInsights.call(.init(storeDirectory: store,
            operation: .init("comparison_task_explain", id: created.id)))
        try XCTUnwrap(explained.comparisonTaskDetail).validateSupportedSchema(expectedID: created.id)
        let pending = try TCInsights.call(.init(storeDirectory: store,
            operation: .init("comparison_task_set_outcome", id: created.id, outcome: "pending",
                             expectedRevision: created.revision)))
        XCTAssertEqual(pending.task?.outcome?.value, .pending)
        let listed = try TCInsights.call(.init(storeDirectory: store,
            operation: .init("comparison_task_list")))
        XCTAssertEqual(listed.tasks?.map(\.id), [created.id])
        let deleted = try TCInsights.call(.init(storeDirectory: store,
            operation: .init("comparison_task_delete", id: created.id,
                             expectedRevision: try XCTUnwrap(pending.task?.revision))))
        XCTAssertEqual(deleted.task?.id, created.id)
    }

    static func response(type: String, task: [String: Any]) -> String {
        let data = try! JSONSerialization.data(withJSONObject: ["type": type, "task": task])
        return String(decoding: data, as: UTF8.self)
    }
    static func task(id: String = "20c18c96-6093-49f5-bb6f-6092ef0630b9", revision: UInt64 = 1,
                     digest: String = String(repeating: "a", count: 64)) -> [String: Any] {
        ["schema_version": 1, "id": id, "revision": revision, "material_revision": 1,
         "material_digest": digest, "created_at": "2026-09-12T00:00:00Z",
         "updated_at": "2026-09-12T00:00:00Z",
         "episodes": [["episode_id": "d0c18c96-6093-49f5-bb6f-6092ef0630b9", "revision": 1,
                       "membership_revision": 1, "members_digest": String(repeating: "b", count: 64),
                       "members": [["snapshot_id": String(repeating: "c", count: 64),
                                    "source_digest": String(repeating: "d", count: 64)]]]],
         "context": NSNull(), "outcome": NSNull(), "independence_confirmation": NSNull()]
    }
    static func detail(task: [String: Any] = task()) -> [String: Any] {
        ["task": task, "stale_reasons": ["context_incomplete", "attribution_pending_qualification"],
         "overlapping_task_ids": [], "resolved_at": "2026-09-12T00:00:00Z"]
    }
}
