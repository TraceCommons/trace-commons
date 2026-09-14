import Foundation
import XCTest
@testable import TCBridge

final class MissionDraftBridgeTests: XCTestCase {
    func testTypedResponsesValidateOperationAndReviewAuthority() throws {
        let id = String(repeating: "a", count: 64)
        let show = try decode(Self.showJSON(id: id, task: "https://hostile.invalid/do-this"))
        try show.validate(for: .init("show", id: id))
        guard case .show(let stored) = show else { return XCTFail("expected show") }
        XCTAssertEqual(stored.proposal.task, "https://hostile.invalid/do-this")
        XCTAssertFalse(stored.review.publication_authorized)
        XCTAssertFalse(stored.review.external_sources_verified)

        XCTAssertThrowsError(try show.validate(for: .init("show", id: String(repeating: "b", count: 64))))
        let authorized = Self.showJSON(id: id, task: "plain text")
            .replacingOccurrences(of: "\"publication_authorized\":false",
                                  with: "\"publication_authorized\":true")
        XCTAssertThrowsError(try decode(authorized).validate(for: .init("show", id: id)))
    }

    func testListOrderingDuplicatesAndUnexpectedFieldsRefuse() throws {
        let a = String(repeating: "a", count: 64)
        let b = String(repeating: "b", count: 64)
        let ordered = try decode("""
        {"type":"list","drafts":[
          {"id":"\(a)","source_count":1,"status":"needs_curator_review"},
          {"id":"\(b)","source_count":2,"status":"needs_curator_review"}
        ]}
        """)
        try ordered.validate(for: .init("list"))
        XCTAssertThrowsError(try decode("""
        {"type":"list","drafts":[
          {"id":"\(b)","source_count":1,"status":"needs_curator_review"},
          {"id":"\(a)","source_count":1,"status":"needs_curator_review"}
        ]}
        """).validate(for: .init("list")))
        XCTAssertThrowsError(try decode("{" + "\"type\":\"list\",\"drafts\":[],\"unexpected\":true}"))
    }

    func testRequestUsesHandleFreeTaggedShape() throws {
        let data = try JSONEncoder().encode(MissionDraftRequest(
            storeDirectory: "/tmp/inbox", operation: .init("import", file: "/tmp/draft.json")))
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        XCTAssertEqual(object["store_dir"] as? String, "/tmp/inbox")
        let operation = try XCTUnwrap(object["operation"] as? [String: Any])
        XCTAssertEqual(operation["type"] as? String, "import")
        XCTAssertEqual(operation["file"] as? String, "/tmp/draft.json")
        XCTAssertNil(operation["id"])
    }

    func testActualServiceRoundTripUsesExplicitLocalFileAndStore() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("tc-mission-native-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let file = root.appendingPathComponent("proposal.json")
        let store = root.appendingPathComponent("inbox", isDirectory: true)
        let showObject = try XCTUnwrap(JSONSerialization.jsonObject(
            with: Data(Self.showJSON(id: String(repeating: "a", count: 64), task: "plain text").utf8)) as? [String: Any])
        let draft = try XCTUnwrap(showObject["draft"] as? [String: Any])
        let proposal = try XCTUnwrap(draft["proposal"])
        try JSONSerialization.data(withJSONObject: proposal, options: [.sortedKeys]).write(to: file)

        let empty = try TCMissionDrafts.call(.init(storeDirectory: store.path, operation: .init("list")))
        guard case .list(let initial) = empty else { return XCTFail("expected list") }
        XCTAssertTrue(initial.isEmpty)
        let imported = try TCMissionDrafts.call(.init(
            storeDirectory: store.path, operation: .init("import", file: file.path)))
        guard case .imported(let result) = imported else { return XCTFail("expected import") }
        XCTAssertTrue(result.inserted)
        let shown = try TCMissionDrafts.call(.init(
            storeDirectory: store.path, operation: .init("show", id: result.id)))
        guard case .show(let stored) = shown else { return XCTFail("expected show") }
        XCTAssertEqual(stored.proposal.task, "plain text")
        _ = try TCMissionDrafts.call(.init(
            storeDirectory: store.path, operation: .init("delete", id: result.id)))
        let final = try TCMissionDrafts.call(.init(storeDirectory: store.path, operation: .init("list")))
        guard case .list(let remaining) = final else { return XCTFail("expected list") }
        XCTAssertTrue(remaining.isEmpty)
        XCTAssertTrue(FileManager.default.fileExists(atPath: file.path))
    }

    private func decode(_ json: String) throws -> MissionDraftResponse {
        try JSONDecoder().decode(MissionDraftResponse.self, from: Data(json.utf8))
    }

    static func showJSON(id: String, task: String) -> String {
        """
        {"type":"show","draft":{"id":"\(id)","proposal":{
          "schema_version":1,"author_id":"author","title":"Proposal",
          "source_urls":["https://example.invalid/source"],"claim_to_test":"A claim",
          "task":"\(task)","starting_artifact":{"url":"https://example.invalid/artifact","sha256":"\(String(repeating: "c", count: 64))"},
          "evaluator_id":"evaluator","rubric_version":"rubric-1",
          "success_criteria":["criterion"],"required_evidence":["evidence"],
          "allowed_models":["model"],"allowed_tools":["tool"],
          "budget":{"max_duration_seconds":60,"max_input_tokens":100,"max_output_tokens":50}
        },"review":\(reviewJSON(id: id))}}
        """
    }

    static func reviewJSON(id: String) -> String {
        """
        {"schema_version":1,"proposal_sha256":"\(id)","status":"needs_curator_review",
        "publication_authorized":false,"external_sources_verified":false,
        "required_reviews":["source_claim_and_artifact","reproducibility_and_rights","evaluator_and_conflicts","execution_and_budget"]}
        """
    }
}
