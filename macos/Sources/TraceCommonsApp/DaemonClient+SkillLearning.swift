// INTEGRATION: keeps the tested-skill IPC surface out of the general daemon
// client while reusing its single typed JSON framing boundary.

import Foundation

extension DaemonClient {
    func skillCandidate(submissionID: String) throws -> SkillCandidate {
        try call(
            "skill_candidate",
            params: ["submission_id": submissionID],
            as: SkillCandidate.self
        )
    }

    func reviewSkill(
        candidateID: String,
        draft: SkillDraft,
        replacesReviewID: String? = nil
    ) throws -> SkillReview {
        var params: [String: Any] = [
            "candidate_id": candidateID,
            "draft": [
                "name": draft.name,
                "description": draft.description,
                "procedure": draft.procedure,
            ],
        ]
        if let replacesReviewID {
            params["replaces_review_id"] = replacesReviewID
        }
        return try call(
            "skill_review",
            params: params,
            as: SkillReview.self
        )
    }

    func evaluateSkill(reviewID: String, skillSHA256: String) throws -> SkillEvaluationReport {
        try call(
            "skill_evaluate",
            params: ["review_id": reviewID, "skill_sha256": skillSHA256],
            as: SkillEvaluationReport.self
        )
    }

    func skillInstallPlan(evaluationID: String) throws -> SkillInstallPlan {
        try call(
            "skill_install_plan",
            params: ["evaluation_id": evaluationID],
            as: SkillInstallPlan.self
        )
    }

    func installSkill(
        planID: String,
        asPreviewedSHA256: String,
        asPreviewedMarkerSHA256: String
    ) throws -> InstalledSkill {
        try call(
            "skill_install_commit",
            params: [
                "plan_id": planID,
                "as_previewed_sha256": asPreviewedSHA256,
                "as_previewed_marker_sha256": asPreviewedMarkerSHA256,
            ],
            as: InstalledSkill.self
        )
    }

    func skillInstallStatus(sourceSubmissionID: String) throws -> SkillInstallStatus {
        try call(
            "skill_install_status",
            params: ["source_submission_id": sourceSubmissionID],
            as: SkillInstallStatus.self
        )
    }

    func rollbackSkill(
        installID: String,
        sourceSubmissionID: String
    ) throws -> SkillRollbackResult {
        try call(
            "skill_install_rollback",
            params: [
                "install_id": installID,
                "source_submission_id": sourceSubmissionID,
            ],
            as: SkillRollbackResult.self
        )
    }
}
