// INTEGRATION: typed wire models for correction-derived Agent Skills,
// controlled NEAR AI evaluation, and guarded Codex installation.

import Foundation

struct SkillLearningCopy: Decodable, Equatable {
    let heading: String
    let promise: String
    let supportedFamily: String
    let learnAction: String
    let learning: String
    let candidateHeading: String
    let generatedSource: String
    let name: String
    let applicability: String
    let procedure: String
    let sourceEvidence: String
    let manualInstruction: String
    let testContract: String
    let contractSummaryFormat: String
    let reviewAction: String
    let reviewing: String
    let exactPackage: String
    let digest: String
    let evaluationDisclosure: String
    let reviewBudgetFormat: String
    let approveAndTest: String
    let editSkill: String
    let testing: String
    let results: String
    let passedGate: String
    let failedGate: String
    let repositoryPlans: String
    let skillApplicability: String
    let regressions: String
    let noRegressions: String
    let inspectRuns: String
    let modelAndBudget: String
    let modelBudgetFormat: String
    let openFixtureSource: String
    let modelOutput: String
    let edits: String
    let commands: String
    let checks: String
    let baseline: String
    let simpleInstruction: String
    let candidateSkill: String
    let passed: String
    let failed: String
    let preparing: String
    let reviewInstall: String
    let retryInstall: String
    let installPreview: String
    let installDisclosure: String
    let persistentWrites: String
    let targetPath: String
    let skillFile: String
    let ownershipMarker: String
    let markerDigest: String
    let exactMarker: String
    let codingTool: String
    let installAction: String
    let installing: String
    let installed: String
    let rollback: String
    let rollingBack: String
    let rollbackDisclosure: String
    let rollbackIncomplete: String
    let unavailable: String

    static func decode(fromJSON json: String) -> SkillLearningCopy? {
        guard let data = json.data(using: .utf8) else { return nil }
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return try? decoder.decode(SkillLearningCopy.self, from: data)
    }
}

struct SkillSourceEvidence: Decodable, Equatable, Identifiable {
    let eventID: String
    let kind: SessionEvidenceKind
    let excerpt: String

    var id: String { eventID }
    enum CodingKeys: String, CodingKey {
        case eventID = "event_id"
        case kind
        case excerpt
    }
}

struct SkillDraft: Codable, Equatable {
    var name: String
    var description: String
    var procedure: String
}

struct SkillDraftValidation: Decodable, Equatable {
    let valid: Bool
    let error: String?
    let nameChars: Int
    let descriptionChars: Int
    let procedureChars: Int
    let nameMaxChars: Int
    let descriptionMaxChars: Int
    let procedureMaxChars: Int

    enum CodingKeys: String, CodingKey {
        case valid
        case error
        case nameChars = "name_chars"
        case descriptionChars = "description_chars"
        case procedureChars = "procedure_chars"
        case nameMaxChars = "name_max_chars"
        case descriptionMaxChars = "description_max_chars"
        case procedureMaxChars = "procedure_max_chars"
    }
}

struct SkillEvaluationContract: Decodable, Equatable {
    let taskCount: Int
    let planTaskCount: Int
    let fixturePoolCount: Int
    let clusterCount: Int
    let armCount: Int
    let totalRequests: Int
    let outputTokenLimit: Int
    let requestTimeoutSeconds: Int
    let requiredModelOwner: String
    let fixtureScope: String
    let selectionPolicy: String

    enum CodingKeys: String, CodingKey {
        case taskCount = "task_count"
        case planTaskCount = "plan_task_count"
        case fixturePoolCount = "fixture_pool_count"
        case clusterCount = "cluster_count"
        case armCount = "arm_count"
        case totalRequests = "total_requests"
        case outputTokenLimit = "output_token_limit"
        case requestTimeoutSeconds = "request_timeout_seconds"
        case requiredModelOwner = "required_model_owner"
        case fixtureScope = "fixture_scope"
        case selectionPolicy = "selection_policy"
    }
}

struct SkillCandidate: Decodable, Equatable {
    let candidateID: String
    let family: String
    let sourceSubmissionID: String
    let sourceCorrection: String
    let sourceEvidence: [SkillSourceEvidence]
    let draft: SkillDraft
    let replacesReviewID: String?
    let manualControlInstruction: String
    let evaluationContract: SkillEvaluationContract

    enum CodingKeys: String, CodingKey {
        case candidateID = "candidate_id"
        case family
        case sourceSubmissionID = "source_submission_id"
        case sourceCorrection = "source_correction"
        case sourceEvidence = "source_evidence"
        case draft
        case replacesReviewID = "replaces_review_id"
        case manualControlInstruction = "manual_control_instruction"
        case evaluationContract = "evaluation_contract"
    }

    func replacingDraft(_ value: SkillDraft) -> SkillCandidate {
        SkillCandidate(
            candidateID: candidateID,
            family: family,
            sourceSubmissionID: sourceSubmissionID,
            sourceCorrection: sourceCorrection,
            sourceEvidence: sourceEvidence,
            draft: value,
            replacesReviewID: replacesReviewID,
            manualControlInstruction: manualControlInstruction,
            evaluationContract: evaluationContract
        )
    }
}

struct SkillReview: Decodable, Equatable {
    let reviewID: String
    let candidateID: String
    let family: String
    let sourceSubmissionID: String
    let sourceEvidenceIDs: [String]
    let draft: SkillDraft
    let skillMD: String
    let skillSHA256: String

    enum CodingKeys: String, CodingKey {
        case reviewID = "review_id"
        case candidateID = "candidate_id"
        case family
        case sourceSubmissionID = "source_submission_id"
        case sourceEvidenceIDs = "source_evidence_ids"
        case draft
        case skillMD = "skill_md"
        case skillSHA256 = "skill_sha256"
    }
}

enum SkillEvaluationArm: String, Decodable, Equatable, Hashable {
    case baseline
    case manualInstruction = "manual_instruction"
    case candidateSkill = "candidate_skill"
}

struct SkillEvaluationUsage: Decodable, Equatable {
    let promptTokens: Int?
    let completionTokens: Int?
    let reasoningTokens: Int?
    let totalTokens: Int?

    enum CodingKeys: String, CodingKey {
        case promptTokens = "prompt_tokens"
        case completionTokens = "completion_tokens"
        case reasoningTokens = "reasoning_tokens"
        case totalTokens = "total_tokens"
    }
}

struct ProposedTaskPlan: Decodable, Equatable {
    let diagnosis: String
    let editPaths: [String]
    let commands: [String]
    let verification: [String]

    enum CodingKeys: String, CodingKey {
        case diagnosis
        case editPaths = "edit_paths"
        case commands
        case verification
    }
}

struct SkillTrialResult: Decodable, Equatable, Identifiable {
    let taskID: String
    let cluster: String
    let task: String
    let sourceURL: URL
    let arm: SkillEvaluationArm
    let passed: Bool
    let failureReasons: [String]
    let answer: ProposedTaskPlan?
    let rawOutput: String
    let requestID: String?
    let servedModel: String
    let finishReason: String
    let usage: SkillEvaluationUsage

    var id: String { "\(taskID)-\(arm.rawValue)" }

    enum CodingKeys: String, CodingKey {
        case taskID = "task_id"
        case cluster
        case task
        case sourceURL = "source_url"
        case arm
        case passed
        case failureReasons = "failure_reasons"
        case answer
        case rawOutput = "raw_output"
        case requestID = "request_id"
        case servedModel = "served_model"
        case finishReason = "finish_reason"
        case usage
    }
}

struct SkillArmSummary: Decodable, Equatable, Identifiable {
    let arm: SkillEvaluationArm
    let label: String
    let passed: Int
    let total: Int

    var id: SkillEvaluationArm { arm }
}

struct SkillEvaluationExecution: Decodable, Equatable {
    let requestedModel: String
    let servedModel: String
    let modelOwnedBy: String
    let sourceTaskFingerprintSHA256: String
    let selectedPlanTaskIDs: [String]
    let excludedSourceOverlapTaskIDs: [String]
    let reservePlanTaskIDs: [String]
    let ambientContext: [String]

    enum CodingKeys: String, CodingKey {
        case requestedModel = "requested_model"
        case servedModel = "served_model"
        case modelOwnedBy = "model_owned_by"
        case sourceTaskFingerprintSHA256 = "source_task_fingerprint_sha256"
        case selectedPlanTaskIDs = "selected_plan_task_ids"
        case excludedSourceOverlapTaskIDs = "excluded_source_overlap_task_ids"
        case reservePlanTaskIDs = "reserve_plan_task_ids"
        case ambientContext = "ambient_context"
    }
}

struct SkillEvaluationReport: Decodable, Equatable {
    let evaluationID: String
    let reviewID: String
    let skillSHA256: String
    let evaluationContract: SkillEvaluationContract
    let execution: SkillEvaluationExecution
    let summaries: [SkillArmSummary]
    let planSummaries: [SkillArmSummary]
    let applicabilitySummaries: [SkillArmSummary]
    let trials: [SkillTrialResult]
    let regressions: [String]
    let installAllowed: Bool
    let gateReason: String

    var outputTokenLimit: Int { evaluationContract.outputTokenLimit }
    var requestTimeoutSeconds: Int { evaluationContract.requestTimeoutSeconds }
    var armCount: Int { evaluationContract.armCount }
    var taskCount: Int { evaluationContract.taskCount }
    var planTaskCount: Int { evaluationContract.planTaskCount }
    var fixturePoolCount: Int { evaluationContract.fixturePoolCount }
    var clusterCount: Int { evaluationContract.clusterCount }
    var selectionPolicy: String { evaluationContract.selectionPolicy }
    var requestedModel: String { execution.requestedModel }
    var servedModel: String { execution.servedModel }
    var modelOwnedBy: String { execution.modelOwnedBy }
    var sourceTaskFingerprintSHA256: String { execution.sourceTaskFingerprintSHA256 }
    var selectedPlanTaskIDs: [String] { execution.selectedPlanTaskIDs }
    var excludedSourceOverlapTaskIDs: [String] { execution.excludedSourceOverlapTaskIDs }
    var reservePlanTaskIDs: [String] { execution.reservePlanTaskIDs }
    var ambientContext: [String] { execution.ambientContext }

    enum CodingKeys: String, CodingKey {
        case evaluationID = "evaluation_id"
        case reviewID = "review_id"
        case skillSHA256 = "skill_sha256"
        case summaries
        case planSummaries = "plan_summaries"
        case applicabilitySummaries = "applicability_summaries"
        case trials
        case regressions
        case installAllowed = "install_allowed"
        case gateReason = "gate_reason"
    }

    init(from decoder: Decoder) throws {
        evaluationContract = try SkillEvaluationContract(from: decoder)
        execution = try SkillEvaluationExecution(from: decoder)
        let container = try decoder.container(keyedBy: CodingKeys.self)
        evaluationID = try container.decode(String.self, forKey: .evaluationID)
        reviewID = try container.decode(String.self, forKey: .reviewID)
        skillSHA256 = try container.decode(String.self, forKey: .skillSHA256)
        summaries = try container.decode([SkillArmSummary].self, forKey: .summaries)
        planSummaries = try container.decode([SkillArmSummary].self, forKey: .planSummaries)
        applicabilitySummaries = try container.decode(
            [SkillArmSummary].self,
            forKey: .applicabilitySummaries
        )
        trials = try container.decode([SkillTrialResult].self, forKey: .trials)
        regressions = try container.decode([String].self, forKey: .regressions)
        installAllowed = try container.decode(Bool.self, forKey: .installAllowed)
        gateReason = try container.decode(String.self, forKey: .gateReason)
    }
}

struct SkillInstallPlan: Decodable, Equatable {
    let planID: String
    let evaluationID: String
    let tool: String
    let targetLocation: String
    let skillLocation: String
    let markerLocation: String
    let occupied: Bool
    let canInstall: Bool
    let skillMD: String
    let skillSHA256: String
    let markerJSON: String
    let markerFileSHA256: String

    enum CodingKeys: String, CodingKey {
        case planID = "plan_id"
        case evaluationID = "evaluation_id"
        case tool
        case targetLocation = "target_location"
        case skillLocation = "skill_location"
        case markerLocation = "marker_location"
        case occupied
        case canInstall = "can_install"
        case skillMD = "skill_md"
        case skillSHA256 = "skill_sha256"
        case markerJSON = "marker_json"
        case markerFileSHA256 = "marker_file_sha256"
    }
}

struct InstalledSkill: Decodable, Equatable {
    let installID: String
    let evaluationID: String
    let sourceSubmissionID: String
    let tool: String
    let name: String
    let targetLocation: String
    let skillSHA256: String
    let markerSHA256: String
    let installedAt: Date

    enum CodingKeys: String, CodingKey {
        case installID = "install_id"
        case evaluationID = "evaluation_id"
        case sourceSubmissionID = "source_submission_id"
        case tool
        case name
        case targetLocation = "target_location"
        case skillSHA256 = "skill_sha256"
        case markerSHA256 = "marker_sha256"
        case installedAt = "installed_at"
    }
}

struct SkillInstallStatus: Decodable, Equatable {
    let installed: Bool
    let skill: InstalledSkill?
}

struct SkillRollbackResult: Decodable, Equatable {
    let removed: Bool
    let retainedDirectory: Bool

    enum CodingKeys: String, CodingKey {
        case removed
        case retainedDirectory = "retained_directory"
    }
}
