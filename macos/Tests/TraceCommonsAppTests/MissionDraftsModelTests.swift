import Foundation
import XCTest
import TCBridge
@testable import TraceCommonsApp

final class MissionDraftsModelTests: XCTestCase {
    @MainActor
    func testImportListShowAndDeleteLifecycle() async throws {
        let service = MissionDraftFakeService()
        let model = MissionDraftsModel(service: { try await service.call($0) })
        model.open(); try await settle(model)
        XCTAssertTrue(model.drafts.isEmpty)

        model.importFile(URL(fileURLWithPath: "/synthetic/draft.json")); try await settle(model)
        XCTAssertEqual(model.notice, "added copy")
        XCTAssertEqual(model.drafts.map(\.id), [MissionDraftFakeService.id])

        model.show(MissionDraftFakeService.id); try await settle(model)
        XCTAssertEqual(model.detail?.proposal.title, "Proposal")
        let confirmation = try XCTUnwrap(model.deleteConfirmation())
        model.delete(confirmation); try await settle(model)
        XCTAssertNil(model.detail)
        XCTAssertNil(model.selectedID)
        XCTAssertTrue(model.drafts.isEmpty)
        XCTAssertEqual(model.notice, "deleted copy")
    }

    @MainActor
    func testStaleShowCannotResurrectClosedDetail() async throws {
        let service = MissionDraftFakeService(seed: true)
        let model = MissionDraftsModel(service: { try await service.call($0) })
        model.open(); try await settle(model)
        await service.holdNextShow()
        model.show(MissionDraftFakeService.id)
        XCTAssertTrue(model.detailBusy)
        model.closeDetail()
        await service.releaseShow()
        try await Task.sleep(for: .milliseconds(30))
        XCTAssertNil(model.detail)
        XCTAssertNil(model.selectedID)
    }

    @MainActor
    func testFailedShowPreservesLastValidDetailAndSelection() async throws {
        let service = MissionDraftFakeService(seed: true)
        let model = MissionDraftsModel(service: { try await service.call($0) })
        model.open(); try await settle(model)
        model.show(MissionDraftFakeService.id); try await settle(model)
        await service.failNextShow()
        model.show(MissionDraftFakeService.id); try await settle(model)
        XCTAssertEqual(model.selectedID, MissionDraftFakeService.id)
        XCTAssertEqual(model.detail?.id, MissionDraftFakeService.id)
        XCTAssertEqual(model.error, "error copy")
    }

    @MainActor
    func testStaleDeleteConfirmationCannotDeleteReopenedDraft() async throws {
        let service = MissionDraftFakeService(seed: true)
        let model = MissionDraftsModel(service: { try await service.call($0) })
        model.open(); try await settle(model)
        model.show(MissionDraftFakeService.id); try await settle(model)
        let stale = try XCTUnwrap(model.deleteConfirmation())
        model.closeDetail()
        model.show(MissionDraftFakeService.id); try await settle(model)
        model.delete(stale)
        try await Task.sleep(for: .milliseconds(20))
        XCTAssertNotNil(model.detail)
        let deleteCalls = await service.deleteCalls
        XCTAssertEqual(deleteCalls, 0)
    }

    @MainActor
    func testRefreshFailurePreservesLastValidListAndShowsSharedError() async throws {
        let service = MissionDraftFakeService(seed: true)
        let model = MissionDraftsModel(service: { try await service.call($0) })
        model.open(); try await settle(model)
        await service.failNextList()
        model.refresh(); try await settle(model)
        XCTAssertEqual(model.drafts.map(\.id), [MissionDraftFakeService.id])
        XCTAssertEqual(model.error, "error copy")
    }

    @MainActor
    func testDeleteFailurePreservesPresentedDraftForRetry() async throws {
        let service = MissionDraftFakeService(seed: true)
        let model = MissionDraftsModel(service: { try await service.call($0) })
        model.open(); try await settle(model)
        model.show(MissionDraftFakeService.id); try await settle(model)
        let confirmation = try XCTUnwrap(model.deleteConfirmation())
        await service.failNextDelete()
        model.delete(confirmation); try await settle(model)
        XCTAssertEqual(model.detail?.id, MissionDraftFakeService.id)
        XCTAssertEqual(model.selectedID, MissionDraftFakeService.id)
        XCTAssertEqual(model.error, "error copy")
        XCTAssertNotNil(model.deleteConfirmation())
    }

    @MainActor
    func testMissionNavigationDoesNotActivateEnrollmentServices() async throws {
        let navigation = MainWindowNavigation()
        var starts = 0
        navigation.section = .missionDrafts
        navigation.activateServicesIfNeeded { starts += 1 }
        XCTAssertEqual(starts, 0)
        XCTAssertEqual(
            MainWindowView.title(.missionDrafts, compute: nil, privateInference: nil,
                                 missionCopy: ["title": "shared mission title"]),
            "shared mission title"
        )
    }

    @MainActor private func settle(_ model: MissionDraftsModel) async throws {
        for _ in 0..<500 {
            if !model.loading && !model.detailBusy && !model.mutationBusy {
                try await Task.sleep(for: .milliseconds(10))
                return
            }
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTFail("Mission draft model did not settle")
    }
}

private actor MissionDraftFakeService {
    static let id = String(repeating: "a", count: 64)
    private var seeded: Bool
    private var holdShow = false
    private var failShow = false
    private var showContinuation: CheckedContinuation<Void, Never>?
    private var failList = false
    private var failDelete = false
    private(set) var deleteCalls = 0

    init(seed: Bool = false) { seeded = seed }

    func holdNextShow() { holdShow = true }
    func failNextShow() { failShow = true }
    func releaseShow() { showContinuation?.resume(); showContinuation = nil }
    func failNextList() { failList = true }
    func failNextDelete() { failDelete = true }

    func call(_ request: MissionDraftRequest) async throws -> MissionDraftResponse {
        switch request.operation.type {
        case "copy":
            return try decode("""
            {"type":"copy","copy":{"title":"mission title","intro":"mission intro","added":"added copy",
            "duplicate":"duplicate copy","deleted":"deleted copy","error":"error copy"}}
            """)
        case "list":
            if failList { failList = false; throw MissionDraftBridgeError.service("list-failed") }
            return try decode(seeded ? Self.listJSON : "{\"type\":\"list\",\"drafts\":[]}")
        case "import":
            let inserted = !seeded
            seeded = true
            return try decode("""
            {"type":"import","draft":{"id":"\(Self.id)","review":\(Self.reviewJSON),"inserted":\(inserted)}}
            """)
        case "show":
            if holdShow {
                holdShow = false
                await withCheckedContinuation { showContinuation = $0 }
            }
            if failShow { failShow = false; throw MissionDraftBridgeError.service("show-failed") }
            guard seeded else { throw MissionDraftBridgeError.service("not-found") }
            return try decode(Self.showJSON)
        case "delete":
            deleteCalls += 1
            if failDelete { failDelete = false; throw MissionDraftBridgeError.service("delete-failed") }
            seeded = false
            return try decode("{\"type\":\"delete\",\"draft\":{\"id\":\"\(Self.id)\",\"deleted\":true}}")
        default:
            throw MissionDraftBridgeError.service("unsupported")
        }
    }

    private func decode(_ json: String) throws -> MissionDraftResponse {
        try JSONDecoder().decode(MissionDraftResponse.self, from: Data(json.utf8))
    }

    private static var listJSON: String {
        "{\"type\":\"list\",\"drafts\":[{\"id\":\"\(id)\",\"source_count\":1,\"status\":\"needs_curator_review\"}]}"
    }

    private static var reviewJSON: String {
        """
        {"schema_version":1,"proposal_sha256":"\(id)","status":"needs_curator_review",
        "publication_authorized":false,"external_sources_verified":false,
        "required_reviews":["source_claim_and_artifact","reproducibility_and_rights","evaluator_and_conflicts","execution_and_budget"]}
        """
    }

    private static var showJSON: String {
        """
        {"type":"show","draft":{"id":"\(id)","proposal":{
          "schema_version":1,"author_id":"author","title":"Proposal",
          "source_urls":["https://example.invalid/source"],"claim_to_test":"A claim",
          "task":"plain text","starting_artifact":{"url":"https://example.invalid/artifact","sha256":"\(String(repeating: "c", count: 64))"},
          "evaluator_id":"evaluator","rubric_version":"rubric-1",
          "success_criteria":["criterion"],"required_evidence":["evidence"],
          "allowed_models":["model"],"allowed_tools":["tool"],
          "budget":{"max_duration_seconds":60,"max_input_tokens":100,"max_output_tokens":50}
        },"review":\(reviewJSON)}}
        """
    }
}
