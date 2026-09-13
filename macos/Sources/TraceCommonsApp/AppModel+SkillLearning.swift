// INTEGRATION: extends AppModel with the tested-skill workflow while the model
// retains ownership of its published state and private daemon client.

import Foundation
import TCBridge

extension AppModel {
    func learnSkill(from record: HistoryRecord) {
        guard let client = skillLearningClient else { return }
        let id = record.submissionID
        if skillLearningStore.installStatusNeedsResolution(for: id) {
            ensureLocalInstalledSkillStatus(for: record, continueLearningWhenAbsent: true)
            return
        }
        guard skillLearningStore.installStatusIsAbsent(for: id) else { return }
        guard skillLearningStore.beginLearning(for: id) else { return }
        Task.detached(priority: .userInitiated) {
            let result = Result {
                let candidate = try client.skillCandidate(submissionID: id)
                guard candidate.sourceSubmissionID == id else {
                    throw DaemonClient.Failure(
                        code: "unavailable",
                        message: "skill-source-lineage-invalid"
                    )
                }
                return candidate
            }
            await MainActor.run {
                switch result {
                case .success(let candidate):
                    let applied = self.skillLearningStore.finishLearning(
                        for: id,
                        candidate: candidate
                    )
                    if !applied {
                        self.skillLearningStore.finishFailure(
                            for: id,
                            operation: .learning,
                            message: self.skillLearningError("skill-source-lineage-invalid")
                        )
                    }
                case .failure(let error):
                    let label = (error as? DaemonClient.Failure)?.message ?? ""
                    self.skillLearningStore.finishFailure(
                        for: id,
                        operation: .learning,
                        message: self.skillLearningError(label)
                    )
                }
            }
        }
    }

    func ensureLocalInstalledSkillStatus(for record: HistoryRecord) {
        ensureLocalInstalledSkillStatus(for: record, continueLearningWhenAbsent: false)
    }

    private func ensureLocalInstalledSkillStatus(
        for record: HistoryRecord,
        continueLearningWhenAbsent: Bool
    ) {
        guard let client = skillLearningClient else { return }
        let id = record.submissionID
        guard skillLearningStore.beginInstallStatusLoading(for: id) else { return }
        Task.detached(priority: .userInitiated) {
            let result = Result {
                let status = try client.skillInstallStatus(sourceSubmissionID: id)
                return try Self.validatedInstalledSkill(
                    in: status,
                    sourceSubmissionID: id
                )
            }
            await MainActor.run {
                switch result {
                case .success(let installed):
                    let applied = self.skillLearningStore.finishInstallStatusLoading(
                        for: id,
                        installed: installed
                    )
                    if !applied {
                        _ = self.skillLearningStore.finishInstallStatusFailure(
                            for: id,
                            message: self.skillLearningError("skill-source-lineage-invalid")
                        )
                    } else if installed == nil, continueLearningWhenAbsent {
                        self.learnSkill(from: record)
                    }
                case .failure(let error):
                    let label = (error as? DaemonClient.Failure)?.message ?? ""
                    _ = self.skillLearningStore.finishInstallStatusFailure(
                        for: id,
                        message: self.skillLearningError(label)
                    )
                }
            }
        }
    }

    func reviewSkill(from record: HistoryRecord, draft: SkillDraft) {
        guard let client = skillLearningClient else { return }
        let id = record.submissionID
        guard let start = skillLearningStore.beginReview(for: id, draft: draft) else { return }
        Task.detached(priority: .userInitiated) {
            let result = Result {
                try client.reviewSkill(
                    candidateID: start.candidate.candidateID,
                    draft: start.candidate.draft,
                    replacesReviewID: start.replacesReviewID
                )
            }
            await MainActor.run {
                switch result {
                case .success(let review):
                    if !self.skillLearningStore.finishReview(for: id, review: review) {
                        self.skillLearningStore.finishFailure(
                            for: id,
                            operation: .reviewing,
                            message: self.skillLearningError("skill-source-lineage-invalid")
                        )
                    }
                case .failure(let error):
                    let label = (error as? DaemonClient.Failure)?.message ?? ""
                    self.finishSkillLearningFailure(for: id, operation: .reviewing, label: label)
                }
            }
        }
    }

    func testSkill(from record: HistoryRecord) {
        guard let client = skillLearningClient else { return }
        let id = record.submissionID
        guard let review = skillLearningStore.beginEvaluation(for: id) else { return }
        Task.detached(priority: .userInitiated) {
            let result = Result {
                try client.evaluateSkill(
                    reviewID: review.reviewID,
                    skillSHA256: review.skillSHA256
                )
            }
            await MainActor.run {
                switch result {
                case .success(let report):
                    if !self.skillLearningStore.finishEvaluation(for: id, report: report) {
                        self.skillLearningStore.finishFailure(
                            for: id,
                            operation: .evaluating,
                            message: self.skillLearningError("skill-source-lineage-invalid")
                        )
                    }
                case .failure(let error):
                    let label = (error as? DaemonClient.Failure)?.message ?? ""
                    self.finishSkillLearningFailure(for: id, operation: .evaluating, label: label)
                }
            }
        }
    }

    func reviewSkillInstall(from record: HistoryRecord) {
        guard let client = skillLearningClient else { return }
        let id = record.submissionID
        if skillLearningStore.installStatusNeedsResolution(for: id) {
            ensureLocalInstalledSkillStatus(for: record)
            return
        }
        guard let evaluation = skillLearningStore.beginInstallPlanning(for: id) else { return }
        Task.detached(priority: .userInitiated) {
            let result = Result {
                try client.skillInstallPlan(evaluationID: evaluation.evaluationID)
            }
            await MainActor.run {
                switch result {
                case .success(let plan):
                    if !self.skillLearningStore.finishInstallPlanning(for: id, plan: plan) {
                        self.skillLearningStore.finishFailure(
                            for: id,
                            operation: .planningInstall,
                            message: self.skillLearningError("skill-source-lineage-invalid")
                        )
                    }
                case .failure(let error):
                    let label = (error as? DaemonClient.Failure)?.message ?? ""
                    self.finishSkillLearningFailure(
                        for: id,
                        operation: .planningInstall,
                        label: label
                    )
                }
            }
        }
    }

    func installSkill(from record: HistoryRecord) {
        guard let client = skillLearningClient else { return }
        let id = record.submissionID
        guard let plan = skillLearningStore.beginInstall(for: id) else { return }
        Task.detached(priority: .userInitiated) {
            let commitResult = Result {
                try client.installSkill(
                    planID: plan.planID,
                    asPreviewedSHA256: plan.skillSHA256,
                    asPreviewedMarkerSHA256: plan.markerFileSHA256
                )
            }
            let statusResult = Result {
                let status = try client.skillInstallStatus(sourceSubmissionID: id)
                return try Self.validatedInstalledSkill(
                    in: status,
                    sourceSubmissionID: id
                )
            }
            await MainActor.run {
                var commitReceipt: InstalledSkill? = nil
                if case .success(let installed) = commitResult,
                   installed.sourceSubmissionID == id {
                    commitReceipt = installed
                }
                switch statusResult {
                case .success(let installed):
                    if let installed {
                        if !self.skillLearningStore.finishInstall(for: id, installed: installed) {
                            self.skillLearningStore.finishInstallReconciliationFailure(
                                for: id,
                                lastKnownInstalled: installed,
                                statusWasResolved: true,
                                message: self.skillLearningError("skill-source-lineage-invalid")
                            )
                        }
                    } else {
                        let label = if case .failure(let error) = commitResult {
                            (error as? DaemonClient.Failure)?.message ?? ""
                        } else {
                            "skill-source-lineage-invalid"
                        }
                        self.skillLearningStore.finishInstallReconciliationFailure(
                            for: id,
                            lastKnownInstalled: nil,
                            statusWasResolved: true,
                            message: self.skillLearningError(label)
                        )
                    }
                case .failure(let error):
                    let label = (error as? DaemonClient.Failure)?.message ?? ""
                    self.skillLearningStore.finishInstallReconciliationFailure(
                        for: id,
                        lastKnownInstalled: commitReceipt,
                        statusWasResolved: false,
                        message: self.skillLearningError(label)
                    )
                }
            }
        }
    }

    func rollbackSkill(from record: HistoryRecord) {
        guard let client = skillLearningClient else { return }
        let id = record.submissionID
        guard let installed = skillLearningStore.beginRollback(for: id) else { return }
        Task.detached(priority: .userInitiated) {
            let result = Result {
                do {
                    return try client.rollbackSkill(
                        installID: installed.installID,
                        sourceSubmissionID: installed.sourceSubmissionID
                    )
                } catch {
                    let label = (error as? DaemonClient.Failure)?.message ?? ""
                    guard SkillLearningDaemonFailure(label: label) == .installUnknown else {
                        throw error
                    }
                    let status = try client.skillInstallStatus(
                        sourceSubmissionID: installed.sourceSubmissionID
                    )
                    if try Self.validatedInstalledSkill(
                        in: status,
                        sourceSubmissionID: installed.sourceSubmissionID
                    ) != nil {
                        throw error
                    }
                    return SkillRollbackResult(removed: true, retainedDirectory: false)
                }
            }
            await MainActor.run {
                switch result {
                case .success(let rollback):
                    self.skillLearningStore.finishRollback(
                        for: id,
                        removed: rollback.removed,
                        incompleteMessage: self.skillLearningCopy?.rollbackIncomplete ?? ""
                    )
                case .failure(let error):
                    let label = (error as? DaemonClient.Failure)?.message ?? ""
                    self.skillLearningStore.finishFailure(
                        for: id,
                        operation: .rollingBack,
                        message: self.skillLearningError(label)
                    )
                }
            }
        }
    }

    func editSkill(from record: HistoryRecord) {
        _ = skillLearningStore.edit(for: record.submissionID)
    }

    func skillLearningState(for submissionID: String) -> SkillLearningSessionState {
        skillLearningStore.state(for: submissionID)
    }

    private func skillLearningError(_ label: String) -> String {
        TCSkillLearning.errorLine(label: label) ?? skillLearningCopy?.unavailable ?? ""
    }

    nonisolated private static func validatedInstalledSkill(
        in status: SkillInstallStatus,
        sourceSubmissionID: String
    ) throws -> InstalledSkill? {
        guard status.installed == (status.skill != nil),
              status.skill?.sourceSubmissionID == nil
                || status.skill?.sourceSubmissionID == sourceSubmissionID
        else {
            throw DaemonClient.Failure(
                code: "unavailable",
                message: "skill-source-lineage-invalid"
            )
        }
        return status.skill
    }

    private func finishSkillLearningFailure(
        for submissionID: String,
        operation: SkillLearningOperation,
        label: String
    ) {
        let message = skillLearningError(label)
        if SkillLearningDaemonFailure(label: label).invalidatesWorkflow {
            skillLearningStore.finishExpiredWorkflow(
                for: submissionID,
                operation: operation,
                message: message
            )
        } else {
            skillLearningStore.finishFailure(
                for: submissionID,
                operation: operation,
                message: message
            )
        }
    }
}
