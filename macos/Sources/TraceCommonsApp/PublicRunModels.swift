// INTEGRATION: shared by HistoryView, SessionDetailView, DaemonClient, and
// AppModel for account-owned session detail and reviewed public workflows.

import Foundation

struct SessionEvidenceCandidate: Decodable, Identifiable, Equatable {
    let eventID: String
    let kind: String
    let label: String
    let excerpt: String

    var id: String { eventID }

    enum CodingKeys: String, CodingKey {
        case eventID = "event_id"
        case kind
        case label
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

/// Every word on the session-publication surface, decoded all or nothing
/// from the Rust contributor crate.
struct PublicRunCopy: Decodable, Equatable {
    let allContributions: String
    let viewSession: String
    let sessionDetail: String
    let readingRecord: String
    let retryRead: String
    let creatorReport: String
    let decisiveCorrection: String
    let noCorrection: String
    let supportingEvidence: String
    let observedInVersion: String
    let noEvidence: String
    let contributedVersion: String
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
    let taskSuccess: String
    let taskOutcome: String
    let userFeedback: String
    let feedbackLine: String?
    let humanCorrection: String?
    let evidence: [SessionEvidenceCandidate]
    let contributedVersion: String
    let consentPolicyVersion: String
    let redactionPipelineVersion: String
    var publication: PublicRunPage?
    var publicationVersion: Int
    let retainedSourceSlug: String?

    enum CodingKeys: String, CodingKey {
        case taskSuccess = "task_success"
        case taskOutcome = "task_outcome"
        case userFeedback = "user_feedback"
        case feedbackLine = "feedback_line"
        case humanCorrection = "human_correction"
        case evidence
        case contributedVersion = "contributed_version"
        case consentPolicyVersion = "consent_policy_version"
        case redactionPipelineVersion = "redaction_pipeline_version"
        case publication
        case publicationVersion = "publication_version"
        case retainedSourceSlug = "retained_source_slug"
    }
}
