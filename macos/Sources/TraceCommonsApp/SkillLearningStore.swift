// INTEGRATION: owns the native tested-skill workflow as one typed state per
// session so invalid combinations cannot leak into AppModel or SwiftUI.

import Foundation

enum SkillLearningOperation: Equatable {
    case learning
    case reviewing
    case evaluating
    case planningInstall
    case installing
    case rollingBack
}

/// What the app has established about the source session's local installation.
/// An optional receipt cannot distinguish an unchecked path from a verified
/// absence, so those states remain explicit and status loading is idempotent.
enum SkillInstallationState: Equatable {
    case unknown
    case loading(previous: InstalledSkill?)
    case absent
    case installed(InstalledSkill)
    case failed(message: String, previous: InstalledSkill?)

    var installedSkill: InstalledSkill? {
        switch self {
        case .installed(let installed):
            return installed
        case .loading(let previous), .failed(_, let previous):
            return previous
        case .unknown, .absent:
            return nil
        }
    }

    var failure: String? {
        guard case .failed(let message, _) = self else { return nil }
        return message
    }

    var isLoading: Bool {
        if case .loading = self { return true }
        return false
    }

    var needsResolution: Bool {
        switch self {
        case .unknown, .failed:
            return true
        case .loading, .absent, .installed:
            return false
        }
    }

    var isAbsent: Bool {
        if case .absent = self { return true }
        return false
    }
}

enum SkillLearningDaemonFailure: Equatable {
    case candidateUnknown
    case reviewUnknown
    case evaluationUnknown
    case installUnknown
    case other(String)

    init(label: String) {
        switch label {
        case "skill-candidate-unknown": self = .candidateUnknown
        case "skill-review-unknown": self = .reviewUnknown
        case "skill-evaluation-unknown": self = .evaluationUnknown
        case "skill-install-unknown": self = .installUnknown
        default: self = .other(label)
        }
    }

    var invalidatesWorkflow: Bool {
        switch self {
        case .candidateUnknown, .reviewUnknown, .evaluationUnknown:
            return true
        case .installUnknown, .other:
            return false
        }
    }
}

struct SkillReviewStart: Equatable {
    let candidate: SkillCandidate
    let replacesReviewID: String?
}

enum SkillLearningPhase: Equatable {
    case idle
    case candidate(SkillCandidate)
    case reviewed(candidate: SkillCandidate, review: SkillReview)
    case evaluated(
        candidate: SkillCandidate,
        review: SkillReview,
        report: SkillEvaluationReport
    )
    case planned(
        candidate: SkillCandidate,
        review: SkillReview,
        report: SkillEvaluationReport,
        plan: SkillInstallPlan
    )

    var candidate: SkillCandidate? {
        switch self {
        case .idle:
            return nil
        case .candidate(let candidate),
             .reviewed(let candidate, _),
             .evaluated(let candidate, _, _),
             .planned(let candidate, _, _, _):
            return candidate
        }
    }

    var isReviewedAwaitingEvaluation: Bool {
        if case .reviewed = self { return true }
        return false
    }
}

struct SkillLearningSessionState: Equatable {
    var phase: SkillLearningPhase = .idle
    var installation: SkillInstallationState = .unknown
    var operation: SkillLearningOperation?
    var workflowFailure: String?
    var preservedDraft: SkillDraft?
    var replacesReviewID: String?

    var installedSkill: InstalledSkill? { installation.installedSkill }
    var failure: String? { workflowFailure ?? installation.failure }
    var isWorking: Bool { operation != nil || installation.isLoading }
}

@MainActor
struct SkillLearningStore {
    private static let maximumRetainedSessions = 24
    private var sessions: [String: SkillLearningSessionState] = [:]
    private var recency: [String] = []

    var retainedSessionCount: Int { sessions.count }

    func state(for submissionID: String) -> SkillLearningSessionState {
        sessions[submissionID] ?? SkillLearningSessionState()
    }

    mutating func beginLearning(for submissionID: String) -> Bool {
        let current = state(for: submissionID)
        guard current.installation.isAbsent,
              case .idle = current.phase
        else { return false }
        return begin(.learning, for: submissionID)
    }

    mutating func finishLearning(
        for submissionID: String,
        candidate: SkillCandidate
    ) -> Bool {
        var current = state(for: submissionID)
        guard current.operation == .learning,
              current.installation.isAbsent,
              candidate.sourceSubmissionID == submissionID
        else { return false }
        let loadedCandidate = if let draft = current.preservedDraft {
            candidate.replacingDraft(draft)
        } else {
            candidate
        }
        current.phase = .candidate(loadedCandidate)
        current.operation = nil
        current.workflowFailure = nil
        current.preservedDraft = nil
        current.replacesReviewID = candidate.replacesReviewID
        store(current, for: submissionID)
        return true
    }

    mutating func beginInstallStatusLoading(for submissionID: String) -> Bool {
        var current = state(for: submissionID)
        guard current.operation == nil, current.installation.needsResolution else { return false }
        current.installation = .loading(previous: current.installedSkill)
        current.workflowFailure = nil
        store(current, for: submissionID)
        return true
    }

    mutating func finishInstallStatusLoading(
        for submissionID: String,
        installed: InstalledSkill?
    ) -> Bool {
        var current = state(for: submissionID)
        guard case .loading = current.installation,
              installed?.sourceSubmissionID == nil
                || installed?.sourceSubmissionID == submissionID
        else { return false }
        current.installation = installed.map(SkillInstallationState.installed) ?? .absent
        current.workflowFailure = nil
        store(current, for: submissionID)
        return true
    }

    mutating func finishInstallStatusFailure(
        for submissionID: String,
        message: String
    ) -> Bool {
        var current = state(for: submissionID)
        guard case .loading(let previous) = current.installation else { return false }
        current.installation = .failed(message: message, previous: previous)
        store(current, for: submissionID)
        return true
    }

    func installStatusNeedsResolution(for submissionID: String) -> Bool {
        state(for: submissionID).installation.needsResolution
    }

    func installStatusIsAbsent(for submissionID: String) -> Bool {
        state(for: submissionID).installation.isAbsent
    }

    mutating func beginReview(
        for submissionID: String,
        draft: SkillDraft? = nil
    ) -> SkillReviewStart? {
        var current = state(for: submissionID)
        guard current.operation == nil,
              current.installation.isAbsent,
              case .candidate(let candidate) = current.phase
        else { return nil }
        let reviewedCandidate = candidate.replacingDraft(draft ?? candidate.draft)
        current.phase = .candidate(reviewedCandidate)
        current.operation = .reviewing
        current.workflowFailure = nil
        store(current, for: submissionID)
        return SkillReviewStart(
            candidate: reviewedCandidate,
            replacesReviewID: current.replacesReviewID
        )
    }

    mutating func finishReview(for submissionID: String, review: SkillReview) -> Bool {
        var current = state(for: submissionID)
        guard current.operation == .reviewing,
              case .candidate(let candidate) = current.phase,
              review.candidateID == candidate.candidateID,
              review.sourceSubmissionID == submissionID,
              review.sourceSubmissionID == candidate.sourceSubmissionID
        else { return false }
        current.phase = .reviewed(candidate: candidate, review: review)
        current.operation = nil
        current.workflowFailure = nil
        current.preservedDraft = nil
        current.replacesReviewID = nil
        store(current, for: submissionID)
        return true
    }

    mutating func beginEvaluation(for submissionID: String) -> SkillReview? {
        let current = state(for: submissionID)
        guard current.installation.isAbsent,
              case .reviewed(_, let review) = current.phase,
              begin(.evaluating, for: submissionID)
        else { return nil }
        return review
    }

    mutating func finishEvaluation(for submissionID: String, report: SkillEvaluationReport) -> Bool {
        var current = state(for: submissionID)
        guard current.operation == .evaluating,
              case .reviewed(let candidate, let review) = current.phase,
              report.reviewID == review.reviewID,
              report.skillSHA256 == review.skillSHA256
        else { return false }
        current.phase = .evaluated(candidate: candidate, review: review, report: report)
        current.operation = nil
        current.workflowFailure = nil
        store(current, for: submissionID)
        return true
    }

    mutating func beginInstallPlanning(for submissionID: String) -> SkillEvaluationReport? {
        var current = state(for: submissionID)
        guard current.operation == nil, current.installation.isAbsent else { return nil }
        let report: SkillEvaluationReport
        switch current.phase {
        case .evaluated(_, _, let value):
            report = value
        case .planned(let candidate, let review, let value, let plan) where !plan.canInstall:
            report = value
            current.phase = .evaluated(candidate: candidate, review: review, report: value)
        default:
            return nil
        }
        guard report.installAllowed else { return nil }
        current.operation = .planningInstall
        current.workflowFailure = nil
        store(current, for: submissionID)
        return report
    }

    mutating func finishInstallPlanning(
        for submissionID: String,
        plan: SkillInstallPlan
    ) -> Bool {
        var current = state(for: submissionID)
        guard current.operation == .planningInstall,
              case .evaluated(let candidate, let review, let report) = current.phase,
              plan.evaluationID == report.evaluationID,
              plan.skillSHA256 == report.skillSHA256
        else { return false }
        current.phase = .planned(
            candidate: candidate,
            review: review,
            report: report,
            plan: plan
        )
        current.operation = nil
        current.workflowFailure = nil
        store(current, for: submissionID)
        return true
    }

    mutating func beginInstall(for submissionID: String) -> SkillInstallPlan? {
        let current = state(for: submissionID)
        guard current.installation.isAbsent,
              case .planned(_, _, _, let plan) = current.phase,
              plan.canInstall,
              begin(.installing, for: submissionID)
        else { return nil }
        return plan
    }

    mutating func finishInstall(for submissionID: String, installed: InstalledSkill) -> Bool {
        var current = state(for: submissionID)
        guard current.operation == .installing,
              case .planned(let candidate, let review, let report, let plan) = current.phase,
              installed.evaluationID == plan.evaluationID,
              installed.skillSHA256 == plan.skillSHA256,
              installed.sourceSubmissionID == submissionID,
              installed.sourceSubmissionID == candidate.sourceSubmissionID,
              installed.sourceSubmissionID == review.sourceSubmissionID
        else { return false }
        current.phase = .evaluated(candidate: candidate, review: review, report: report)
        current.installation = .installed(installed)
        current.operation = nil
        current.workflowFailure = nil
        store(current, for: submissionID)
        return true
    }

    mutating func finishInstallReconciliationFailure(
        for submissionID: String,
        lastKnownInstalled: InstalledSkill?,
        statusWasResolved: Bool,
        message: String
    ) {
        var current = state(for: submissionID)
        guard current.operation == .installing,
              case .planned(let candidate, let review, let report, _) = current.phase
        else { return }
        current.phase = .evaluated(candidate: candidate, review: review, report: report)
        current.installation = if let lastKnownInstalled {
            .failed(message: message, previous: lastKnownInstalled)
        } else if statusWasResolved {
            .absent
        } else {
            .failed(message: message, previous: nil)
        }
        current.operation = nil
        current.workflowFailure = message
        store(current, for: submissionID)
    }

    mutating func beginRollback(for submissionID: String) -> InstalledSkill? {
        guard let installed = state(for: submissionID).installedSkill,
              begin(.rollingBack, for: submissionID)
        else { return nil }
        return installed
    }

    mutating func finishRollback(
        for submissionID: String,
        removed: Bool,
        incompleteMessage: String
    ) {
        var current = state(for: submissionID)
        guard current.operation == .rollingBack,
              let installed = current.installedSkill
        else { return }
        if removed {
            current.installation = .absent
            current.workflowFailure = nil
        } else {
            current.installation = .failed(message: incompleteMessage, previous: installed)
            current.workflowFailure = incompleteMessage
        }
        current.operation = nil
        store(current, for: submissionID)
    }

    mutating func finishFailure(
        for submissionID: String,
        operation: SkillLearningOperation,
        message: String
    ) {
        var current = state(for: submissionID)
        guard current.operation == operation else { return }
        if operation == .installing,
           case .planned(let candidate, let review, let report, _) = current.phase
        {
            current.phase = .evaluated(candidate: candidate, review: review, report: report)
        }
        current.operation = nil
        current.workflowFailure = message
        store(current, for: submissionID)
    }

    mutating func edit(for submissionID: String) -> Bool {
        var current = state(for: submissionID)
        guard current.operation == nil, current.installation.isAbsent else { return false }
        let candidate: SkillCandidate
        let reviewID: String
        switch current.phase {
        case .reviewed(let value, let review),
             .evaluated(let value, let review, _),
             .planned(let value, let review, _, _):
            candidate = value.replacingDraft(review.draft)
            reviewID = review.reviewID
        default:
            return false
        }
        current.phase = .candidate(candidate)
        current.operation = nil
        current.workflowFailure = nil
        current.preservedDraft = nil
        current.replacesReviewID = reviewID
        store(current, for: submissionID)
        return true
    }

    mutating func finishExpiredWorkflow(
        for submissionID: String,
        operation: SkillLearningOperation,
        message: String
    ) {
        var current = state(for: submissionID)
        guard current.operation == operation else { return }
        current.preservedDraft = draft(from: current.phase)
        current.phase = .idle
        current.operation = nil
        current.workflowFailure = message
        current.replacesReviewID = nil
        store(current, for: submissionID)
    }

    private mutating func begin(
        _ operation: SkillLearningOperation,
        for submissionID: String
    ) -> Bool {
        var current = state(for: submissionID)
        guard current.operation == nil else { return false }
        current.operation = operation
        current.workflowFailure = nil
        store(current, for: submissionID)
        return true
    }

    private mutating func store(
        _ value: SkillLearningSessionState,
        for submissionID: String
    ) {
        sessions[submissionID] = value
        recency.removeAll { $0 == submissionID }
        recency.append(submissionID)
        while sessions.count > Self.maximumRetainedSessions,
              let index = recency.firstIndex(where: { candidate in
                  candidate != submissionID && sessions[candidate]?.isWorking == false
              })
        {
            let evicted = recency.remove(at: index)
            sessions.removeValue(forKey: evicted)
        }
    }

    private func draft(from phase: SkillLearningPhase) -> SkillDraft? {
        switch phase {
        case .idle:
            return nil
        case .candidate(let candidate):
            return candidate.draft
        case .reviewed(_, let review),
             .evaluated(_, let review, _),
             .planned(_, let review, _, _):
            return review.draft
        }
    }
}
