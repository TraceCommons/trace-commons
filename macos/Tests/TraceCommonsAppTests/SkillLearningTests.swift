// INTEGRATION: verifies the correction-to-tested-skill state machine through
// the real Swift daemon client. Synthetic responses stop at this process edge;
// live NEAR AI evaluation and filesystem installation are Rust integration gates.

import AppKit
import SwiftUI
import TCBridge
import XCTest

@testable import TraceCommonsApp

final class SkillLearningTests: XCTestCase {
    private var daemon: SkillLearningDaemon!
    private var client: DaemonClient!

    override func setUp() {
        super.setUp()
        daemon = SkillLearningDaemon()
        client = DaemonClient(daemon: daemon)
    }

    @MainActor
    func testStoreBoundsSettledSessionsAndKeepsTypedRecoveryLabels() {
        var store = SkillLearningStore()
        for index in 0..<30 {
            let id = "session-\(index)"
            XCTAssertTrue(store.beginInstallStatusLoading(for: id))
            XCTAssertFalse(store.beginInstallStatusLoading(for: id))
            XCTAssertTrue(store.finishInstallStatusLoading(for: id, installed: nil))
            XCTAssertFalse(store.beginInstallStatusLoading(for: id))
        }

        XCTAssertEqual(store.retainedSessionCount, 24)
        XCTAssertTrue(
            SkillLearningDaemonFailure(label: "skill-review-unknown").invalidatesWorkflow
        )
        XCTAssertEqual(
            SkillLearningDaemonFailure(label: "skill-install-unknown"),
            .installUnknown
        )
        XCTAssertFalse(
            SkillLearningDaemonFailure(label: "skill-install-unknown").invalidatesWorkflow
        )
    }

    @MainActor
    func testStoreAllowsOnlyLegalTransitionsAndRestoresThePriorPhaseAfterRollback() throws {
        let candidate = try client.skillCandidate(submissionID: record.submissionID)
        let review = try client.reviewSkill(candidateID: candidate.candidateID, draft: candidate.draft)
        let report = try client.evaluateSkill(
            reviewID: review.reviewID,
            skillSHA256: review.skillSHA256
        )
        let plan = try client.skillInstallPlan(evaluationID: report.evaluationID)
        let installed = try client.installSkill(
            planID: plan.planID,
            asPreviewedSHA256: plan.skillSHA256,
            asPreviewedMarkerSHA256: plan.markerFileSHA256
        )
        let wrongCandidate = try client.skillCandidate(
            submissionID: "88888888-8888-4888-8888-888888888888"
        )
        daemon.setResponseField(
            "skill_review",
            field: "candidate_id",
            value: "99999999-9999-4999-8999-999999999999"
        )
        let wrongReview = try client.reviewSkill(
            candidateID: candidate.candidateID,
            draft: candidate.draft
        )
        daemon.setResponseField("skill_review", field: "candidate_id", value: nil)
        daemon.setResponseField(
            "skill_evaluate",
            field: "review_id",
            value: "99999999-9999-4999-8999-999999999999"
        )
        let wrongReport = try client.evaluateSkill(
            reviewID: review.reviewID,
            skillSHA256: review.skillSHA256
        )
        daemon.setResponseField("skill_evaluate", field: "review_id", value: nil)
        daemon.setResponseField(
            "skill_install_plan",
            field: "evaluation_id",
            value: "99999999-9999-4999-8999-999999999999"
        )
        let wrongPlan = try client.skillInstallPlan(evaluationID: report.evaluationID)
        daemon.setResponseField("skill_install_plan", field: "evaluation_id", value: nil)
        daemon.setResponseField(
            "skill_install_commit",
            field: "evaluation_id",
            value: "99999999-9999-4999-8999-999999999999"
        )
        let wrongInstalled = try client.installSkill(
            planID: plan.planID,
            asPreviewedSHA256: plan.skillSHA256,
            asPreviewedMarkerSHA256: plan.markerFileSHA256
        )
        daemon.setResponseField("skill_install_commit", field: "evaluation_id", value: nil)

        var store = SkillLearningStore()
        let id = record.submissionID
        XCTAssertNil(store.beginReview(for: id))
        XCTAssertNil(store.beginInstallPlanning(for: id))
        XCTAssertFalse(store.beginLearning(for: id))
        XCTAssertTrue(store.beginInstallStatusLoading(for: id))
        XCTAssertTrue(store.finishInstallStatusLoading(for: id, installed: nil))
        XCTAssertTrue(store.beginLearning(for: id))
        XCTAssertFalse(store.beginLearning(for: id))
        XCTAssertFalse(store.finishLearning(for: id, candidate: wrongCandidate))
        XCTAssertTrue(store.state(for: id).isWorking)
        XCTAssertTrue(store.finishLearning(for: id, candidate: candidate))

        XCTAssertFalse(store.beginLearning(for: id))
        XCTAssertEqual(store.beginReview(for: id)?.candidate, candidate)
        XCTAssertFalse(store.finishReview(for: id, review: wrongReview))
        XCTAssertTrue(store.state(for: id).isWorking)
        XCTAssertTrue(store.finishReview(for: id, review: review))
        XCTAssertEqual(store.beginEvaluation(for: id), review)
        XCTAssertFalse(store.finishEvaluation(for: id, report: wrongReport))
        XCTAssertTrue(store.state(for: id).isWorking)
        XCTAssertTrue(store.finishEvaluation(for: id, report: report))
        XCTAssertEqual(store.beginInstallPlanning(for: id), report)
        XCTAssertFalse(store.finishInstallPlanning(for: id, plan: wrongPlan))
        XCTAssertTrue(store.state(for: id).isWorking)
        XCTAssertTrue(store.finishInstallPlanning(for: id, plan: plan))
        XCTAssertEqual(store.beginInstall(for: id), plan)
        XCTAssertFalse(store.finishInstall(for: id, installed: wrongInstalled))
        XCTAssertTrue(store.state(for: id).isWorking)
        XCTAssertTrue(store.finishInstall(for: id, installed: installed))
        XCTAssertEqual(store.beginRollback(for: id), installed)
        store.finishRollback(for: id, removed: true, incompleteMessage: "incomplete")

        guard case .evaluated(let restoredCandidate, let restoredReview, let restoredReport) =
            store.state(for: id).phase
        else {
            return XCTFail("Rollback must restore the evaluated install source")
        }
        XCTAssertEqual(restoredCandidate, candidate)
        XCTAssertEqual(restoredReview, review)
        XCTAssertEqual(restoredReport, report)
        XCTAssertNil(store.state(for: id).operation)
        XCTAssertNil(store.state(for: id).failure)
    }

    @MainActor
    func testModelCompletesApprovalEvaluationInstallAndRollback() async throws {
        let model = AppModel()
        model.setClientForTesting(client)

        model.learnSkill(from: record)
        try await waitUntil { self.candidate(in: model, id: self.record.submissionID) != nil }
        let candidate = try XCTUnwrap(candidate(in: model, id: record.submissionID))

        model.reviewSkill(from: record, draft: candidate.draft)
        try await waitUntil { self.review(in: model, id: self.record.submissionID) != nil }
        model.testSkill(from: record)
        try await waitUntil { self.evaluation(in: model, id: self.record.submissionID) != nil }
        XCTAssertTrue(try XCTUnwrap(evaluation(in: model, id: record.submissionID)).installAllowed)

        model.reviewSkillInstall(from: record)
        try await waitUntil { self.installPlan(in: model, id: self.record.submissionID) != nil }
        model.installSkill(from: record)
        try await waitUntil { self.installedSkill(in: model, id: self.record.submissionID) != nil }
        model.rollbackSkill(from: record)
        try await waitUntil {
            self.evaluation(in: model, id: self.record.submissionID) != nil
                && !model.skillLearningState(for: self.record.submissionID).isWorking
        }
        XCTAssertNil(model.skillLearningState(for: record.submissionID).failure)
    }

    @MainActor
    func testMismatchedSuccessResponsesClearBusyStateAndKeepTheRecoverablePhase() async throws {
        let model = AppModel()
        model.setClientForTesting(client)
        let mismatch = "99999999-9999-4999-8999-999999999999"
        let expectedFailure = TCSkillLearning.errorLine(label: "skill-source-lineage-invalid")

        model.learnSkill(from: record)
        try await waitUntil { self.candidate(in: model, id: self.record.submissionID) != nil }
        let draft = try XCTUnwrap(candidate(in: model, id: record.submissionID)).draft

        daemon.setResponseField("skill_review", field: "candidate_id", value: mismatch)
        model.reviewSkill(from: record, draft: draft)
        try await waitUntil {
            model.skillLearningState(for: self.record.submissionID).failure != nil
                && !model.skillLearningState(for: self.record.submissionID).isWorking
        }
        guard case .candidate = model.skillLearningState(for: record.submissionID).phase else {
            return XCTFail("A mismatched review must return to its candidate")
        }
        XCTAssertEqual(model.skillLearningState(for: record.submissionID).failure, expectedFailure)

        daemon.setResponseField("skill_review", field: "candidate_id", value: nil)
        model.reviewSkill(from: record, draft: draft)
        try await waitUntil {
            self.review(in: model, id: self.record.submissionID) != nil
                && !model.skillLearningState(for: self.record.submissionID).isWorking
        }

        daemon.setResponseField("skill_evaluate", field: "review_id", value: mismatch)
        model.testSkill(from: record)
        try await waitUntil {
            model.skillLearningState(for: self.record.submissionID).failure != nil
                && !model.skillLearningState(for: self.record.submissionID).isWorking
        }
        guard case .reviewed = model.skillLearningState(for: record.submissionID).phase else {
            return XCTFail("A mismatched report must return to its review")
        }
        XCTAssertEqual(model.skillLearningState(for: record.submissionID).failure, expectedFailure)

        daemon.setResponseField("skill_evaluate", field: "review_id", value: nil)
        model.testSkill(from: record)
        try await waitUntil {
            self.evaluation(in: model, id: self.record.submissionID) != nil
                && !model.skillLearningState(for: self.record.submissionID).isWorking
        }

        daemon.setResponseField("skill_install_plan", field: "evaluation_id", value: mismatch)
        model.reviewSkillInstall(from: record)
        try await waitUntil {
            model.skillLearningState(for: self.record.submissionID).failure != nil
                && !model.skillLearningState(for: self.record.submissionID).isWorking
        }
        guard case .evaluated = model.skillLearningState(for: record.submissionID).phase else {
            return XCTFail("A mismatched installation plan must return to its evaluation")
        }
        XCTAssertEqual(model.skillLearningState(for: record.submissionID).failure, expectedFailure)

        daemon.setResponseField("skill_install_plan", field: "evaluation_id", value: nil)
        model.reviewSkillInstall(from: record)
        try await waitUntil {
            self.installPlan(in: model, id: self.record.submissionID) != nil
                && !model.skillLearningState(for: self.record.submissionID).isWorking
        }

        daemon.setResponseField("skill_install_commit", field: "evaluation_id", value: mismatch)
        model.installSkill(from: record)
        try await waitUntil {
            self.installedSkill(in: model, id: self.record.submissionID) != nil
                && !model.skillLearningState(for: self.record.submissionID).isWorking
        }
        guard case .evaluated = model.skillLearningState(for: record.submissionID).phase else {
            return XCTFail("Reconciled installation must retain its evaluated source")
        }
        XCTAssertNil(model.skillLearningState(for: record.submissionID).failure)
        XCTAssertEqual(
            daemon.calls.suffix(2).map(\.method),
            ["skill_install_commit", "skill_install_status"]
        )
    }

    @MainActor
    func testApprovedCustomDraftSurvivesEdit() async throws {
        let model = AppModel()
        model.setClientForTesting(client)
        let customDraft = SkillDraft(
            name: "repair-generated-artifacts",
            description: "Use when an artifact is generated from a source definition.",
            procedure: "# Procedure\n\nEdit the source definition, regenerate, and run drift checks."
        )

        model.learnSkill(from: record)
        try await waitUntil { self.candidate(in: model, id: self.record.submissionID) != nil }
        model.reviewSkill(from: record, draft: customDraft)
        try await waitUntil { self.review(in: model, id: self.record.submissionID) != nil }
        XCTAssertEqual(review(in: model, id: record.submissionID)?.draft, customDraft)

        model.editSkill(from: record)
        guard case .candidate(let candidate) = model.skillLearningState(for: record.submissionID).phase
        else {
            return XCTFail("Edit must restore the reviewed candidate")
        }
        XCTAssertEqual(candidate.draft, customDraft)

        model.reviewSkill(from: record, draft: customDraft)
        try await waitUntil {
            self.review(in: model, id: self.record.submissionID) != nil
                && !model.skillLearningState(for: self.record.submissionID).isWorking
        }
        let replacementCall = try XCTUnwrap(
            daemon.calls.last { $0.method == "skill_review" }
        )
        let replacementParams = try XCTUnwrap(
            JSONSerialization.jsonObject(with: Data(replacementCall.params.utf8))
                as? [String: Any]
        )
        XCTAssertEqual(
            replacementParams["replaces_review_id"] as? String,
            "44444444-4444-4444-8444-444444444444"
        )
    }

    @MainActor
    func testFreshClientCarriesRecoveredReviewIdentityIntoPaidEvaluationReuse() async throws {
        let reviewID = "44444444-4444-4444-8444-444444444444"
        let customDraft = SkillDraft(
            name: "repair-generated-artifacts",
            description: "Use when an artifact is generated from a source definition.",
            procedure: "# Procedure\n\nEdit the source definition, regenerate, and run drift checks."
        )
        daemon.seedRecoveredReview(draft: customDraft, reviewID: reviewID)
        let model = AppModel()
        model.setClientForTesting(client)

        model.learnSkill(from: record)
        try await waitUntil {
            self.candidate(in: model, id: self.record.submissionID) != nil
                && !model.skillLearningState(for: self.record.submissionID).isWorking
        }
        let recovered = try XCTUnwrap(candidate(in: model, id: record.submissionID))
        XCTAssertEqual(recovered.draft, customDraft)
        XCTAssertEqual(recovered.replacesReviewID, reviewID)
        XCTAssertEqual(
            model.skillLearningState(for: record.submissionID).replacesReviewID,
            reviewID
        )

        model.reviewSkill(from: record, draft: recovered.draft)
        try await waitUntil { self.review(in: model, id: self.record.submissionID) != nil }
        let replacementCall = try XCTUnwrap(
            daemon.calls.last { $0.method == "skill_review" }
        )
        let replacementParams = try XCTUnwrap(
            JSONSerialization.jsonObject(with: Data(replacementCall.params.utf8))
                as? [String: Any]
        )
        XCTAssertEqual(replacementParams["replaces_review_id"] as? String, reviewID)

        model.testSkill(from: record)
        try await waitUntil { self.evaluation(in: model, id: self.record.submissionID) != nil }
        XCTAssertEqual(evaluation(in: model, id: record.submissionID)?.reviewID, reviewID)
        XCTAssertEqual(
            daemon.calls.filter { $0.method == "skill_evaluate" }.count,
            1
        )
    }

    @MainActor
    func testExpiredOpaqueIDsResetToIdleAndPreserveDraftAcrossTheNextLearn() async throws {
        let cases = [
            (method: "skill_review", label: "skill-candidate-unknown"),
            (method: "skill_evaluate", label: "skill-review-unknown"),
            (method: "skill_install_plan", label: "skill-evaluation-unknown"),
        ]

        for (index, testCase) in cases.enumerated() {
            let localDaemon = SkillLearningDaemon()
            let localClient = DaemonClient(daemon: localDaemon)
            let model = AppModel()
            model.setClientForTesting(localClient)
            let customDraft = SkillDraft(
                name: "repair-generated-artifacts-\(index)",
                description: "Use when generated output has an authoritative source.",
                procedure: "# Procedure\n\nChange the source, regenerate, and verify drift."
            )

            model.learnSkill(from: record)
            try await waitUntil { self.candidate(in: model, id: self.record.submissionID) != nil }

            if index == 0 {
                localDaemon.setFailure(testCase.method, label: testCase.label)
                model.reviewSkill(from: record, draft: customDraft)
            } else {
                model.reviewSkill(from: record, draft: customDraft)
                try await waitUntil { self.review(in: model, id: self.record.submissionID) != nil }
                if index == 1 {
                    localDaemon.setFailure(testCase.method, label: testCase.label)
                    model.testSkill(from: record)
                } else {
                    model.testSkill(from: record)
                    try await waitUntil {
                        self.evaluation(in: model, id: self.record.submissionID) != nil
                    }
                    localDaemon.setFailure(testCase.method, label: testCase.label)
                    model.reviewSkillInstall(from: record)
                }
            }

            try await waitUntil {
                if case .idle = model.skillLearningState(for: self.record.submissionID).phase {
                    return !model.skillLearningState(for: self.record.submissionID).isWorking
                }
                return false
            }
            XCTAssertEqual(
                model.skillLearningState(for: record.submissionID).failure,
                TCSkillLearning.errorLine(label: testCase.label)
            )

            localDaemon.setFailure(testCase.method, label: nil)
            model.learnSkill(from: record)
            try await waitUntil {
                self.candidate(in: model, id: self.record.submissionID) != nil
                    && !model.skillLearningState(for: self.record.submissionID).isWorking
            }
            XCTAssertEqual(candidate(in: model, id: record.submissionID)?.draft, customDraft)
        }
    }

    @MainActor
    func testExpiredDraftSurvivesAFailedRelearnBeforeRecovery() async throws {
        let model = AppModel()
        model.setClientForTesting(client)
        let customDraft = SkillDraft(
            name: "repair-generated-artifacts",
            description: "Use when generated output has an authoritative source.",
            procedure: "# Procedure\n\nChange the source, regenerate, and verify drift."
        )

        model.learnSkill(from: record)
        try await waitUntil { self.candidate(in: model, id: self.record.submissionID) != nil }
        model.reviewSkill(from: record, draft: customDraft)
        try await waitUntil { self.review(in: model, id: self.record.submissionID) != nil }
        daemon.setFailure("skill_evaluate", label: "skill-review-unknown")
        model.testSkill(from: record)
        try await waitUntil {
            if case .idle = model.skillLearningState(for: self.record.submissionID).phase {
                return !model.skillLearningState(for: self.record.submissionID).isWorking
            }
            return false
        }

        daemon.setFailure("skill_evaluate", label: nil)
        daemon.setFailure("skill_candidate", label: "skill-candidate-unavailable")
        model.learnSkill(from: record)
        try await waitUntil {
            model.skillLearningState(for: self.record.submissionID).failure
                == TCSkillLearning.errorLine(label: "skill-candidate-unavailable")
                && !model.skillLearningState(for: self.record.submissionID).isWorking
        }

        daemon.setFailure("skill_candidate", label: nil)
        model.learnSkill(from: record)
        try await waitUntil {
            self.candidate(in: model, id: self.record.submissionID) != nil
                && !model.skillLearningState(for: self.record.submissionID).isWorking
        }
        XCTAssertEqual(candidate(in: model, id: record.submissionID)?.draft, customDraft)
    }

    @MainActor
    func testInitialRecoveryRejectsContradictoryInstallStatusShapes() async throws {
        let cases = [
            (installed: true, includesSkill: false),
            (installed: false, includesSkill: true),
        ]

        for shape in cases {
            let localDaemon = SkillLearningDaemon()
            localDaemon.setInstallStatusShape(
                installed: shape.installed,
                includesSkill: shape.includesSkill
            )
            let model = AppModel()
            model.setClientForTesting(DaemonClient(daemon: localDaemon))

            model.learnSkill(from: record)
            try await waitUntil {
                model.skillLearningState(for: self.record.submissionID).failure
                    == TCSkillLearning.errorLine(label: "skill-source-lineage-invalid")
                    && !model.skillLearningState(for: self.record.submissionID).isWorking
            }
            guard case .idle = model.skillLearningState(for: record.submissionID).phase else {
                return XCTFail("A contradictory install status must not enter an installed phase")
            }
            XCTAssertEqual(
                localDaemon.calls.map(\.method),
                ["skill_install_status"]
            )
        }
    }

    @MainActor
    func testLocalInstallRecoveryRejectsAnotherSourceLineage() async throws {
        let withdrawn = record(status: "withdrawn")
        daemon.seedInstalledSkill(
            name: "repair-generated-artifacts",
            sourceSubmissionID: "99999999-9999-4999-8999-999999999999"
        )
        daemon.setInstallStatusShape(installed: true, includesSkill: true)
        let model = AppModel()
        model.setClientForTesting(client)

        model.ensureLocalInstalledSkillStatus(for: withdrawn)
        try await waitUntil {
            model.skillLearningState(for: withdrawn.submissionID).failure
                == TCSkillLearning.errorLine(label: "skill-source-lineage-invalid")
                && !model.skillLearningState(for: withdrawn.submissionID).isWorking
        }
        guard case .idle = model.skillLearningState(for: withdrawn.submissionID).phase else {
            return XCTFail("Another source's installation must not be shown or rolled back")
        }
    }

    @MainActor
    func testRollbackUnknownReconcilesADeletedInstallationThroughStatus() async throws {
        let model = AppModel()
        model.setClientForTesting(client)

        model.learnSkill(from: record)
        try await waitUntil { self.candidate(in: model, id: self.record.submissionID) != nil }
        model.reviewSkill(
            from: record,
            draft: try XCTUnwrap(candidate(in: model, id: record.submissionID)).draft
        )
        try await waitUntil { self.review(in: model, id: self.record.submissionID) != nil }
        model.testSkill(from: record)
        try await waitUntil { self.evaluation(in: model, id: self.record.submissionID) != nil }
        model.reviewSkillInstall(from: record)
        try await waitUntil { self.installPlan(in: model, id: self.record.submissionID) != nil }
        model.installSkill(from: record)
        try await waitUntil { self.installedSkill(in: model, id: self.record.submissionID) != nil }

        let statusCallsBeforeRollback = daemon.calls.filter {
            $0.method == "skill_install_status"
        }.count
        daemon.setInstalled(false)
        daemon.setFailure("skill_install_rollback", label: "skill-install-unknown")
        model.rollbackSkill(from: record)
        try await waitUntil {
            self.evaluation(in: model, id: self.record.submissionID) != nil
                && !model.skillLearningState(for: self.record.submissionID).isWorking
        }

        XCTAssertNil(model.skillLearningState(for: record.submissionID).failure)
        XCTAssertEqual(
            daemon.calls.filter { $0.method == "skill_install_status" }.count,
            statusCallsBeforeRollback + 1
        )
    }

    @MainActor
    func testOccupiedInstallCanBeRetriedWithoutRepeatingEvaluation() async throws {
        daemon.setInstallOccupied(true)
        let model = AppModel()
        model.setClientForTesting(client)

        model.learnSkill(from: record)
        try await waitUntil { self.candidate(in: model, id: self.record.submissionID) != nil }
        model.reviewSkill(
            from: record,
            draft: try XCTUnwrap(candidate(in: model, id: record.submissionID)).draft
        )
        try await waitUntil { self.review(in: model, id: self.record.submissionID) != nil }
        model.testSkill(from: record)
        try await waitUntil { self.evaluation(in: model, id: self.record.submissionID) != nil }
        model.reviewSkillInstall(from: record)
        try await waitUntil {
            self.installPlan(in: model, id: self.record.submissionID)?.occupied == true
        }

        daemon.setInstallOccupied(false)
        model.reviewSkillInstall(from: record)
        try await waitUntil {
            self.installPlan(in: model, id: self.record.submissionID)?.canInstall == true
        }

        XCTAssertEqual(
            daemon.calls.filter { $0.method == "skill_install_plan" }.count,
            2
        )
        XCTAssertEqual(
            daemon.calls.filter { $0.method == "skill_evaluate" }.count,
            1
        )
    }

    @MainActor
    func testFailedGateCannotCreateAnInstallationPlan() async throws {
        daemon.setGatePasses(false)
        let model = AppModel()
        model.setClientForTesting(client)
        model.learnSkill(from: record)
        try await waitUntil { self.candidate(in: model, id: self.record.submissionID) != nil }
        model.reviewSkill(
            from: record,
            draft: try XCTUnwrap(candidate(in: model, id: record.submissionID)).draft
        )
        try await waitUntil { self.review(in: model, id: self.record.submissionID) != nil }
        model.testSkill(from: record)
        try await waitUntil { self.evaluation(in: model, id: self.record.submissionID) != nil }

        let before = daemon.calls.count
        model.reviewSkillInstall(from: record)
        await Task.yield()
        XCTAssertEqual(daemon.calls.count, before)
        XCTAssertNil(installPlan(in: model, id: record.submissionID))
        guard case .evaluated = model.skillLearningState(for: record.submissionID).phase else {
            return XCTFail("A failed gate must remain in the evaluated phase")
        }
    }

    @MainActor
    func testEvaluationFailureClearsBusyStateAndUsesSharedCopy() async throws {
        let model = AppModel()
        model.setClientForTesting(client)
        model.learnSkill(from: record)
        try await waitUntil { self.candidate(in: model, id: self.record.submissionID) != nil }
        model.reviewSkill(
            from: record,
            draft: try XCTUnwrap(candidate(in: model, id: record.submissionID)).draft
        )
        try await waitUntil { self.review(in: model, id: self.record.submissionID) != nil }
        daemon.setFailure("skill_evaluate", label: "skill-evaluation-request-unavailable")

        model.testSkill(from: record)
        try await waitUntil { !model.skillLearningState(for: self.record.submissionID).isWorking }
        XCTAssertNil(evaluation(in: model, id: record.submissionID))
        XCTAssertEqual(
            model.skillLearningState(for: record.submissionID).failure,
            TCSkillLearning.errorLine(label: "skill-evaluation-request-unavailable")
        )

        daemon.setFailure("skill_evaluate", label: nil)
        model.testSkill(from: record)
        try await waitUntil { self.evaluation(in: model, id: self.record.submissionID) != nil }
        model.editSkill(from: record)
        guard case .candidate = model.skillLearningState(for: record.submissionID).phase else {
            return XCTFail("Edit must return a completed evaluation to its candidate")
        }
        XCTAssertNil(model.skillLearningState(for: record.submissionID).failure)
    }

    @MainActor
    func testTwoSessionWorkflowsStayIsolated() async throws {
        let first = record
        let second = record(submissionID: "88888888-8888-4888-8888-888888888888")
        let model = AppModel()
        model.setClientForTesting(client)

        model.learnSkill(from: first)
        model.learnSkill(from: second)
        try await waitUntil {
            self.candidate(in: model, id: first.submissionID) != nil
                && self.candidate(in: model, id: second.submissionID) != nil
        }

        let firstCandidate = try XCTUnwrap(candidate(in: model, id: first.submissionID))
        model.reviewSkill(from: first, draft: firstCandidate.draft)
        try await waitUntil { self.review(in: model, id: first.submissionID) != nil }

        guard case .reviewed = model.skillLearningState(for: first.submissionID).phase else {
            return XCTFail("The first session must advance independently")
        }
        guard case .candidate = model.skillLearningState(for: second.submissionID).phase else {
            return XCTFail("The second session must retain its candidate")
        }
        XCTAssertNil(model.skillLearningState(for: second.submissionID).failure)
        XCTAssertFalse(model.skillLearningState(for: second.submissionID).isWorking)
    }

    @MainActor
    func testCustomNameInstallationRecoversAndRollsBackBySourceLineage() async throws {
        let customName = "repair-generated-artifacts"
        daemon.seedInstalledSkill(
            name: customName,
            sourceSubmissionID: record.submissionID
        )
        let model = AppModel()
        model.setClientForTesting(client)

        model.learnSkill(from: record)
        try await waitUntil {
            self.installedSkill(in: model, id: self.record.submissionID) != nil
        }
        let installed = try XCTUnwrap(
            installedSkill(in: model, id: record.submissionID)
        )
        XCTAssertEqual(installed.name, customName)
        XCTAssertEqual(installed.sourceSubmissionID, record.submissionID)

        model.rollbackSkill(from: record)
        try await waitUntil {
            if case .idle = model.skillLearningState(for: self.record.submissionID).phase {
                return !model.skillLearningState(for: self.record.submissionID).isWorking
            }
            return false
        }
        let calls = daemon.calls
        let status = try XCTUnwrap(calls.first { $0.method == "skill_install_status" })
        let rollback = try XCTUnwrap(calls.first { $0.method == "skill_install_rollback" })
        XCTAssertTrue(status.params.contains(record.submissionID))
        XCTAssertFalse(status.params.contains(customName))
        XCTAssertTrue(rollback.params.contains(record.submissionID))
        XCTAssertTrue(rollback.params.contains(installed.installID))
    }

    @MainActor
    func testWithdrawnSessionRecoversLocalInstallAndRollsBackToIdle() async throws {
        let withdrawn = record(submissionID: record.submissionID, status: "withdrawn")
        daemon.seedInstalledSkill(
            name: "repair-generated-artifacts",
            sourceSubmissionID: withdrawn.submissionID
        )
        let model = AppModel()
        model.setClientForTesting(client)

        model.ensureLocalInstalledSkillStatus(for: withdrawn)
        try await waitUntil {
            self.installedSkill(in: model, id: withdrawn.submissionID) != nil
                && !model.skillLearningState(for: withdrawn.submissionID).isWorking
        }
        let recovered = model.skillLearningState(for: withdrawn.submissionID)
        guard let installed = recovered.installedSkill else {
            return XCTFail("A withdrawn source must recover its verified local installation")
        }
        guard case .idle = recovered.phase else {
            return XCTFail("Install recovery must not manufacture workflow progress")
        }
        XCTAssertEqual(installed.sourceSubmissionID, withdrawn.submissionID)
        XCTAssertEqual(daemon.calls.map(\.method), ["skill_install_status"])

        let detailImage = try await capture(
            SessionDetailView(record: withdrawn, onBack: {}).environmentObject(model),
            width: 760,
            scheme: .dark
        )
        XCTAssertGreaterThan(detailImage.count, 1_000)
        let rowWithSessionAction = try await capture(
            HistoryRow(record: withdrawn, onOpen: {})
                .frame(minHeight: 320, alignment: .top)
                .environmentObject(model),
            width: 760,
            scheme: .dark
        )
        let rowWithoutSessionAction = try await capture(
            HistoryRow(record: withdrawn)
                .frame(minHeight: 320, alignment: .top)
                .environmentObject(model),
            width: 760,
            scheme: .dark
        )
        XCTAssertNotEqual(rowWithSessionAction, rowWithoutSessionAction)
        try await waitUntil {
            !model.skillLearningState(for: withdrawn.submissionID).isWorking
        }

        model.rollbackSkill(from: withdrawn)
        try await waitUntil {
            if case .idle = model.skillLearningState(for: withdrawn.submissionID).phase {
                return !model.skillLearningState(for: withdrawn.submissionID).isWorking
            }
            return false
        }
        XCTAssertNil(model.skillLearningState(for: withdrawn.submissionID).failure)
        XCTAssertTrue(daemon.calls.contains { $0.method == "skill_install_rollback" })
        XCTAssertFalse(daemon.calls.contains { $0.method == "skill_candidate" })
    }

    @MainActor
    private func waitUntil(
        timeout: Duration = .seconds(3),
        condition: @escaping @MainActor () -> Bool
    ) async throws {
        let clock = ContinuousClock()
        let deadline = clock.now.advanced(by: timeout)
        while !condition() {
            if clock.now >= deadline {
                XCTFail("Timed out waiting for tested-skill state")
                return
            }
            try await Task.sleep(for: .milliseconds(20))
        }
    }

    @MainActor
    private func capture<V: View>(
        _ view: V,
        width: CGFloat,
        scheme: ColorScheme
    ) async throws -> Data {
        _ = NSApplication.shared
        let content = view
            .padding(24)
            .frame(width: width, alignment: .topLeading)
            .fixedSize(horizontal: false, vertical: true)
            .background(Color(nsColor: .windowBackgroundColor))
            .environment(\.colorScheme, scheme)
        let hosting = NSHostingView(rootView: content)
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: width, height: 1_600),
            styleMask: [.borderless],
            backing: .buffered,
            defer: false
        )
        window.isReleasedWhenClosed = false
        window.appearance = NSAppearance(named: scheme == .dark ? .darkAqua : .aqua)
        window.contentView = hosting
        defer { window.close() }
        await Task.yield()
        hosting.layoutSubtreeIfNeeded()
        let height = ceil(hosting.fittingSize.height)
        XCTAssertGreaterThan(height, 300)
        XCTAssertLessThan(height, 4_500)
        let bounds = NSRect(x: 0, y: 0, width: width, height: height)
        window.setContentSize(bounds.size)
        hosting.frame = bounds
        hosting.layoutSubtreeIfNeeded()
        window.displayIfNeeded()
        let bitmap = try XCTUnwrap(hosting.bitmapImageRepForCachingDisplay(in: bounds))
        hosting.cacheDisplay(in: bounds, to: bitmap)
        return try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
    }

    @MainActor
    private func candidate(in model: AppModel, id: String) -> SkillCandidate? {
        model.skillLearningState(for: id).phase.candidate
    }

    @MainActor
    private func review(in model: AppModel, id: String) -> SkillReview? {
        switch model.skillLearningState(for: id).phase {
        case .reviewed(_, let review),
             .evaluated(_, let review, _),
             .planned(_, let review, _, _):
            return review
        case .idle, .candidate:
            return nil
        }
    }

    @MainActor
    private func evaluation(in model: AppModel, id: String) -> SkillEvaluationReport? {
        switch model.skillLearningState(for: id).phase {
        case .evaluated(_, _, let report), .planned(_, _, let report, _):
            return report
        case .idle, .candidate, .reviewed:
            return nil
        }
    }

    @MainActor
    private func installPlan(in model: AppModel, id: String) -> SkillInstallPlan? {
        guard case .planned(_, _, _, let plan) = model.skillLearningState(for: id).phase else {
            return nil
        }
        return plan
    }

    @MainActor
    private func installedSkill(in model: AppModel, id: String) -> InstalledSkill? {
        model.skillLearningState(for: id).installedSkill
    }

    private var record: HistoryRecord {
        record(submissionID: "22222222-2222-4222-8222-222222222222")
    }

    private func record(status: String) -> HistoryRecord {
        record(submissionID: "22222222-2222-4222-8222-222222222222", status: status)
    }

    private func record(submissionID: String, status: String = "accepted") -> HistoryRecord {
        HistoryRecord(
            submissionID: submissionID,
            submittedAt: Date(timeIntervalSince1970: 1_789_000_000),
            projectID: "project-test",
            projectLabel: "Synthetic project",
            source: "codex",
            status: status,
            consentScopes: ["code_content"],
            creditPointsPending: 0,
            creditPointsFinal: 1,
            explanations: [],
            lastRefreshedAt: nil
        )
    }
}
