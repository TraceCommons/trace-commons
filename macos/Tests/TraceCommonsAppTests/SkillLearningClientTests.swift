// INTEGRATION: verifies the tested-skill daemon client's wire contract apart
// from AppModel lifecycle and rendering coverage.

import Foundation
import TCBridge
import XCTest

@testable import TraceCommonsApp

final class SkillLearningClientTests: XCTestCase {
    private var daemon: SkillLearningDaemon!
    private var client: DaemonClient!

    override func setUp() {
        super.setUp()
        daemon = SkillLearningDaemon()
        client = DaemonClient(daemon: daemon)
    }

    func testClientUsesOpaqueIDsAndTheExactReviewedDigestAcrossEveryMutation() throws {
        let candidate = try client.skillCandidate(submissionID: record.submissionID)
        XCTAssertEqual(candidate.evaluationContract.taskCount, 8)
        XCTAssertEqual(candidate.evaluationContract.planTaskCount, 6)
        XCTAssertEqual(candidate.evaluationContract.fixturePoolCount, 7)
        XCTAssertEqual(candidate.evaluationContract.clusterCount, 7)
        XCTAssertEqual(candidate.evaluationContract.armCount, 3)
        XCTAssertEqual(candidate.evaluationContract.totalRequests, 24)
        XCTAssertEqual(candidate.evaluationContract.requiredModelOwner, "nearai")
        XCTAssertNil(candidate.replacesReviewID)
        let review = try client.reviewSkill(candidateID: candidate.candidateID, draft: candidate.draft)
        let evaluation = try client.evaluateSkill(
            reviewID: review.reviewID,
            skillSHA256: review.skillSHA256
        )
        XCTAssertEqual(evaluation.taskCount, 8)
        XCTAssertEqual(evaluation.planTaskCount, 6)
        XCTAssertEqual(evaluation.fixturePoolCount, 7)
        XCTAssertEqual(evaluation.clusterCount, 7)
        XCTAssertEqual(evaluation.evaluationContract, candidate.evaluationContract)
        XCTAssertEqual(evaluation.armCount, 3)
        XCTAssertEqual(evaluation.sourceTaskFingerprintSHA256, String(repeating: "e", count: 64))
        XCTAssertEqual(evaluation.trials.count, 24)
        XCTAssertEqual(Set(evaluation.trials.map(\.id)).count, 24)
        XCTAssertEqual(Set(evaluation.trials.map(\.cluster)).count, 7)
        XCTAssertTrue(evaluation.planSummaries.allSatisfy { $0.total == 6 })
        XCTAssertTrue(evaluation.applicabilitySummaries.allSatisfy { $0.total == 2 })
        XCTAssertEqual(
            evaluation.applicabilitySummaries.first { $0.arm == .candidateSkill }?.passed,
            2
        )
        XCTAssertTrue(evaluation.trials.allSatisfy { trial in
            trial.sourceURL.scheme == "https"
                && !trial.rawOutput.isEmpty
                && !(trial.requestID ?? "").isEmpty
        })
        let trialsByTask = Dictionary(grouping: evaluation.trials, by: \.taskID)
        XCTAssertEqual(trialsByTask.count, 8)
        for trials in trialsByTask.values {
            XCTAssertEqual(
                Set(trials.map(\.arm)),
                Set([.baseline, .manualInstruction, .candidateSkill])
            )
        }
        let applicabilityTrials = evaluation.trials.filter {
            $0.cluster == "metadata-applicability"
        }
        XCTAssertEqual(applicabilityTrials.count, 6)
        XCTAssertTrue(applicabilityTrials.allSatisfy { $0.answer == nil })
        let planTrials = evaluation.trials.filter { $0.cluster != "metadata-applicability" }
        XCTAssertEqual(planTrials.count, 18)
        XCTAssertTrue(planTrials.allSatisfy { $0.answer != nil })
        for summary in evaluation.summaries {
            let matching = evaluation.trials.filter { $0.arm == summary.arm }
            XCTAssertEqual(summary.total, matching.count)
            XCTAssertEqual(summary.passed, matching.filter(\.passed).count)
        }
        let summaries = Dictionary(uniqueKeysWithValues: evaluation.summaries.map {
            ($0.arm, $0.passed)
        })
        let candidatePasses = try XCTUnwrap(summaries[.candidateSkill])
        XCTAssertGreaterThan(candidatePasses, try XCTUnwrap(summaries[.baseline]))
        XCTAssertGreaterThan(candidatePasses, try XCTUnwrap(summaries[.manualInstruction]))
        XCTAssertEqual(
            evaluation.trials.first { $0.taskID == "mark-msix-scale-ladder" }?.sourceURL.absoluteString,
            "https://github.com/TraceCommons/trace-commons/commit/b6722426bb4b83d90425494b664ac468d67943b5"
        )
        let plan = try client.skillInstallPlan(evaluationID: evaluation.evaluationID)
        XCTAssertEqual(plan.skillLocation, plan.targetLocation + "/SKILL.md")
        XCTAssertEqual(
            plan.markerLocation,
            plan.targetLocation + "/.trace-commons-install.json"
        )
        XCTAssertFalse(plan.targetLocation.contains("/Users/"))
        XCTAssertEqual(plan.markerFileSHA256.count, 64)
        XCTAssertTrue(plan.markerJSON.contains(#""source_submission_id""#))
        XCTAssertTrue(plan.markerJSON.contains(candidate.sourceSubmissionID))
        let installed = try client.installSkill(
            planID: plan.planID,
            asPreviewedSHA256: plan.skillSHA256,
            asPreviewedMarkerSHA256: plan.markerFileSHA256
        )
        let status = try client.skillInstallStatus(
            sourceSubmissionID: candidate.sourceSubmissionID
        )
        XCTAssertEqual(status.skill, installed)
        XCTAssertTrue(
            try client.rollbackSkill(
                installID: installed.installID,
                sourceSubmissionID: installed.sourceSubmissionID
            ).removed
        )

        let calls = daemon.calls
        XCTAssertEqual(
            calls.map(\.method),
            [
                "skill_candidate", "skill_review", "skill_evaluate", "skill_install_plan",
                "skill_install_commit", "skill_install_status", "skill_install_rollback",
            ]
        )
        let parameterSets = try calls.map { call in
            try XCTUnwrap(
                JSONSerialization.jsonObject(with: Data(call.params.utf8)) as? [String: Any]
            )
        }
        XCTAssertEqual(Set(parameterSets[0].keys), ["submission_id"])
        XCTAssertEqual(Set(parameterSets[2].keys), ["review_id", "skill_sha256"])
        XCTAssertEqual(
            Set(parameterSets[4].keys),
            ["plan_id", "as_previewed_sha256", "as_previewed_marker_sha256"]
        )
        XCTAssertEqual(
            parameterSets[4]["as_previewed_marker_sha256"] as? String,
            plan.markerFileSHA256
        )
        XCTAssertEqual(Set(parameterSets[5].keys), ["source_submission_id"])
        XCTAssertEqual(
            Set(parameterSets[6].keys),
            ["install_id", "source_submission_id"]
        )
        XCTAssertFalse(calls.dropFirst().contains { $0.params.contains(candidate.sourceCorrection) })
    }

    private var record: HistoryRecord {
        HistoryRecord(
            submissionID: "22222222-2222-4222-8222-222222222222",
            submittedAt: Date(timeIntervalSince1970: 1_789_000_000),
            projectID: "project-test",
            projectLabel: "Synthetic project",
            source: "codex",
            status: "accepted",
            consentScopes: ["code_content"],
            creditPointsPending: 0,
            creditPointsFinal: 1,
            explanations: [],
            lastRefreshedAt: nil
        )
    }
}
