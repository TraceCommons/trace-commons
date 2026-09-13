// INTEGRATION: shared by HistoryView, SessionDetailView, DaemonClient, and
// AppModel for account-owned session detail and reviewed public workflows.

import Foundation

enum SessionEvidenceKind: String, Decodable, Equatable {
    case userMessage = "user_message"
    case assistantMessage = "assistant_message"
    case reasoning
    case toolCall = "tool_call"
    case toolResult = "tool_result"
    case routingDecision = "routing_decision"
    case feedback
    case httpExchange = "http_exchange"
    case unknown

    init(from decoder: Decoder) throws {
        let value = try decoder.singleValueContainer().decode(String.self)
        self = SessionEvidenceKind(rawValue: value) ?? .unknown
    }
}

struct SessionEvidenceCandidate: Decodable, Identifiable, Equatable {
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

enum PublicRunReusePermission: String, Codable, Identifiable {
    case ccBy40 = "cc_by_4_0"
    case cc0 = "cc0_1_0"

    var id: String { rawValue }
}

struct PublicRunReuseChoice: Decodable, Equatable, Identifiable {
    let permission: PublicRunReusePermission
    let label: String
    let explanation: String

    var id: PublicRunReusePermission { permission }
}

struct PublicRunValueLabel: Decodable, Equatable, Identifiable {
    let value: String
    let label: String

    var id: String { value }
}

/// Every word on the session-publication surface, decoded all or nothing
/// from the Rust contributor crate.
struct PublicRunCopy: Decodable, Equatable {
    let allContributions: String
    let viewSession: String
    let sessionDetail: String
    let readingRecord: String
    let retryRead: String
    let contentUnavailable: String
    let unavailableValue: String
    let creatorReport: String
    let task: String
    let noTask: String
    let outcome: String
    let outcomeUnavailable: String
    let decisiveCorrection: String
    let noCorrection: String
    let supportingEvidence: String
    let observedInVersion: String
    let noEvidence: String
    let contributedVersion: String
    let contributionDetails: String
    let processingStatus: String
    let permittedUses: String
    let noPermittedUses: String
    let permittedUsesUnavailable: String
    let unrecognizedValue: String
    let nextAction: String
    let withdraw: String
    let keepContribution: String
    let envelopeVersion: String
    let consentPolicyVersion: String
    let redactionVersion: String
    let publicationAfterAcceptance: String
    let publicWorkflow: String
    let published: String
    let openPage: String
    let editPage: String
    let unpublishing: String
    let unpublish: String
    let publicationDisclosure: String
    let pageTitle: String
    let publicOutcome: String
    let reusableInstructions: String
    let selectEvidence: String
    let publishCorrection: String
    let reusePermission: String
    let choosePermission: String
    let sourcePublicRun: String
    let sourcePlaceholder: String
    let sourceHelp: String
    let cancelEdit: String
    let reviewPage: String
    let exactPublicPreview: String
    let observedEvidence: String
    let useWorkflow: String
    let editDraft: String
    let publishing: String
    let publishPage: String
    let updatePage: String
    let validationRequired: String
    let validationTooLong: String
    let validationCorrectionTooLong: String
    let validationEvidenceRequired: String
    let validationPermissionRequired: String
    let validationSourceInvalid: String
    let sessionAccountRequired: String
    let sessionNotFound: String
    let sessionUnavailable: String
    let publicationAccountRequired: String
    let publicationConflict: String
    let publicationTraceNotFound: String
    let publicationSourceNotFound: String
    let publicationInvalid: String
    let publicationUnavailable: String
    let credentialStorageWarning: String
    let taskOutcomeChoices: [PublicRunValueLabel]
    let feedbackChoices: [PublicRunValueLabel]
    let evidenceKindChoices: [PublicRunValueLabel]
    let contributionStatusChoices: [PublicRunValueLabel]
    let permittedUseChoices: [PublicRunValueLabel]
    let reusePermissions: [PublicRunReuseChoice]

    static func decode(fromJSON json: String) -> PublicRunCopy? {
        guard let data = json.data(using: .utf8) else { return nil }
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return try? decoder.decode(PublicRunCopy.self, from: data)
    }

    func reuseChoice(for permission: PublicRunReusePermission) -> PublicRunReuseChoice? {
        reusePermissions.first { $0.permission == permission }
    }

    func contributionStatusLabel(for value: String) -> String {
        contributionStatusChoices.first { $0.value == value }?.label ?? unrecognizedValue
    }

    func permittedUseLabel(for value: String) -> String {
        permittedUseChoices.first { $0.value == value }?.label ?? unrecognizedValue
    }

    func taskOutcomeLabel(for value: String?) -> String? {
        guard let value else { return nil }
        return taskOutcomeChoices.first { choice in choice.value == value }?.label
    }

    func feedbackLabel(for value: String?) -> String? {
        guard let value else { return nil }
        return feedbackChoices.first { choice in choice.value == value }?.label
    }

    func evidenceKindLabel(for kind: SessionEvidenceKind) -> String {
        evidenceKindChoices.first { $0.value == kind.rawValue }?.label ?? unrecognizedValue
    }
}

enum ContributionStatusPresentation {
    private static let terminalValues: Set<String> = [
        "withdrawn", "revoked", "purged", "expired",
    ]

    static func isTerminal(_ value: String?) -> Bool {
        value.map(terminalValues.contains) ?? false
    }
}

struct PublicRunEvidenceDraft: Codable, Equatable {
    let eventID: String
    let excerpt: String

    enum CodingKeys: String, CodingKey {
        case eventID = "event_id"
        case excerpt
    }
}

struct PublicRunDraftInput: Codable, Equatable {
    let title: String
    let outcomeSummary: String
    let correctionExcerpt: String?
    let workflow: String
    let reusePermission: PublicRunReusePermission
    let evidence: [PublicRunEvidenceDraft]
    let sourceSlug: String?

    enum CodingKeys: String, CodingKey {
        case title
        case outcomeSummary = "outcome_summary"
        case correctionExcerpt = "correction_excerpt"
        case workflow
        case reusePermission = "reuse_permission"
        case evidence
        case sourceSlug = "source_slug"
    }
}

struct PublicRunEditorInput: Encodable {
    let title: String
    let outcomeSummary: String
    let correctionExcerpt: String?
    let workflow: String
    let reusePermission: PublicRunReusePermission?
    let evidence: [PublicRunEvidenceDraft]
    let source: String

    enum CodingKeys: String, CodingKey {
        case title
        case outcomeSummary = "outcome_summary"
        case correctionExcerpt = "correction_excerpt"
        case workflow
        case reusePermission = "reuse_permission"
        case evidence
        case source
    }
}

struct PublicRunEditorValidation: Decodable {
    let draft: PublicRunDraftInput?
    let error: String?
}

struct PublicRunLink: Decodable, Equatable, Identifiable {
    let slug: String
    let title: String
    var id: String { slug }
}

struct PublicRunEvidence: Decodable, Equatable, Identifiable {
    let excerpt: String
    var id: String { excerpt }
}

struct PublicRunPage: Decodable, Equatable {
    let slug: String
    let title: String
    let outcomeSummary: String
    let correctionExcerpt: String?
    let workflow: String
    let reusePermission: PublicRunReusePermission
    let evidence: [PublicRunEvidence]
    let taskSuccess: String
    let contributedVersion: String
    let version: Int
    let publishedAt: Date
    let publicURL: URL?
    let source: PublicRunLink?
    let sourceUnavailable: Bool?
    let variations: [PublicRunLink]
    let credentialWarning: String?

    enum CodingKeys: String, CodingKey {
        case slug
        case title
        case outcomeSummary = "outcome_summary"
        case correctionExcerpt = "correction_excerpt"
        case workflow
        case reusePermission = "reuse_permission"
        case evidence
        case taskSuccess = "task_success"
        case contributedVersion = "contributed_version"
        case version
        case publishedAt = "published_at"
        case publicURL = "public_url"
        case source
        case sourceUnavailable = "source_unavailable"
        case variations
        case credentialWarning = "credential_warning"
    }
}

struct PublicRunUnpublishResult: Decodable, Equatable {
    let unpublished: Bool
    let expectedPublicationVersion: Int
    let credentialWarning: String?

    enum CodingKeys: String, CodingKey {
        case unpublished
        case expectedPublicationVersion = "expected_publication_version"
        case credentialWarning = "credential_warning"
    }
}

struct SessionDetail: Decodable, Equatable {
    /// Opaque local binding supplied by the daemon. The app uses it only to
    /// evict account-owned content when the signed-in account changes.
    let ownerScopeSHA256: String?
    let contentUnavailable: Bool?
    let task: String?
    let taskSuccess: String?
    let userFeedback: String?
    let humanCorrection: String?
    let evidence: [SessionEvidenceCandidate]
    let contributionStatus: String?
    let permittedUses: [String]?
    let contributedVersion: String
    let consentPolicyVersion: String
    let redactionPipelineVersion: String
    var publication: PublicRunPage?
    var publicationVersion: Int
    let retainedSourceSlug: String?

    var accepted: Bool { contributionStatus == "accepted" }
    enum CodingKeys: String, CodingKey {
        case ownerScopeSHA256 = "owner_scope_sha256"
        case contentUnavailable = "content_unavailable"
        case task
        case taskSuccess = "task_success"
        case userFeedback = "user_feedback"
        case humanCorrection = "human_correction"
        case evidence
        case contributionStatus = "contribution_status"
        case permittedUses = "permitted_uses"
        case contributedVersion = "contributed_version"
        case consentPolicyVersion = "consent_policy_version"
        case redactionPipelineVersion = "redaction_pipeline_version"
        case publication
        case publicationVersion = "publication_version"
        case retainedSourceSlug = "retained_source_slug"
    }
}
