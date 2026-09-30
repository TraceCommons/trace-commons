import Foundation
import AppKit
import SwiftUI
import XCTest
import TCBridge
@testable import TraceCommonsApp

final class InsightsEpisodeModelTests: XCTestCase {
    @MainActor
    func testCreateEditAssessClearAndDeleteLifecycle() async throws {
        let service = EpisodeFakeService()
        let model = InsightsModel(service: { try await service.call($0) })
        model.open(); try await settle(model)
        model.episodeCreateSelection = [EpisodeFakeService.snapshotID]
        model.createEpisode(); try await settle(model)
        XCTAssertEqual(model.episodes.count, 1)
        XCTAssertEqual(model.episodeDetail?.episode.members.map(\.snapshot_id), [EpisodeFakeService.snapshotID])

        model.episodeCategory = "tests"; model.episodeOutcome = "accepted"
        model.saveEpisodeAssessment(); try await settle(model)
        XCTAssertEqual(model.episodeDetail?.episode.manual_assessment?.outcome, "accepted")
        let assessedRevision = try XCTUnwrap(model.episodeDetail?.episode.revision)

        model.beginEpisodeMemberEdit()
        model.saveEpisodeMembers(); try await settle(model)
        XCTAssertEqual(model.episodeDetail?.episode.revision, assessedRevision,
                       "A no-op member replacement preserves the assessment and revision")
        XCTAssertNotNil(model.episodeDetail?.episode.manual_assessment)

        model.clearEpisodeAssessment(try XCTUnwrap(model.episodeConfirmation())); try await settle(model)
        XCTAssertNil(model.episodeDetail?.episode.manual_assessment)
        model.deleteEpisode(try XCTUnwrap(model.episodeConfirmation())); try await settle(model)
        XCTAssertNil(model.episodeDetail)
        XCTAssertTrue(model.episodes.isEmpty)
        let mutationTypes = await service.mutationTypes
        XCTAssertEqual(mutationTypes,
                       ["episode_create", "episode_annotate", "episode_replace_members",
                        "episode_clear_assessment", "episode_delete"])
    }

    @MainActor
    func testConflictDiscardsDraftRefreshesAndNeverRetries() async throws {
        let service = EpisodeFakeService(seedEpisode: true)
        let model = InsightsModel(service: { try await service.call($0) })
        model.open(); try await settle(model)
        model.openEpisode(EpisodeFakeService.episodeID); try await settle(model)
        model.beginEpisodeMemberEdit()
        await service.advanceRevisionOutsideUI()
        model.saveEpisodeMembers(); try await settle(model)
        XCTAssertEqual(model.episodeError, "conflict")
        XCTAssertTrue(model.episodeEditSelection.isEmpty)
        XCTAssertEqual(model.episodeDetail?.episode.revision, 2)
        let mutationTypes = await service.mutationTypes
        XCTAssertEqual(mutationTypes.filter { $0 == "episode_replace_members" }.count, 1)
    }

    @MainActor
    func testLateDetailCannotReplaceNewPresentationAndSnapshotControlsRemainIndependent() async throws {
        let service = EpisodeFakeService(seedEpisode: true)
        let model = InsightsModel(service: { try await service.call($0) })
        model.open(); try await settle(model)
        await service.holdNextExplain()
        model.openEpisode(EpisodeFakeService.episodeID)
        XCTAssertTrue(model.episodeBusy)
        model.analyze(file: URL(fileURLWithPath: "/synthetic"), source: "codex")
        XCTAssertTrue(model.busy, "Episode IO must not disable snapshot operations")
        model.closeEpisode()
        await service.releaseExplain()
        try await Task.sleep(for: .milliseconds(30))
        XCTAssertNil(model.episodeDetail, "A callback from the closed presentation must be ignored")
    }

    @MainActor
    func testEpisodeDetailProducesSyntheticNativeRender() async throws {
        let service = EpisodeFakeService(seedEpisode: true, useActualCopy: true)
        let model = InsightsModel(service: { try await service.call($0) })
        model.open(); try await settle(model)
        model.openEpisode(EpisodeFakeService.episodeID); try await settle(model)
        _ = NSApplication.shared
        let size = CGSize(width: 700, height: 900)
        let content = ScrollView { InsightsEpisodesView(model: model).padding(20).frame(width: 700) }
            .frame(width: size.width, height: size.height, alignment: .topLeading)
            .background(Color(nsColor: .windowBackgroundColor))
        let hosting = NSHostingView(rootView: content)
        let bounds = NSRect(origin: .zero, size: size)
        hosting.frame = bounds
        let window = NSWindow(contentRect: bounds, styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false; window.contentView = hosting
        defer { window.close() }
        hosting.layoutSubtreeIfNeeded(); window.displayIfNeeded()
        let bitmap = try XCTUnwrap(hosting.bitmapImageRepForCachingDisplay(in: bounds))
        hosting.cacheDisplay(in: bounds, to: bitmap)
        let png = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
        XCTAssertGreaterThan(png.count, 15_000)
        if let path = ProcessInfo.processInfo.environment["TC_INSIGHTS_EPISODE_RENDER"] {
            try png.write(to: URL(fileURLWithPath: path))
        }
    }

    @MainActor
    func testAcceptedMutationRefreshFailureClearsEditableDetail() async throws {
        let service = EpisodeFakeService(seedEpisode: true)
        let model = InsightsModel(service: { try await service.call($0) })
        model.open(); try await settle(model)
        model.openEpisode(EpisodeFakeService.episodeID); try await settle(model)
        await service.failNextListRefresh()
        model.saveEpisodeAssessment(); try await settle(model)
        XCTAssertNil(model.episodeDetail)
        XCTAssertTrue(model.episodeEditSelection.isEmpty)
        XCTAssertEqual(model.episodeError, "list unavailable")
        let committedRevision = await service.currentRevision()
        XCTAssertEqual(committedRevision, 2, "The accepted write remains committed")
    }

    @MainActor
    func testStaleConfirmationCannotDeleteReopenedEpisode() async throws {
        let service = EpisodeFakeService(seedEpisode: true)
        let model = InsightsModel(service: { try await service.call($0) })
        model.open(); try await settle(model)
        model.openEpisode(EpisodeFakeService.episodeID); try await settle(model)
        let stale = try XCTUnwrap(model.episodeConfirmation())
        model.closeEpisode()
        model.openEpisode(EpisodeFakeService.episodeID); try await settle(model)
        model.deleteEpisode(stale)
        try await Task.sleep(for: .milliseconds(20))
        XCTAssertNotNil(model.episodeDetail)
        let mutations = await service.mutationTypes
        XCTAssertFalse(mutations.contains("episode_delete"))
    }

    @MainActor
    func testChangedMembershipNoticeSurvivesFailedReconciliationAndEmptyDraftStaysOpen() async throws {
        let service = EpisodeFakeService(seedEpisode: true)
        let model = InsightsModel(service: { try await service.call($0) })
        model.open(); try await settle(model)
        model.openEpisode(EpisodeFakeService.episodeID); try await settle(model)
        model.beginEpisodeMemberEdit()
        model.episodeEditSelection = []
        model.saveEpisodeMembers()
        XCTAssertTrue(model.episodeEditingMembers, "An empty selection error must not hide the editor")
        model.episodeEditSelection = [EpisodeFakeService.snapshotID]
        await service.changeMembershipOnNextReplaceAndFailRefresh()
        model.saveEpisodeMembers(); try await settle(model)
        XCTAssertEqual(model.episodeNotice, "members saved membership changed")
        XCTAssertNil(model.episodeDetail)
    }

    @MainActor
    func testMismatchedDetailAndExternalDeletionCannotLeaveEditableState() async throws {
        let service = EpisodeFakeService(seedEpisode: true)
        let model = InsightsModel(service: { try await service.call($0) })
        model.open(); try await settle(model)
        await service.mismatchNextDetail()
        model.openEpisode(EpisodeFakeService.episodeID); try await settle(model)
        XCTAssertNil(model.episodeDetail)
        XCTAssertEqual(model.episodeError, "detail unavailable")
        model.openEpisode(EpisodeFakeService.episodeID); try await settle(model)
        XCTAssertNotNil(model.episodeDetail)
        await service.deleteOutsideUI()
        model.refreshEpisodes(); try await settle(model)
        XCTAssertNil(model.episodeDetail)
        XCTAssertTrue(model.episodes.isEmpty)
    }

    func testActualServiceEpisodeRoundTrip() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let source = root.appendingPathComponent("synthetic.json")
        try Data("[{\"role\":\"meta\",\"source\":\"fixture\",\"model\":\"fixture\"},{\"role\":\"user\",\"timestamp\":\"2026-01-01T00:00:00Z\",\"content\":\"synthetic\"}]".utf8).write(to: source)
        let store = root.appendingPathComponent("store").path
        let saved = try TCInsights.call(.init(storeDirectory: store,
            operation: .init("analyze", source: "trajectory", file: source.path, save: true)))
        let snapshotID = try XCTUnwrap(saved.insight?.id)
        let created = try TCInsights.call(.init(storeDirectory: store,
            operation: .init("episode_create", snapshotIDs: [snapshotID])))
        let episode = try XCTUnwrap(created.episode)
        try episode.validateSupportedSchema()
        let explained = try TCInsights.call(.init(storeDirectory: store,
            operation: .init("episode_explain", id: episode.id)))
        try XCTUnwrap(explained.detail).validateSupportedSchema(expectedID: episode.id)
        let assessed = try TCInsights.call(.init(storeDirectory: store,
            operation: .init("episode_annotate", id: episode.id, category: "tests", outcome: "accepted",
                             expectedRevision: episode.revision)))
        XCTAssertEqual(assessed.episode?.manual_assessment?.outcome, "accepted")
        let deleted = try TCInsights.call(.init(storeDirectory: store,
            operation: .init("episode_delete", id: episode.id,
                             expectedRevision: try XCTUnwrap(assessed.episode?.revision))))
        XCTAssertEqual(deleted.episode?.id, episode.id)
        let list = try TCInsights.call(.init(storeDirectory: store, operation: .init("episode_list")))
        XCTAssertTrue(try XCTUnwrap(list.episodes).isEmpty)
    }

    @MainActor private func settle(_ model: InsightsModel) async throws {
        for _ in 0..<500 {
            if !model.busy && !model.episodeBusy { return }
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTFail("Model did not settle")
    }
}

private actor EpisodeFakeService {
    static let snapshotID = String(repeating: "a", count: 64)
    static let episodeID = "d0c18c96-6093-49f5-bb6f-6092ef0630b9"
    private var episode: [String: Any]?
    private var heldExplain = false
    private var failList = false
    private var mismatchedDetail = false
    private var changeMembership = false
    private var explainContinuation: CheckedContinuation<Void, Never>?
    private(set) var mutationTypes: [String] = []
    private let useActualCopy: Bool

    init(seedEpisode: Bool = false, useActualCopy: Bool = false) {
        self.useActualCopy = useActualCopy
        if seedEpisode { episode = Self.makeEpisode(revision: 1) }
    }
    func holdNextExplain() { heldExplain = true }
    func releaseExplain() { explainContinuation?.resume(); explainContinuation = nil }
    func advanceRevisionOutsideUI() { episode = Self.makeEpisode(revision: 2) }
    func failNextListRefresh() { failList = true }
    func mismatchNextDetail() { mismatchedDetail = true }
    func deleteOutsideUI() { episode = nil }
    func currentRevision() -> UInt64? { episode?["revision"] as? UInt64 }
    func changeMembershipOnNextReplaceAndFailRefresh() { changeMembership = true; failList = true }

    func call(_ request: InsightsRequest) async throws -> InsightsResponse {
        let op = request.operation
        if op.type.hasPrefix("episode_") && !["episode_list", "episode_explain"].contains(op.type) {
            mutationTypes.append(op.type)
        }
        var result: [String: Any] = ["type": op.type]
        switch op.type {
        case "copy":
            if useActualCopy { return try TCInsights.call(.init(operation: .init("copy"))) }
            result["copy"] = Self.copy
        case "list": result["insights"] = [Self.snapshot]
        case "summary": return try actualEmptySummary(type: op.type)
        case "analyze": result["insight"] = Self.snapshot
        case "episode_list":
            if failList { failList = false; throw InsightsError.invalidResponse }
            result["episodes"] = episode.map { [["episode": $0, "overlapping_episode_ids": []]] } ?? []
        case "episode_explain":
            if heldExplain {
                heldExplain = false
                await withCheckedContinuation { explainContinuation = $0 }
            }
            guard let episode else { throw InsightsError.service("insights_episode_not_found") }
            var returnedEpisode = episode
            if mismatchedDetail {
                mismatchedDetail = false
                returnedEpisode["id"] = "e1b29da7-e97d-4506-b714-20aa996aad75"
            }
            result["detail"] = ["episode": returnedEpisode, "members": [Self.snapshot],
                                "overlap": [["snapshot_id": Self.snapshotID, "episode_ids": []]],
                                "resolved_at": "2026-01-01T00:00:00Z"]
        case "episode_create": episode = Self.makeEpisode(revision: 1); result["episode"] = episode
        case "episode_annotate":
            guard expected(op, matches: episode) else { throw InsightsError.service("insights_episode_revision_conflict") }
            episode = Self.makeEpisode(revision: (op.expected_revision ?? 0) + 1, assessed: true); result["episode"] = episode
        case "episode_replace_members":
            guard expected(op, matches: episode) else { throw InsightsError.service("insights_episode_revision_conflict") }
            if changeMembership {
                changeMembership = false
                episode = Self.makeEpisode(revision: (op.expected_revision ?? 0) + 1,
                                           membershipRevision: 2)
            }
            result["episode"] = episode
        case "episode_clear_assessment":
            guard expected(op, matches: episode) else { throw InsightsError.service("insights_episode_revision_conflict") }
            episode = Self.makeEpisode(revision: (op.expected_revision ?? 0) + 1); result["episode"] = episode
        case "episode_delete":
            guard expected(op, matches: episode) else { throw InsightsError.service("insights_episode_revision_conflict") }
            result["episode"] = episode; episode = nil
        default: throw InsightsError.invalidResponse
        }
        return try JSONDecoder().decode(InsightsResponse.self,
                                        from: JSONSerialization.data(withJSONObject: result))
    }

    private func expected(_ op: InsightsRequest.Operation, matches episode: [String: Any]?) -> Bool {
        op.expected_revision == episode?["revision"] as? UInt64
    }
    private func actualEmptySummary(type: String) throws -> InsightsResponse {
        let json = "{\"type\":\"summary\",\"summary\":{\"schema_version\":1,\"scope\":\"all_saved_selected_session_snapshots\",\"limitations\":[\"selected_saved_sessions_are_not_verified_tasks\",\"assessments_are_user_reported\",\"observed_sums_require_both_coverages\",\"analysis_dates_are_not_activity_time\",\"source_formats_are_not_model_identity\",\"no_model_rankings_time_savings_or_cost\"],\"provider\":{\"id\":\"local\",\"version\":\"1\",\"rubric_version\":\"1\",\"execution_mode\":\"local\"},\"saved_snapshots\":0,\"snapshot_analysis_range\":null,\"user_reported\":{\"assessed_snapshots\":0,\"unassessed_snapshots\":0,\"categories\":[],\"outcomes\":[]},\"metrics\":[],\"snapshots\":[]}}"
        return try JSONDecoder().decode(InsightsResponse.self, from: Data(json.utf8))
    }
    private static let snapshot: [String: Any] = [
        "id": snapshotID, "source_format": "codex", "boundary": "session_proxy",
        "analyzed_at": "2026-01-01T00:00:00Z", "cost_unavailable_reason": "unknown",
        "report": ["schema_version": 1, "provider": ["id": "local", "version": "1", "rubric_version": "1", "execution_mode": "local"],
                   "metrics": [], "evidence": [["id": "session", "source_digest": snapshotID]]]
    ]
    private static let copy = [
        "episode_title": "Episodes", "episode_scope": "whole snapshots", "episode_saved": "Saved episodes",
        "episode_empty": "empty", "episode_create": "Create", "episode_create_notice": "select whole snapshots",
        "episode_selection_empty": "select", "episode_create_success": "created", "refresh": "Refresh",
        "episode_members": "Members", "episode_no_overlap": "No overlap", "episode_overlaps": "Overlaps",
        "episode_delete": "Delete", "episode_id": "ID", "episode_revision": "Revision",
        "episode_membership_revision": "Membership revision", "episode_created_at": "Created",
        "episode_updated_at": "Edited", "episode_resolved": "Resolved", "episode_overlap_notice": "overlap notice",
        "episode_member_evidence": "Evidence", "episode_edit_members": "Edit", "episode_edit_members_notice": "review",
        "episode_save_members": "Save members", "episode_assessment": "Assessment",
        "episode_assessment_notice_short": "independent", "episode_unassessed": "Unassessed",
        "episode_save_assessment": "Save assessment", "episode_clear_assessment": "Clear assessment",
        "episode_assessment_notice": "user report", "episode_clear_assessment_confirm": "Clear?",
        "episode_delete_confirm": "Delete?", "episode_deleted": "deleted", "episode_members_saved": "members saved",
        "episode_assessment_saved": "assessment saved", "episode_assessment_cleared": "assessment cleared",
        "episode_membership_changed": "membership changed",
        "episode_revision_conflict": "conflict", "episode_missing": "missing",
        "episode_list_unavailable": "list unavailable", "episode_detail_unavailable": "detail unavailable",
        "category": "Category", "outcome": "Outcome", "category_unknown": "Unknown", "category_refactor": "Refactor",
        "category_tests": "Tests", "category_docs": "Docs", "category_debugging": "Debug", "category_other": "Other",
        "outcome_unknown": "Unknown", "outcome_accepted": "Accepted", "outcome_partial": "Partial",
        "outcome_rejected": "Rejected", "codex": "Codex", "error": "error"
    ]
    private static func makeEpisode(revision: UInt64, membershipRevision: UInt64 = 1,
                                    assessed: Bool = false) -> [String: Any] {
        var value: [String: Any] = ["schema_version": 1, "id": episodeID, "revision": revision,
            "membership_revision": membershipRevision, "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z", "provenance": "user_selected_whole_snapshots",
            "members": [["snapshot_id": snapshotID, "source_digest": snapshotID]]]
        if assessed { value["manual_assessment"] = ["category": "tests", "outcome": "accepted",
            "provenance": "user_reported", "recorded_at": "2026-01-01T00:00:00Z",
            "membership_revision": 1, "members_digest": snapshotID] }
        return value
    }
}
