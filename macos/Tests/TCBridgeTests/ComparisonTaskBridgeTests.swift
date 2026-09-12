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
