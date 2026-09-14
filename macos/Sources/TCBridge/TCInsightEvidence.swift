import Foundation

public struct ClaudeTaskAttributionEvidence: Decodable, Sendable {
    public let schema_version, extractor_version: UInt32
    public let profile_id, observed_writer_version, qualification_scope, source_digest: String
    public let record_count, recognized_records: UInt64
    public let root_session_identity_sha256, agent_branch_identity_sha256: String?
    public let declared_model, model_selector, context_window_selector: String?
    public let state: State
    public struct State: Decodable, Sendable {
        public let status: String
    }
}

public struct InsightModelObservations: Decodable, Sendable {
    public let schema_version: UInt32
    public let scope: Scope
    public let source_format, source_digest: String
    public let coordinates: Coordinates
    public let record_count, candidate_records, valid_declarations: UInt64
    public let missing_declarations, invalid_declarations, omitted_declarations: UInt64
    public let model_labels_omitted, mixed_declared_models: Bool
    public let declared_models: [String]
    public let declarations: [Declaration]
    /// Rust's verdict, recomputed there on every response. This shell does not
    /// re-derive the contract; an unknown value fails decoding, which is the
    /// fail-closed answer for a contract newer than this build.
    public let contract: Contract
    public enum Contract: String, Decodable, Sendable {
        case legacyDeclaredMetadataV1 = "legacy_declared_metadata_v1"
        case codexTurnContextV2 = "codex_turn_context_v2"
        case claudeAssistantMessageV3 = "claude_assistant_message_v3"
        case unsupported
    }
    public enum Scope: String, Decodable, Sendable {
        case declaredMetadataOnly = "declared_metadata_only"
    }
    public enum Coordinates: String, Decodable, Sendable {
        case jsonlPhysicalLinesOneBased = "jsonl_physical_lines_one_based"
        case trajectoryArrayIndexesZeroBased = "trajectory_array_indexes_zero_based"
    }
    public struct Declaration: Decodable, Sendable, Identifiable {
        public let model: String
        public let record_index: UInt64
        public let kind: Kind
        public var id: UInt64 { record_index }
    }
    public enum Kind: String, Decodable, Sendable {
        case claudeAssistantMessage = "claude_assistant_message"
        case codexSessionMetadata = "codex_session_metadata"
        case codexTurnContext = "codex_turn_context"
        case codexAssistantMessage = "codex_assistant_message"
        case trajectoryMetadata = "trajectory_metadata"
    }
}

public struct InsightOutcomeLink: Decodable, Sendable, Identifiable {
    public let id, source_digest, linked_at: String
    public let provenance: Provenance
    public let evidence: Evidence
    public enum Provenance: String, Decodable, Sendable { case userLinked = "user_linked" }
    public enum Evidence: Decodable, Sendable {
        case gitCommit(GitCommit)
        case testReport(TestReport)
        private enum CodingKeys: String, CodingKey { case type, evidence }
        private enum Kind: String, Decodable { case gitCommit = "git_commit", testReport = "test_report" }
        public init(from decoder: Decoder) throws {
            let container = try decoder.container(keyedBy: CodingKeys.self)
            switch try container.decode(Kind.self, forKey: .type) {
            case .gitCommit: self = .gitCommit(try container.decode(GitCommit.self, forKey: .evidence))
            case .testReport: self = .testReport(try container.decode(TestReport.self, forKey: .evidence))
            }
        }
    }
    public struct GitCommit: Decodable, Sendable {
        public let repository_path_digest, object_id, tree_id, inspected_at: String
        public let parent_ids: [String]
        public let provenance: Authority
        public enum Authority: String, Decodable, Sendable { case inspectedLocalObject = "inspected_local_object" }
    }
    public struct TestReport: Decodable, Sendable {
        public let schema_version: UInt32
        public let runner, observed_at, artifact_digest, imported_at: String
        public let commit_id: String?
        public let passed, failed, skipped: UInt64
        public let provenance: Authority
        public enum Authority: String, Decodable, Sendable { case importedReport = "imported_report" }
    }
}

extension LocalInsight {
    func validateSupportedEvidence() throws {
        if let attribution = claude_task_attribution {
            guard source_format == "claude_code", attribution.schema_version == 1,
                  attribution.extractor_version == 1,
                  attribution.profile_id == "claude-code-v2.1.260-observed-agent-branch-v1",
                  attribution.observed_writer_version == "2.1.260",
                  attribution.qualification_scope == "observed_writer_agent_branch_records",
                  attribution.record_count > 0,
                  attribution.recognized_records <= attribution.record_count,
                  ["attributed", "unavailable"].contains(attribution.state.status) else {
                throw InsightsError.invalidResponse
            }
        }
        if let models = model_observations, models.contract == .unsupported {
            throw InsightsError.invalidResponse
        }
        for link in outcome_links ?? [] {
            if case .testReport(let report) = link.evidence, report.schema_version != 1 {
                throw InsightsError.invalidResponse
            }
        }
    }
}
