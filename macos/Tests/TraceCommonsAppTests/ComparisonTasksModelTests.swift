import Foundation
import XCTest
import TCBridge
@testable import TraceCommonsApp

final class ComparisonTasksModelTests: XCTestCase {
    @MainActor
    func testCreateOutcomeReconfirmReplaceAndDeleteUseCASWithoutRetries() async throws {
        let service = ComparisonFakeService()
        let model = ComparisonTasksModel(service: { try await service.call($0) })
        model.open(); try await settle(model)
        model.createSelection = [ComparisonFakeService.episodeID]; model.create(); try await settle(model)
        XCTAssertEqual(model.detail?.task.revision, 1)
        model.outcome = .accepted; model.setOutcome(); try await settle(model)
        XCTAssertEqual(model.detail?.task.outcome?.value, .accepted)
        model.reconfirm(try XCTUnwrap(model.reconfirmation())); try await settle(model)
        XCTAssertNotNil(model.detail?.task.independence_confirmation)
        model.beginEpisodeEdit(); model.replaceEpisodes(); try await settle(model)
        model.delete(try XCTUnwrap(model.deletion())); try await settle(model)
        XCTAssertTrue(model.tasks.isEmpty)
        let mutations = await service.mutations
        XCTAssertEqual(mutations, ["comparison_task_create", "comparison_task_set_outcome",
                                   "comparison_task_reconfirm", "comparison_task_replace_episodes",
                                   "comparison_task_delete"])
    }

    @MainActor
    func testCommittedMutationNoticeSurvivesFailedReload() async throws {
        let service = ComparisonFakeService(seed: true)
        let model = ComparisonTasksModel(service: { try await service.call($0) })
        model.open(); try await settle(model); model.select(ComparisonFakeService.taskID); try await settle(model)
        await service.failNextList(); model.setOutcome(); try await settle(model)
        XCTAssertNil(model.detail)
        XCTAssertEqual(model.notice, "comparison_task_set_outcome")
        XCTAssertEqual(model.error, "comparison_task_committed_reload_failed")
        let outcomeWrites = await service.count("comparison_task_set_outcome")
        XCTAssertEqual(outcomeWrites, 1)
    }

    @MainActor
    func testStaleConfirmationCannotActOnReopenedPresentation() async throws {
        let service = ComparisonFakeService(seed: true)
        let model = ComparisonTasksModel(service: { try await service.call($0) })
        model.open(); try await settle(model); model.select(ComparisonFakeService.taskID); try await settle(model)
        let stale = try XCTUnwrap(model.deletion())
        model.closeDetail(); model.select(ComparisonFakeService.taskID); try await settle(model)
        model.delete(stale); try await Task.sleep(for: .milliseconds(20))
        XCTAssertNotNil(model.detail)
        let deletes = await service.count("comparison_task_delete")
        XCTAssertEqual(deletes, 0)
    }

    @MainActor private func settle(_ model: ComparisonTasksModel) async throws {
        for _ in 0..<300 { if !model.busy { return }; try await Task.sleep(for: .milliseconds(10)) }
        XCTFail("Comparison model did not settle")
    }
}

private actor ComparisonFakeService {
    static let taskID = "20c18c96-6093-49f5-bb6f-6092ef0630b9"
    static let episodeID = "d0c18c96-6093-49f5-bb6f-6092ef0630b9"
    private var value: [String: Any]?
    private var failList = false
    private(set) var mutations: [String] = []
    init(seed: Bool = false) { if seed { value = Self.makeTask() } }
    func failNextList() { failList = true }
    func count(_ type: String) -> Int { mutations.filter { $0 == type }.count }
    func call(_ request: InsightsRequest) throws -> InsightsResponse {
        let op = request.operation; var response: [String: Any] = ["type": op.type]
        switch op.type {
        case "comparison_task_list":
            if failList { failList = false; throw InsightsError.invalidResponse }
            response["tasks"] = value.map { [Self.detail($0)] } ?? []
        case "comparison_task_explain": response["detail"] = Self.detail(try present())
        case "comparison_task_create": mutations.append(op.type); value = Self.makeTask(); response["type"] = "comparison_task"; response["task"] = value
        case "comparison_task_set_outcome":
            mutations.append(op.type); try check(op); advance()
            let digest = value!["material_digest"]!
            value?["outcome"] = ["value": op.outcome!, "material_revision": 1,
                                  "material_digest": digest, "recorded_at": "2026-09-12T00:00:00Z"]
            response["type"] = "comparison_task"; response["task"] = value
        case "comparison_task_reconfirm":
            mutations.append(op.type); try check(op); advance()
            let digest = value!["material_digest"]!
            value?["independence_confirmation"] = ["material_revision": 1,
                "material_digest": digest, "confirmed_at": "2026-09-12T00:00:00Z"]
            response["type"] = "comparison_task"; response["task"] = value
        case "comparison_task_replace_episodes":
            mutations.append(op.type); try check(op); advance(); response["type"] = "comparison_task"; response["task"] = value
        case "comparison_task_delete":
            mutations.append(op.type); try check(op); response["type"] = "comparison_task_delete"; response["task"] = value; value = nil
        default: throw InsightsError.invalidResponse
        }
        return try JSONDecoder().decode(InsightsResponse.self,
            from: JSONSerialization.data(withJSONObject: response))
    }
    private func present() throws -> [String: Any] { guard let value else { throw InsightsError.invalidResponse }; return value }
    private func check(_ op: InsightsRequest.Operation) throws {
        guard op.expected_revision == value?["revision"] as? UInt64 else {
            throw InsightsError.service("insights_comparison_task_revision_conflict")
        }
    }
    private func advance() {
        let revision = ((value?["revision"] as? UInt64) ?? 0) + 1
        value?["revision"] = revision
    }
    private static func makeTask() -> [String: Any] {
        ["schema_version": 1, "id": taskID, "revision": UInt64(1), "material_revision": UInt64(1),
         "material_digest": String(repeating: "a", count: 64), "created_at": "2026-09-12T00:00:00Z",
         "updated_at": "2026-09-12T00:00:00Z",
         "episodes": [["episode_id": episodeID, "revision": UInt64(1), "membership_revision": UInt64(1),
                       "members_digest": String(repeating: "b", count: 64),
                       "members": [["snapshot_id": String(repeating: "c", count: 64),
                                    "source_digest": String(repeating: "d", count: 64)]]]],
         "context": NSNull(), "outcome": NSNull(), "independence_confirmation": NSNull()]
    }
    private static func detail(_ task: [String: Any]) -> [String: Any] {
        ["task": task, "stale_reasons": ["context_incomplete", "attribution_pending_qualification"],
         "overlapping_task_ids": [], "resolved_at": "2026-09-12T00:00:00Z"]
    }
}
