import Foundation

public enum ComparisonTaskOutcome: String, Codable, Sendable, CaseIterable {
    case pending, accepted, partial, rejected, unknown
}

public enum ComparisonReasoningEffort: String, Codable, Sendable, CaseIterable {
    case unknown, none, minimal, low, medium, high, xhigh
}

public enum ComparisonContextString: Codable, Sendable, Equatable {
    case unknown
    case known(String)
    private enum Keys: String, CodingKey { case state, value }
    private enum State: String, Codable { case unknown, known }
    public init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: Keys.self)
        switch try values.decode(State.self, forKey: .state) {
        case .unknown: self = .unknown
        case .known: self = .known(try values.decode(String.self, forKey: .value))
        }
    }
    public func encode(to encoder: Encoder) throws {
        var values = encoder.container(keyedBy: Keys.self)
        switch self {
        case .unknown: try values.encode(State.unknown, forKey: .state)
        case .known(let value):
            try values.encode(State.known, forKey: .state); try values.encode(value, forKey: .value)
        }
    }
}

public enum ComparisonContextDigest: Codable, Sendable, Equatable {
    case unknown
    case known(String)
    private enum Keys: String, CodingKey { case state, digest }
    private enum State: String, Codable { case unknown, known }
    public init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: Keys.self)
        switch try values.decode(State.self, forKey: .state) {
        case .unknown: self = .unknown
        case .known: self = .known(try values.decode(String.self, forKey: .digest))
        }
    }
    public func encode(to encoder: Encoder) throws {
        var values = encoder.container(keyedBy: Keys.self)
        switch self {
        case .unknown: try values.encode(State.unknown, forKey: .state)
        case .known(let digest):
            try values.encode(State.known, forKey: .state); try values.encode(digest, forKey: .digest)
        }
    }
}

public enum ComparisonCheckoutProvenance: Codable, Sendable, Equatable {
    case unavailable
    private enum Keys: String, CodingKey { case state }
    private enum State: String, Codable { case unavailable }
    public init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: Keys.self)
        guard try values.decode(State.self, forKey: .state) == .unavailable else {
            throw InsightsError.invalidResponse
        }
        self = .unavailable
    }
    public func encode(to encoder: Encoder) throws {
        var values = encoder.container(keyedBy: Keys.self)
        try values.encode(State.unavailable, forKey: .state)
    }
}

public struct ComparisonConfiguration: Codable, Sendable, Equatable {
    public var harness_id, harness_version, tool_policy_id, tool_policy_version: ComparisonContextString
    public var reasoning_effort: ComparisonReasoningEffort
    public var prompt_template_digest: ComparisonContextDigest
    public init(harnessID: ComparisonContextString = .unknown,
                harnessVersion: ComparisonContextString = .unknown,
                reasoningEffort: ComparisonReasoningEffort = .unknown,
                toolPolicyID: ComparisonContextString = .unknown,
                toolPolicyVersion: ComparisonContextString = .unknown,
                promptTemplateDigest: ComparisonContextDigest = .unknown) {
        harness_id = harnessID; harness_version = harnessVersion; reasoning_effort = reasoningEffort
        tool_policy_id = toolPolicyID; tool_policy_version = toolPolicyVersion
        prompt_template_digest = promptTemplateDigest
    }
}

public struct ComparisonTaskContextInput: Codable, Sendable, Equatable {
    public var project_id, category, task_date: String
    public var checkout_provenance: ComparisonCheckoutProvenance
    public var language: ComparisonContextString
    public var configuration: ComparisonConfiguration
    public init(projectID: String, taskDate: String, language: ComparisonContextString = .unknown,
                configuration: ComparisonConfiguration = .init()) {
        project_id = projectID; category = "refactor"; task_date = taskDate
        checkout_provenance = .unavailable; self.language = language; self.configuration = configuration
    }
}

public struct ComparisonTaskContext: Codable, Sendable, Equatable {
    public let project_id, category, task_date: String
    public let checkout_provenance: ComparisonCheckoutProvenance
    public let language: ComparisonContextString
    public let configuration: ComparisonConfiguration
    public let configuration_fingerprint: String
    public var isComplete: Bool {
        guard case .known = language, case .known = configuration.harness_id,
              case .known = configuration.harness_version,
              configuration.reasoning_effort != .unknown,
              case .known = configuration.tool_policy_id,
              case .known = configuration.tool_policy_version,
              case .known = configuration.prompt_template_digest else { return false }
        return true
    }
}

public struct FrozenEpisodeBinding: Decodable, Sendable, Equatable, Identifiable {
    public let episode_id: String
    public let revision, membership_revision: UInt64
    public let members_digest: String
    public let members: [EpisodeMember]
    public var id: String { episode_id }
}

public struct MaterialBoundOutcome: Decodable, Sendable, Equatable {
    public let value: ComparisonTaskOutcome
    public let material_revision: UInt64
    public let material_digest, recorded_at: String
}

public struct IndependenceConfirmation: Decodable, Sendable, Equatable {
    public let material_revision: UInt64
    public let material_digest, confirmed_at: String
}

public struct LocalComparisonTask: Decodable, Sendable, Equatable, Identifiable {
    public let schema_version: UInt32
    public let id: String
    public let revision, material_revision: UInt64
    public let material_digest, created_at, updated_at: String
    public let episodes: [FrozenEpisodeBinding]
    public let context: ComparisonTaskContext?
    public let outcome: MaterialBoundOutcome?
    public let independence_confirmation: IndependenceConfirmation?
    public func validateSupportedSchema() throws {
        guard schema_version == 1, revision > 0, material_revision > 0,
              material_revision <= revision, Self.uuid(id), Self.digest(material_digest), !episodes.isEmpty,
              Set(episodes.map(\.id)).count == episodes.count,
              episodes.allSatisfy({ Self.uuid($0.episode_id) && $0.revision > 0
                  && $0.membership_revision > 0 && $0.membership_revision <= $0.revision
                  && Self.digest($0.members_digest) && !$0.members.isEmpty }) else {
            throw InsightsError.invalidResponse
        }
        if let outcome { guard outcome.material_revision <= material_revision,
            Self.digest(outcome.material_digest) else { throw InsightsError.invalidResponse } }
        if let confirmation = independence_confirmation { guard confirmation.material_revision <= material_revision,
            Self.digest(confirmation.material_digest) else { throw InsightsError.invalidResponse } }
        if let context { guard Self.uuid(context.project_id), context.category == "refactor",
            Self.digest(context.configuration_fingerprint) else { throw InsightsError.invalidResponse } }
    }
    static func digest(_ value: String) -> Bool {
        value.utf8.count == 64 && value.utf8.allSatisfy {
            (UInt8(ascii: "0")...UInt8(ascii: "9")).contains($0)
                || (UInt8(ascii: "a")...UInt8(ascii: "f")).contains($0)
        }
    }
    static func uuid(_ value: String) -> Bool { UUID(uuidString: value)?.uuidString.lowercased() == value }
}

public enum ComparisonTaskStaleReason: String, Codable, Sendable, CaseIterable {
    case episodeMissing = "episode_missing"
    case episodeRevisionChanged = "episode_revision_changed"
    case episodeMembershipChanged = "episode_membership_changed"
    case snapshotMissingOrReplaced = "snapshot_missing_or_replaced"
    case outcomeMaterialChanged = "outcome_material_changed"
    case independenceMaterialChanged = "independence_material_changed"
    case contextIncomplete = "context_incomplete"
    case attributionPendingQualification = "attribution_pending_qualification"
    case overlappingTaskEvidence = "overlapping_task_evidence"
}

public struct ComparisonTaskDetail: Decodable, Sendable, Equatable, Identifiable {
    public let task: LocalComparisonTask
    public let stale_reasons: [ComparisonTaskStaleReason]
    public let overlapping_task_ids: [String]
    public let resolved_at: String
    public var id: String { task.id }
    public func validateSupportedSchema(expectedID: String? = nil) throws {
        try task.validateSupportedSchema()
        guard expectedID == nil || task.id == expectedID,
              Set(stale_reasons).count == stale_reasons.count,
              Set(overlapping_task_ids).count == overlapping_task_ids.count,
              overlapping_task_ids.allSatisfy(LocalComparisonTask.uuid) else {
            throw InsightsError.invalidResponse
        }
    }
}
