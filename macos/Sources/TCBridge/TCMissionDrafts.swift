import CTraceCommons
import Foundation

public enum TCMissionDrafts {
    public static func call(_ request: MissionDraftRequest) throws -> MissionDraftResponse {
        let bytes = try JSONEncoder().encode(request)
        guard bytes.count <= 65_536 else { throw MissionDraftBridgeError.requestTooLarge }
        var error: UnsafeMutablePointer<CChar>?
        let result = bytes.withUnsafeBytes { buffer in
            tc_mission_drafts_call(
                buffer.bindMemory(to: UInt8.self).baseAddress, bytes.count, &error)
        }
        defer {
            if let result { tc_string_free(result) }
            if let error { tc_string_free(error) }
        }
        if let error, let code = String(validatingCString: error) {
            throw MissionDraftBridgeError.service(code)
        }
        guard let result, let text = String(validatingCString: result) else {
            throw MissionDraftBridgeError.operationFailed
        }
        do {
            let response = try JSONDecoder().decode(MissionDraftResponse.self, from: Data(text.utf8))
            try response.validate(for: request.operation)
            return response
        } catch let error as MissionDraftBridgeError {
            throw error
        } catch {
            throw MissionDraftBridgeError.invalidResponse
        }
    }
}

public enum MissionDraftBridgeError: Error, Equatable {
    case requestTooLarge, operationFailed, invalidResponse
    case service(String)
}

public struct MissionDraftRequest: Encodable, Sendable {
    public let store_dir: String?
    public let operation: Operation

    public init(storeDirectory: String? = nil, operation: Operation) {
        store_dir = storeDirectory
        self.operation = operation
    }

    public struct Operation: Encodable, Sendable {
        public let type: String
        public let file: String?
        public let id: String?

        public init(_ type: String, file: String? = nil, id: String? = nil) {
            self.type = type
            self.file = file
            self.id = id
        }
    }
}

public enum MissionDraftResponse: Decodable, Sendable {
    case imported(MissionDraftImport)
    case list([MissionDraftSummary])
    case show(StoredMissionDraft)
    case deleted(MissionDraftDelete)
    case copy([String: String])

    private enum Keys: String, CodingKey { case type, draft, drafts, copy }

    private struct AnyKey: CodingKey {
        let stringValue: String
        let intValue: Int? = nil
        init?(stringValue: String) { self.stringValue = stringValue }
        init?(intValue: Int) { return nil }
    }

    public init(from decoder: Decoder) throws {
        let raw = try decoder.container(keyedBy: AnyKey.self)
        let values = try decoder.container(keyedBy: Keys.self)
        let type = try values.decode(String.self, forKey: .type)
        let allowed: Set<String>
        switch type {
        case "import": allowed = ["type", "draft"]
        case "list": allowed = ["type", "drafts"]
        case "show": allowed = ["type", "draft"]
        case "delete": allowed = ["type", "draft"]
        case "copy": allowed = ["type", "copy"]
        default: throw MissionDraftBridgeError.invalidResponse
        }
        guard Set(raw.allKeys.map(\.stringValue)) == allowed else {
            throw MissionDraftBridgeError.invalidResponse
        }
        switch type {
        case "import": self = .imported(try values.decode(MissionDraftImport.self, forKey: .draft))
        case "list": self = .list(try values.decode([MissionDraftSummary].self, forKey: .drafts))
        case "show": self = .show(try values.decode(StoredMissionDraft.self, forKey: .draft))
        case "delete": self = .deleted(try values.decode(MissionDraftDelete.self, forKey: .draft))
        case "copy": self = .copy(try values.decode([String: String].self, forKey: .copy))
        default: throw MissionDraftBridgeError.invalidResponse
        }
    }

    public func validate(for operation: MissionDraftRequest.Operation) throws {
        switch (operation.type, self) {
        case ("import", .imported(let draft)) where operation.file != nil && operation.id == nil:
            try draft.validate()
        case ("list", .list(let drafts)) where operation.file == nil && operation.id == nil:
            guard drafts.map(\.id) == drafts.map(\.id).sorted(),
                  Set(drafts.map(\.id)).count == drafts.count
            else { throw MissionDraftBridgeError.invalidResponse }
            try drafts.forEach { try $0.validate() }
        case ("show", .show(let draft)) where operation.file == nil && draft.id == operation.id:
            try draft.validate()
        case ("delete", .deleted(let draft))
            where operation.file == nil && draft.id == operation.id && draft.deleted:
            try requireDigest(draft.id)
        case ("copy", .copy(let copy)) where operation.file == nil && operation.id == nil:
            guard Set(copy.keys) == missionDraftCopyKeys,
                  copy.values.allSatisfy({ !$0.isEmpty && !$0.contains("\0") })
            else { throw MissionDraftBridgeError.invalidResponse }
        default: throw MissionDraftBridgeError.invalidResponse
        }
    }
}

private let missionDraftCopyKeys: Set<String> = [
    "title", "intro", "empty", "choose_file", "file_selected", "import", "refresh",
    "refreshed", "show", "delete", "delete_confirm_title", "delete_confirm", "added",
    "duplicate", "deleted", "proposal_sha256", "source_count", "proposal_title",
    "source_claim", "task", "starting_artifact", "starting_artifact_digest", "source_urls",
    "success_criteria", "required_evidence", "allowed_models", "allowed_tools",
    "proposed_budget", "duration_seconds", "input_tokens", "output_tokens",
    "author_unverified", "evaluator_unverified", "rubric_version", "needs_curator_review",
    "review_notice", "authority_notice", "display_notice", "working", "cancel", "close", "error",
]

public struct MissionDraftSummary: Decodable, Sendable, Identifiable, Equatable {
    public let id: String
    public let source_count: Int
    public let status: String

    fileprivate func validate() throws {
        try requireDigest(id)
        guard (1...8).contains(source_count), status == "needs_curator_review" else {
            throw MissionDraftBridgeError.invalidResponse
        }
    }
}

public struct MissionDraftImport: Decodable, Sendable, Equatable {
    public let id: String
    public let review: MissionDraftReview
    public let inserted: Bool

    fileprivate func validate() throws {
        try requireDigest(id)
        try review.validate(expectedID: id)
    }
}

public struct MissionDraftDelete: Decodable, Sendable, Equatable {
    public let id: String
    public let deleted: Bool
}

public struct StoredMissionDraft: Decodable, Sendable, Equatable {
    public let id: String
    public let proposal: MissionProposal
    public let review: MissionDraftReview

    fileprivate func validate() throws {
        try requireDigest(id)
        try review.validate(expectedID: id)
        try proposal.validate()
    }
}

public struct MissionDraftReview: Decodable, Sendable, Equatable {
    public let schema_version: UInt32
    public let proposal_sha256: String
    public let status: String
    public let publication_authorized: Bool
    public let external_sources_verified: Bool
    public let required_reviews: [String]

    fileprivate func validate(expectedID: String) throws {
        guard schema_version == 1, proposal_sha256 == expectedID,
              status == "needs_curator_review", !publication_authorized,
              !external_sources_verified,
              required_reviews == ["source_claim_and_artifact", "reproducibility_and_rights",
                                   "evaluator_and_conflicts", "execution_and_budget"]
        else { throw MissionDraftBridgeError.invalidResponse }
    }
}

public struct MissionProposal: Decodable, Sendable, Equatable {
    public let schema_version: UInt32
    public let author_id, title: String
    public let source_urls: [String]
    public let claim_to_test, task: String
    public let starting_artifact: MissionStartingArtifact
    public let evaluator_id, rubric_version: String
    public let success_criteria, required_evidence, allowed_models, allowed_tools: [String]
    public let budget: MissionBudget

    fileprivate func validate() throws {
        guard schema_version == 1, !author_id.isEmpty, !title.isEmpty,
              !evaluator_id.isEmpty, !rubric_version.isEmpty,
              (1...8).contains(source_urls.count), !claim_to_test.isEmpty, !task.isEmpty,
              !success_criteria.isEmpty, !required_evidence.isEmpty,
              !allowed_models.isEmpty, !allowed_tools.isEmpty,
              source_urls.allSatisfy({ !$0.contains("\0") }),
              !starting_artifact.url.contains("\0") else {
            throw MissionDraftBridgeError.invalidResponse
        }
        try requireDigest(starting_artifact.sha256)
        guard budget.max_duration_seconds > 0, budget.max_input_tokens > 0,
              budget.max_output_tokens > 0 else { throw MissionDraftBridgeError.invalidResponse }
    }
}

public struct MissionStartingArtifact: Decodable, Sendable, Equatable {
    public let url, sha256: String
}

public struct MissionBudget: Decodable, Sendable, Equatable {
    public let max_duration_seconds: UInt32
    public let max_input_tokens, max_output_tokens: UInt64
}

private func requireDigest(_ value: String) throws {
    let bytes = value.utf8
    guard bytes.count == 64, bytes.allSatisfy({
        (UInt8(ascii: "0")...UInt8(ascii: "9")).contains($0)
            || (UInt8(ascii: "a")...UInt8(ascii: "f")).contains($0)
    }) else { throw MissionDraftBridgeError.invalidResponse }
}
