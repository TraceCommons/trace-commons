import Foundation
import XCTest
import TCBridge
@testable import TraceCommonsApp

final class ComparisonSpecificationsModelTests: XCTestCase {
    @MainActor
    func testPreviewSaveSelectEvaluateAndDigestBoundExplain() async throws {
        let service = SpecificationFakeService()
        let model = ComparisonSpecificationsModel(service: { try await service.call($0) })
        model.open(); try await settle(model)
        model.updateSources(tasks: [try Self.task()], snapshots: try Self.snapshots())
        XCTAssertEqual(model.selectedOption?.cohortCandidates, ["model-a", "model-b"])
        model.setCohort("model-a", selected: true); model.setCohort("model-b", selected: true)
        model.preview(); try await settle(model); XCTAssertNotNil(model.previewResult)
        model.save(); try await settle(model); XCTAssertEqual(model.notice, "comparison_specification_saved_notice")
        model.evaluate(); try await settle(model)
        let confirmation = try XCTUnwrap(model.resultConfirmation())
        model.explain(confirmation); try await settle(model)
        let operations = await service.operations()
        XCTAssertEqual(operations, ["comparison_list_specs", "comparison_preview_spec",
            "comparison_save_spec", "comparison_list_specs", "comparison_get_spec",
            "comparison_evaluate", "comparison_explain_result"])
    }

    @MainActor
    func testStaleResultConfirmationCannotExplainAnotherPresentation() async throws {
        let service = SpecificationFakeService(seed: true)
        let model = ComparisonSpecificationsModel(service: { try await service.call($0) })
        model.open(); try await settle(model); model.select(SpecificationFakeService.specID); try await settle(model)
        model.evaluate(); try await settle(model); let stale = try XCTUnwrap(model.resultConfirmation())
        model.refresh(); try await settle(model); model.explain(stale)
        try await Task.sleep(for: .milliseconds(20))
        let explainCount = await service.count("comparison_explain_result")
        XCTAssertEqual(explainCount, 0)
    }

    @MainActor
    func testCloseReopenReconcilesDetachedCommittedSave() async throws {
        let service = SpecificationFakeService()
        let model = ComparisonSpecificationsModel(service: { try await service.call($0) })
        model.open(); try await settle(model); model.updateSources(tasks: [try Self.task()], snapshots: try Self.snapshots())
        model.setCohort("model-a", selected: true); model.setCohort("model-b", selected: true)
        model.preview(); try await settle(model); await service.holdSave(); model.save()
        try await Task.sleep(for: .milliseconds(20)); model.close(); model.open()
        await service.releaseSave(); try await settle(model)
        XCTAssertEqual(model.selected?.id, SpecificationFakeService.specID)
        XCTAssertEqual(model.notice, "comparison_specification_saved_notice")
        let saveCount = await service.count("comparison_save_spec")
        XCTAssertEqual(saveCount, 1)
    }

    @MainActor
    func testUpstreamInvalidationDuringHeldReadIsCoalesced() async throws {
        let service = SpecificationFakeService(seed: true)
        let model = ComparisonSpecificationsModel(service: { try await service.call($0) })
        model.open(); try await settle(model); await service.holdList(); model.refresh()
        try await Task.sleep(for: .milliseconds(20)); model.upstreamEvidenceChanged(); await service.releaseList()
        try await settle(model)
        let listCount = await service.count("comparison_list_specs")
        XCTAssertGreaterThanOrEqual(listCount, 3)
    }

    @MainActor
    func testMalformedCurrentPreviewFinishesWithError() async throws {
        let service = SpecificationFakeService()
        await service.malformedPreview()
        let model = ComparisonSpecificationsModel(service: { try await service.call($0) })
        model.open(); try await settle(model); model.updateSources(tasks: [try Self.task()], snapshots: try Self.snapshots())
        model.setCohort("model-a", selected: true); model.setCohort("model-b", selected: true)
        model.preview(); try await settle(model)
        XCTAssertFalse(model.busy); XCTAssertEqual(model.error, "comparison_specification_error")
    }

    @MainActor func testDraftDayUsesPickerCalendarRatherThanUTCDate() {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(secondsFromGMT: 14 * 60 * 60)!
        var utc = Calendar(identifier: .gregorian); utc.timeZone = TimeZone(secondsFromGMT: 0)!
        let instant = utc.date(from: .init(year: 2026, month: 9, day: 10, hour: 20))!
        XCTAssertEqual(ComparisonSpecificationsModel.day(instant, calendar: calendar), "2026-09-11")
    }

    @MainActor
    func testStableTaskIDContextAndOutcomeChangesRefreshSourcesAndResult() async throws {
        let service = SpecificationFakeService(seed: true)
        let model = ComparisonSpecificationsModel(service: { try await service.call($0) })
        model.open(); try await settle(model)
        model.updateSources(tasks: [try Self.task(complete: false)], snapshots: try Self.snapshots())
        XCTAssertTrue(model.options.isEmpty)
        model.sourceEvidenceChanged(tasks: [try Self.task()], snapshots: try Self.snapshots())
        try await settle(model); XCTAssertEqual(model.options.count, 1)
        model.select(SpecificationFakeService.specID); try await settle(model); model.evaluate(); try await settle(model)
        let prior = await service.count("comparison_evaluate")
        model.sourceEvidenceChanged(tasks: [try Self.task(revision: 2, outcomeRecordedAt: "2026-09-12T01:00:00Z")],
                                    snapshots: try Self.snapshots())
        try await settle(model)
        let current = await service.count("comparison_evaluate")
        XCTAssertEqual(current, prior + 1)
    }

    @MainActor
    func testOldReconciliationFailureCannotOverwriteReopenedRead() async throws {
        let service = SpecificationFakeService()
        let model = ComparisonSpecificationsModel(service: { try await service.call($0) })
        model.open(); try await settle(model); model.updateSources(tasks: [try Self.task()], snapshots: try Self.snapshots())
        model.setCohort("model-a", selected: true); model.setCohort("model-b", selected: true)
        model.preview(); try await settle(model)
        await service.holdAndFailList(); model.save(); try await Task.sleep(for: .milliseconds(20))
        model.close(); await service.holdList(); model.open(); try await Task.sleep(for: .milliseconds(20))
        await service.releaseFailingList(); try await Task.sleep(for: .milliseconds(20))
        XCTAssertTrue(model.busy); XCTAssertNil(model.error)
        await service.releaseList(); try await settle(model)
        XCTAssertNil(model.error)
    }

    @MainActor private func settle(_ model: ComparisonSpecificationsModel) async throws {
        for _ in 0..<300 { if !model.busy { return }; try await Task.sleep(for: .milliseconds(10)) }
        XCTFail("Specification model did not settle")
    }

    private static func task(complete: Bool = true, revision: UInt64 = 1,
                             outcomeRecordedAt: String? = nil) throws -> ComparisonTaskDetail {
        var task = SpecificationFixtures.task()
        task["revision"] = revision
        task["context"] = ["project_id": SpecificationFakeService.projectID, "category": "refactor",
            "task_date": "2026-09-12", "checkout_provenance": ["state": "unavailable"],
            "language": ["state": "known", "value": "swift"],
            "configuration": ["harness_id": ["state": "known", "value": "codex"],
                "harness_version": ["state": "known", "value": "1"], "reasoning_effort": "high",
                "tool_policy_id": ["state": "known", "value": "default"],
                "tool_policy_version": ["state": "known", "value": "1"],
                "prompt_template_digest": ["state": "known", "digest": String(repeating: "f", count: 64)]],
            "configuration_fingerprint": String(repeating: "a", count: 64)]
        if !complete { task["context"] = NSNull() }
        if let outcomeRecordedAt {
            task["outcome"] = ["value": "accepted", "material_revision": 1,
                "material_digest": String(repeating: "a", count: 64), "recorded_at": outcomeRecordedAt]
        }
        return try JSONDecoder().decode(ComparisonTaskDetail.self,
            from: JSONSerialization.data(withJSONObject: SpecificationFixtures.detail(task: task)))
    }
    private static func snapshots() throws -> [LocalInsight] {
        try ["model-a", "model-b"].enumerated().map { index, label in
            let json: [String: Any] = ["id": String(repeating: index == 0 ? "c" : "f", count: 64),
                "source_format": "codex", "boundary": "local", "analyzed_at": "2026-09-12T00:00:00Z",
                "cost_unavailable_reason": "unknown", "report": ["schema_version": 1,
                    "provider": ["id": "local", "version": "1", "rubric_version": "1", "execution_mode": "local"],
                    "metrics": [], "evidence": []], "model_observations": ["schema_version": 2,
                    "scope": "declared_metadata_only", "source_format": "codex", "source_digest": "abc",
                    "coordinates": "jsonl_physical_lines_one_based", "record_count": 1, "candidate_records": 1,
                    "valid_declarations": 1, "missing_declarations": 0, "invalid_declarations": 0,
                    "omitted_declarations": 0, "model_labels_omitted": false, "mixed_declared_models": false,
                    "declared_models": [label], "declarations": [["model": label, "record_index": 1,
                                                                   "kind": "codex_session_metadata"]]]]
            return try JSONDecoder().decode(LocalInsight.self, from: JSONSerialization.data(withJSONObject: json))
        }
    }
}

private actor SpecificationFakeService {
    static let specID = "40c18c96-6093-49f5-bb6f-6092ef0630b9"
    static let projectID = "30c18c96-6093-49f5-bb6f-6092ef0630b9"
    private var saved: [String: Any]?
    private var calls: [String] = []
    private var holdSaveFlag = false, holdListFlag = false
    private var malformedPreviewFlag = false
    private var saveContinuation: CheckedContinuation<Void, Never>?
    private var listContinuation: CheckedContinuation<Void, Never>?
    private var failListContinuation: CheckedContinuation<Void, Never>?
    private var holdFailListFlag = false
    init(seed: Bool = false) { if seed { saved = Self.specification() } }
    func operations() -> [String] { calls }
    func count(_ operation: String) -> Int { calls.filter { $0 == operation }.count }
    func holdSave() { holdSaveFlag = true }
    func releaseSave() { saveContinuation?.resume(); saveContinuation = nil }
    func holdList() { holdListFlag = true }
    func holdAndFailList() { holdFailListFlag = true }
    func malformedPreview() { malformedPreviewFlag = true }
    func releaseList() { listContinuation?.resume(); listContinuation = nil }
    func releaseFailingList() { failListContinuation?.resume(); failListContinuation = nil }
    func call(_ request: InsightsRequest) async throws -> InsightsResponse {
        let type = request.operation.type; calls.append(type); var payload: [String: Any] = ["type": type]
        switch type {
        case "comparison_list_specs":
            let captured = saved.map { [$0] } ?? []
            if holdFailListFlag {
                holdFailListFlag = false
                await withCheckedContinuation { failListContinuation = $0 }
                throw InsightsError.invalidResponse
            }
            if holdListFlag { holdListFlag = false; await withCheckedContinuation { listContinuation = $0 } }
            payload["type"] = "comparison_specification_list"; payload["specifications"] = captured
        case "comparison_preview_spec":
            if !malformedPreviewFlag {
                payload["specification"] = Self.specification(); payload["result"] = Self.result()
            }
            malformedPreviewFlag = false
        case "comparison_save_spec":
            if holdSaveFlag { holdSaveFlag = false; await withCheckedContinuation { saveContinuation = $0 } }
            saved = Self.specification(); payload["type"] = "comparison_specification"; payload["specification"] = saved
        case "comparison_get_spec":
            payload["type"] = "comparison_specification"; payload["specification"] = saved
        case "comparison_evaluate", "comparison_explain_result":
            payload["type"] = "comparison_result"; payload["result"] = Self.result()
        default: throw InsightsError.invalidResponse
        }
        return try JSONDecoder().decode(InsightsResponse.self, from: JSONSerialization.data(withJSONObject: payload))
    }
    private static func specification() -> [String: Any] {
        var value = SpecificationFixtures.specification(); value["id"] = specID
        value["stratum"] = ["project_id": projectID, "language": "swift",
                            "configuration_fingerprint": String(repeating: "a", count: 64)]
        return value
    }
    private static func result() -> [String: Any] {
        var value = SpecificationFixtures.comparisonResult(); value["specification_id"] = specID; return value
    }
}

private enum SpecificationFixtures {
    static func task() -> [String: Any] {
        ["schema_version": 1, "id": "20c18c96-6093-49f5-bb6f-6092ef0630b9", "revision": 1,
         "material_revision": 1, "material_digest": String(repeating: "a", count: 64),
         "created_at": "2026-09-12T00:00:00Z", "updated_at": "2026-09-12T00:00:00Z",
         "episodes": [["episode_id": "d0c18c96-6093-49f5-bb6f-6092ef0630b9", "revision": 1,
                       "membership_revision": 1, "members_digest": String(repeating: "b", count: 64),
                       "members": [["snapshot_id": String(repeating: "c", count: 64),
                                    "source_digest": String(repeating: "d", count: 64)],
                                   ["snapshot_id": String(repeating: "f", count: 64),
                                    "source_digest": String(repeating: "e", count: 64)]]]],
         "context": NSNull(), "outcome": NSNull(), "independence_confirmation": NSNull()]
    }
    static func detail(task: [String: Any]) -> [String: Any] {
        ["task": task, "stale_reasons": ["attribution_pending_qualification"],
         "overlapping_task_ids": [], "resolved_at": "2026-09-12T00:00:00Z"]
    }
    static func specification() -> [String: Any] {
        ["schema_version": 1, "id": SpecificationFakeService.specID,
         "provenance": "retrospective_user_specification", "created_at": "2026-09-12T00:00:00Z",
         "evidence_cutoff": "2026-09-13T00:00:00Z", "cutoff_task_evidence": [], "category": "refactor",
         "cohort_labels": ["model-a", "model-b"], "date_start": "2026-09-01", "date_end": "2026-09-12",
         "stratum": ["project_id": SpecificationFakeService.projectID, "language": "swift",
                     "configuration_fingerprint": String(repeating: "a", count: 64)],
         "task_rule": "one-user-confirmed-work-item-v1", "attempt_rule": "canonical-snapshot-union-v1",
         "outcome_rubric_version": "categorical-user-report-v1",
         "estimands": ["categorical-outcome-distribution-v1"], "estimator_state": "not_yet_calibrated",
         "specification_digest": String(repeating: "b", count: 64),
         "saved_record_digest": String(repeating: "c", count: 64)]
    }
    static func comparisonResult() -> [String: Any] {
        ["schema_version": 1, "specification_id": SpecificationFakeService.specID,
         "specification_digest": String(repeating: "b", count: 64),
         "specification_record_digest": String(repeating: "c", count: 64),
         "audit_digest": String(repeating: "d", count: 64),
         "estimation_input_digest": String(repeating: "e", count: 64), "included_task_ids": [],
         "excluded_tasks": [], "cohorts": []]
    }
}
