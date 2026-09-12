import Foundation
import XCTest
@testable import TCBridge

final class ComparisonTaskBridgeTests: XCTestCase {
    func testTypedContextEncodesUnknownAndKnownValues() throws {
        let context = ComparisonTaskContextInput(
            projectID: "20c18c96-6093-49f5-bb6f-6092ef0630b9", taskDate: "2026-09-12",
            language: .known("swift"),
            configuration: .init(harnessID: .known("codex"), harnessVersion: .known("1"),
                                 reasoningEffort: .high, toolPolicyID: .known("default"),
                                 toolPolicyVersion: .known("1"),
                                 promptTemplateDigest: .known(String(repeating: "f", count: 64))))
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
                     "comparison_task_open_current_episode", "comparison_task_open_frozen_snapshot",
                     "comparison_specification_preview", "comparison_specification_save",
                     "comparison_specification_evaluate", "comparison_specification_explain_result",
                     "comparison_specification_committed_reload_failed"]
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

        var malformedSpec = Self.specification()
        malformedSpec["cohort_labels"] = ["same", "same"]
        let specResponse = try JSONDecoder().decode(InsightsResponse.self, from: try JSONSerialization.data(
            withJSONObject: ["type": "comparison_specification", "specification": malformedSpec]))
        XCTAssertThrowsError(try XCTUnwrap(specResponse.specification).validateStructure())

        var malformedResult = Self.comparisonResult()
        malformedResult["included_task_ids"] = ["not-a-uuid"]
        let resultResponse = try JSONDecoder().decode(InsightsResponse.self, from: try JSONSerialization.data(
            withJSONObject: ["type": "comparison_result", "result": malformedResult]))
        XCTAssertThrowsError(try XCTUnwrap(resultResponse.comparisonResult).validateStructure())

        malformedResult = Self.comparisonResult()
        malformedResult["cohorts"] = [["cohort_label": "a", "included_tasks": UInt64.max,
            "outcomes": ["accepted": UInt64.max, "partial": 1, "rejected": 0, "pending": 0,
                         "unknown": 0, "unassessed": 0, "assessed": UInt64.max],
            "usage": ["tasks_with_observed_attributed_tokens": 0,
                      "tasks_without_observed_attributed_tokens": UInt64.max,
                      "observed_attributed_tokens": 0]]]
        let overflowing = try JSONDecoder().decode(InsightsResponse.self, from: try JSONSerialization.data(
            withJSONObject: ["type": "comparison_result", "result": malformedResult]))
        XCTAssertThrowsError(try XCTUnwrap(overflowing.comparisonResult).validateStructure())
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
        let context = ComparisonTaskContextInput(
            projectID: "20c18c96-6093-49f5-bb6f-6092ef0630b9", taskDate: "2026-09-12",
            language: .known("swift"),
            configuration: .init(harnessID: .known("codex"), harnessVersion: .unknown,
                                 reasoningEffort: .high, toolPolicyID: .unknown,
                                 toolPolicyVersion: .unknown, promptTemplateDigest: .unknown))
        let contextual = try TCInsights.call(.init(storeDirectory: store,
            operation: .init("comparison_task_set_context", id: created.id,
                             expectedRevision: try XCTUnwrap(pending.task?.revision), context: context)))
        let confirmed = try TCInsights.call(.init(storeDirectory: store,
            operation: .init("comparison_task_reconfirm", id: created.id,
                             expectedRevision: try XCTUnwrap(contextual.task?.revision),
                             displayedMaterialDigest: try XCTUnwrap(contextual.task?.material_digest))))
        let input = ComparisonSpecificationDraftInput(
            evidenceCutoff: "2026-09-12T00:00:00Z", cohortLabels: ["fixture-model", "other-model"],
            dateStart: "2026-09-12", dateEnd: "2026-09-12",
            stratum: .init(projectID: context.project_id, language: "swift",
                           configurationFingerprint: try XCTUnwrap(confirmed.task?.context?.configuration_fingerprint)))
        let preview = try TCInsights.call(.init(storeDirectory: store,
            operation: .init("comparison_preview_spec", input: input)))
        try XCTUnwrap(preview.specification).validateStructure()
        try XCTUnwrap(preview.comparisonResult).validateStructure(expectedSpecification: preview.specification)
        let savedSpec = try XCTUnwrap(TCInsights.call(.init(storeDirectory: store,
            operation: .init("comparison_save_spec", input: input))).specification)
        let evaluated = try XCTUnwrap(TCInsights.call(.init(storeDirectory: store,
            operation: .init("comparison_evaluate", id: savedSpec.id))).comparisonResult)
        try evaluated.validateStructure(expectedSpecification: savedSpec)
        let explainedResult = try XCTUnwrap(TCInsights.call(.init(storeDirectory: store,
            operation: .init("comparison_explain_result", specificationID: savedSpec.id,
                             auditDigest: evaluated.audit_digest))).comparisonResult)
        XCTAssertEqual(explainedResult.audit_digest, evaluated.audit_digest)
        let listed = try TCInsights.call(.init(storeDirectory: store,
            operation: .init("comparison_task_list")))
        XCTAssertEqual(listed.tasks?.map(\.id), [created.id])
        let deleted = try TCInsights.call(.init(storeDirectory: store,
            operation: .init("comparison_task_delete", id: created.id,
                             expectedRevision: try XCTUnwrap(confirmed.task?.revision))))
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
    static func specification() -> [String: Any] {
        ["schema_version": 1, "id": "20c18c96-6093-49f5-bb6f-6092ef0630b9",
         "provenance": "retrospective_user_specification", "created_at": "2026-09-12T00:00:00Z",
         "evidence_cutoff": "2026-09-13T00:00:00Z", "cutoff_task_evidence": [], "category": "refactor",
         "cohort_labels": ["a", "b"], "date_start": "2026-09-01", "date_end": "2026-09-12",
         "stratum": ["project_id": "30c18c96-6093-49f5-bb6f-6092ef0630b9", "language": "swift",
                     "configuration_fingerprint": String(repeating: "a", count: 64)],
         "task_rule": "one-user-confirmed-work-item-v1", "attempt_rule": "canonical-snapshot-union-v1",
         "outcome_rubric_version": "categorical-user-report-v1",
         "estimands": ["categorical-outcome-distribution-v1"], "estimator_state": "not_yet_calibrated",
         "specification_digest": String(repeating: "b", count: 64),
         "saved_record_digest": String(repeating: "c", count: 64)]
    }
    static func comparisonResult() -> [String: Any] {
        ["schema_version": 1, "specification_id": "20c18c96-6093-49f5-bb6f-6092ef0630b9",
         "specification_digest": String(repeating: "b", count: 64),
         "specification_record_digest": String(repeating: "c", count: 64),
         "audit_digest": String(repeating: "d", count: 64),
         "estimation_input_digest": String(repeating: "e", count: 64), "included_task_ids": [],
         "excluded_tasks": [], "cohorts": []]
    }
}
