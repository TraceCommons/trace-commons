import CTraceCommons
import Foundation

/// Local, handle-free service. Call off the UI thread; mutations already in
/// progress finish even if the presentation is closed.
public enum TCInsights {
    public static func call(_ request: InsightsRequest) throws -> InsightsResponse {
        let bytes = try JSONEncoder().encode(request)
        guard bytes.count <= 65_536 else { throw InsightsError.requestTooLarge }
        var error: UnsafeMutablePointer<CChar>?
        let result = bytes.withUnsafeBytes { buffer in
            tc_insights_call(buffer.bindMemory(to: UInt8.self).baseAddress, bytes.count, &error)
        }
        defer {
            if let result { tc_string_free(result) }
            if let error { tc_string_free(error) }
        }
        if let error, let code = String(validatingCString: error) {
            throw InsightsError.service(code)
        }
        guard let result, let text = String(validatingCString: result) else { throw InsightsError.operationFailed }
        do {
            let response = try JSONDecoder().decode(InsightsResponse.self, from: Data(text.utf8))
            try response.insight?.validateSupportedEvidence()
            for snapshot in response.insights ?? [] { try snapshot.validateSupportedEvidence() }
            try response.task?.validateSupportedSchema()
            try response.comparisonTaskDetail?.validateSupportedSchema(expectedID: request.operation.id)
            for task in response.tasks ?? [] { try task.validateSupportedSchema() }
            if request.operation.type == "question_cards" {
                guard response.type == "question_cards", let result = response.result,
                      response.text != nil, let questions = request.operation.questions else {
                    throw InsightsError.invalidResponse
                }
                try result.validateSupportedSchema(expectedQuestions: questions)
            }
            return response
        }
        catch { throw InsightsError.invalidResponse }
    }
}

public enum InsightsError: Error, Equatable {
    case requestTooLarge, operationFailed, invalidResponse
    case service(String)
}
public struct InsightsRequest: Encodable, Sendable {
    public let store_dir: String?
    public let operation: Operation
    public init(storeDirectory: String? = nil, operation: Operation) {
        store_dir = storeDirectory; self.operation = operation
    }
    public struct Operation: Encodable, Sendable {
        public let type: String
        public var source: String?
        public var file: String?
        public var save: Bool?
        public var id: String?
        public var category: String?
        public var outcome: String?
        public var repository: String?
        public var commit: String?
        public var evidence_id: String?
        public var snapshot_ids: [String]?
        public var episode_ids: [String]?
        public var questions: [InsightQuestion]?
        public var expected_revision: UInt64?
        public var context: ComparisonTaskContextInput?
        public var displayed_material_digest: String?
        public var input: ComparisonSpecificationDraftInput?
        public var specification_id: String?
        public var audit_digest: String?
        public init(_ type: String, source: String? = nil, file: String? = nil,
                    save: Bool? = nil, id: String? = nil, category: String? = nil, outcome: String? = nil,
                    repository: String? = nil, commit: String? = nil, evidenceID: String? = nil,
                    snapshotIDs: [String]? = nil, episodeIDs: [String]? = nil,
                    questions: [InsightQuestion]? = nil, expectedRevision: UInt64? = nil,
                    context: ComparisonTaskContextInput? = nil,
                    displayedMaterialDigest: String? = nil,
                    input: ComparisonSpecificationDraftInput? = nil,
                    specificationID: String? = nil, auditDigest: String? = nil) {
            self.type = type; self.source = source; self.file = file; self.save = save
            self.id = id; self.category = category; self.outcome = outcome
            self.repository = repository; self.commit = commit; self.evidence_id = evidenceID
            self.snapshot_ids = snapshotIDs
            self.episode_ids = episodeIDs; self.questions = questions
            self.expected_revision = expectedRevision
            self.context = context; self.displayed_material_digest = displayedMaterialDigest
            self.input = input
            self.specification_id = specificationID; self.audit_digest = auditDigest
        }
    }
}
public struct InsightMutationEffects: Decodable, Sendable {
    public let invalidated_episode_ids: [String]
    public let stale_comparison_task_ids: [String]?
}
public struct InsightsResponse: Decodable, Sendable {
    public let type: String
    public let insight: LocalInsight?
    public let insights: [LocalInsight]?
    public let deleted: Bool?
    public let copy: [String: String]?
    public let summary: SavedInsightsSummary?
    public let mutation_effects: InsightMutationEffects?
    public let episode: LocalEpisode?
    public let episodes: [EpisodeListEntry]?
    public let detail: EpisodeDetail?
    public let result: InsightCardResult?
    public let text: String?
    public let task: LocalComparisonTask?
    public let tasks: [ComparisonTaskDetail]?
    public let comparisonTaskDetail: ComparisonTaskDetail?
    public let specification: ComparisonSpecification?
    public let specifications: [ComparisonSpecification]?
    public let comparisonResult: DescriptiveComparisonResult?
    public var invalidatedEpisodeIDs: [String] { mutation_effects?.invalidated_episode_ids ?? [] }
    public var staleComparisonTaskIDs: [String] { mutation_effects?.stale_comparison_task_ids ?? [] }

    private enum CodingKeys: String, CodingKey {
        case type, insight, insights, deleted, copy, summary, mutation_effects, episode, episodes
        case detail, result, text, task, tasks, specification, specifications
    }
    public init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        type = try values.decode(String.self, forKey: .type)
        insight = try values.decodeIfPresent(LocalInsight.self, forKey: .insight)
        insights = try values.decodeIfPresent([LocalInsight].self, forKey: .insights)
        deleted = try values.decodeIfPresent(Bool.self, forKey: .deleted)
        copy = try values.decodeIfPresent([String: String].self, forKey: .copy)
        summary = try values.decodeIfPresent(SavedInsightsSummary.self, forKey: .summary)
        mutation_effects = try values.decodeIfPresent(InsightMutationEffects.self, forKey: .mutation_effects)
        episode = try values.decodeIfPresent(LocalEpisode.self, forKey: .episode)
        episodes = try values.decodeIfPresent([EpisodeListEntry].self, forKey: .episodes)
        if type == "comparison_task_explain" {
            detail = nil
            comparisonTaskDetail = try values.decodeIfPresent(ComparisonTaskDetail.self, forKey: .detail)
        } else {
            detail = try values.decodeIfPresent(EpisodeDetail.self, forKey: .detail)
            comparisonTaskDetail = nil
        }
        if type == "comparison_preview_spec" || type == "comparison_result" {
            result = nil
            comparisonResult = try values.decodeIfPresent(DescriptiveComparisonResult.self, forKey: .result)
        } else {
            result = try values.decodeIfPresent(InsightCardResult.self, forKey: .result)
            comparisonResult = nil
        }
        text = try values.decodeIfPresent(String.self, forKey: .text)
        task = try values.decodeIfPresent(LocalComparisonTask.self, forKey: .task)
        tasks = try values.decodeIfPresent([ComparisonTaskDetail].self, forKey: .tasks)
        specification = try values.decodeIfPresent(ComparisonSpecification.self, forKey: .specification)
        specifications = try values.decodeIfPresent([ComparisonSpecification].self, forKey: .specifications)
    }
}

public enum InsightQuestion: String, Codable, Sendable, CaseIterable {
    case recordedActivity = "recorded_activity"
    case episodeOutcomes = "episode_outcomes"
    case observedModels = "observed_models"
    case estimatedCost = "estimated_cost"
    public var copyKey: String { "card_question_\(rawValue)" }
}
public enum InsightCardState: String, Decodable, Sendable { case observed, partial, unavailable }
public enum InsightCardValue: Decodable, Sendable {
    case count(UInt64), unixMilliseconds(Int64), milliseconds(UInt64)
    private enum Keys: String, CodingKey { case type, value }
    private enum Kind: String, Decodable { case count, unixMilliseconds = "unix_milliseconds", milliseconds }
    public init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: Keys.self)
        switch try values.decode(Kind.self, forKey: .type) {
        case .count: self = .count(try values.decode(UInt64.self, forKey: .value))
        case .unixMilliseconds: self = .unixMilliseconds(try values.decode(Int64.self, forKey: .value))
        case .milliseconds: self = .milliseconds(try values.decode(UInt64.self, forKey: .value))
        }
    }
}
public struct InsightCardRow: Decodable, Sendable, Identifiable {
    public let id, unit: String
    public let label: String?
    public let value: InsightCardValue?
    public let missing_reason: String?
}
public struct InsightCardCoverage: Decodable, Sendable, Identifiable {
    public let unit: String
    public let observed, eligible: UInt64
    public var id: String { unit }
}
public struct InsightEpisodeDenominator: Decodable, Sendable {
    public let eligible, assessed, unassessed, explicit_unknown: UInt64
}
public struct InsightQuestionCard: Decodable, Sendable, Identifiable {
    public let question: InsightQuestion
    public let metric_version: String
    public let state: InsightCardState
    public let rows: [InsightCardRow]
    public let coverage: [InsightCardCoverage]
    public let episode_denominator: InsightEpisodeDenominator?
    public let evidence_ids, episode_ids, limitations: [String]
    public let rows_omitted: Bool
    public var id: InsightQuestion { question }
}
public struct InsightCardResult: Decodable, Sendable {
    public struct Provider: Decodable, Sendable {
        public let id, version, rubric_version, execution_mode: String
        public let schema_version: UInt32
    }
    public let schema_version: UInt32
    public let provider: Provider
    public let input_digest: String
    public let cards: [InsightQuestionCard]
    public func validateSupportedSchema(expectedQuestions: [InsightQuestion]) throws {
        let digest = input_digest.utf8
        guard schema_version == 1, provider.schema_version == 1,
              provider.id == "trace-commons-local", provider.version == "1",
              provider.rubric_version == "deterministic-question-cards-v1", provider.execution_mode == "local",
              digest.count == 64, digest.allSatisfy({ byte in
                  (UInt8(ascii: "0")...UInt8(ascii: "9")).contains(byte)
                    || (UInt8(ascii: "a")...UInt8(ascii: "f")).contains(byte)
              }), cards.map(\.question) == expectedQuestions,
              Set(cards.map(\.question)).count == cards.count,
              cards.allSatisfy({ card in
                  card.metric_version == Self.metricVersion(card.question)
                    && card.evidence_ids.allSatisfy(Self.isDigest)
                    && card.episode_ids.allSatisfy { UUID(uuidString: $0)?.uuidString.lowercased() == $0 }
                    &&
                  Set(card.coverage.map(\.unit)).count == card.coverage.count
                    && card.rows.allSatisfy { ($0.value == nil) != ($0.missing_reason == nil) }
                    && Set(card.evidence_ids).count == card.evidence_ids.count
                    && Set(card.episode_ids).count == card.episode_ids.count
              }) else { throw InsightsError.invalidResponse }
    }
    private static func metricVersion(_ question: InsightQuestion) -> String {
        switch question {
        case .recordedActivity: "recorded-activity-v1"
        case .episodeOutcomes: "episode-outcomes-v1"
        case .observedModels: "observed-models-v1"
        case .estimatedCost: "estimated-cost-v1"
        }
    }
    private static func isDigest(_ value: String) -> Bool {
        value.utf8.count == 64 && value.utf8.allSatisfy {
            (UInt8(ascii: "0")...UInt8(ascii: "9")).contains($0)
                || (UInt8(ascii: "a")...UInt8(ascii: "f")).contains($0)
        }
    }
}
public struct LocalEpisode: Decodable, Sendable, Identifiable, Equatable {
    public let schema_version: UInt32
    public let id: String
    public let revision, membership_revision: UInt64
    public let created_at, updated_at, provenance: String
    public let members: [EpisodeMember]
    public let manual_assessment: EpisodeAssessment?
    public func validateSupportedSchema() throws {
        guard schema_version == 1, provenance == "user_selected_whole_snapshots",
              revision > 0, membership_revision > 0, membership_revision <= revision,
              !members.isEmpty, UUID(uuidString: id)?.uuidString.lowercased() == id,
              Set(members.map(\.snapshot_id)).count == members.count,
              members.allSatisfy({ Self.isDigest($0.snapshot_id) && Self.isDigest($0.source_digest) })
        else { throw InsightsError.invalidResponse }
        if let assessment = manual_assessment {
            guard assessment.provenance == "user_reported",
                  assessment.membership_revision == membership_revision,
                  ["unknown", "refactor", "tests", "docs", "debugging", "other"].contains(assessment.category),
                  ["unknown", "accepted", "partial", "rejected"].contains(assessment.outcome),
                  Self.isDigest(assessment.members_digest) else {
                throw InsightsError.invalidResponse
            }
        }
    }
    private static func isDigest(_ value: String) -> Bool {
        value.utf8.count == 64 && value.utf8.allSatisfy {
            (UInt8(ascii: "0")...UInt8(ascii: "9")).contains($0)
                || (UInt8(ascii: "a")...UInt8(ascii: "f")).contains($0)
        }
    }
}
public struct EpisodeMember: Decodable, Sendable, Equatable {
    public let snapshot_id, source_digest: String
}
public struct EpisodeAssessment: Decodable, Sendable, Equatable {
    public let category, outcome, provenance, recorded_at: String
    public let membership_revision: UInt64
    public let members_digest: String
}
public struct EpisodeListEntry: Decodable, Sendable, Identifiable, Equatable {
    public let episode: LocalEpisode
    public let overlapping_episode_ids: [String]
    public var id: String { episode.id }
}
public struct EpisodeOverlap: Decodable, Sendable, Equatable {
    public let snapshot_id: String
    public let episode_ids: [String]
}
public struct EpisodeDetail: Decodable, Sendable {
    public let episode: LocalEpisode
    public let members: [LocalInsight]
    public let overlap: [EpisodeOverlap]
    public let resolved_at: String
    public func validateSupportedSchema(expectedID: String? = nil) throws {
        try episode.validateSupportedSchema()
        if let expectedID, episode.id != expectedID { throw InsightsError.invalidResponse }
        let memberIDs = Set(episode.members.map(\.snapshot_id))
        let overlapIDs = overlap.map(\.snapshot_id)
        guard Set(members.map(\.id)) == memberIDs,
              Set(overlapIDs).isSubset(of: memberIDs), Set(overlapIDs).count == overlapIDs.count else {
            throw InsightsError.invalidResponse
        }
        for member in members { try member.validateSupportedEvidence() }
    }
}
public struct LocalInsight: Decodable, Sendable, Identifiable {
    public let id, source_format, boundary, analyzed_at: String
    public let report: Report
    public let estimated_cost_usd: Double?
    public let cost_unavailable_reason: String
    public let manual_annotation: Annotation?
    public let model_observations: InsightModelObservations?
    public let outcome_links: [InsightOutcomeLink]?
    public struct Annotation: Decodable, Sendable {
        public let category, outcome, provenance, recorded_at, source_digest: String
    }
    public struct Report: Decodable, Sendable {
        public let schema_version: UInt32
        public let provider: Provider
        public let metrics: [Metric]
        public let evidence: [Evidence]
    }
    public struct Provider: Decodable, Sendable {
        public let id, version, rubric_version, execution_mode: String
    }
    public struct Metric: Decodable, Sendable, Identifiable {
        public let id: String
        public let value: UInt64?
        public let coverage: Coverage
        public let evidence_ids: [String]
    }
    public struct Coverage: Decodable, Sendable { public let observed, total: UInt64 }
    public struct Evidence: Decodable, Sendable, Identifiable { public let id, source_digest: String }
}

/// Shared saved-history reducer output. Counts are supplied by Rust, never
/// reconstructed from snapshot rows or interpreted as model performance.
public struct SavedInsightsSummary: Decodable, Sendable {
    public let schema_version: UInt32
    public let scope: Scope
    public let limitations: [Limitation]
    public let provider: LocalInsight.Provider
    public let saved_snapshots: UInt64
    public let snapshot_analysis_range: AnalysisRange?
    public let user_reported: Assessments
    public let metrics: [Metric]
    public let snapshots: [Snapshot]

    public enum Scope: String, Decodable, Sendable {
        case allSavedSelectedSessionSnapshots = "all_saved_selected_session_snapshots"
    }
    public enum Limitation: String, Decodable, Sendable, CaseIterable, Hashable {
        case selectedSavedSessionsAreNotVerifiedTasks = "selected_saved_sessions_are_not_verified_tasks"
        case assessmentsAreUserReported = "assessments_are_user_reported"
        case observedSumsRequireBothCoverages = "observed_sums_require_both_coverages"
        case analysisDatesAreNotActivityTime = "analysis_dates_are_not_activity_time"
        case sourceFormatsAreNotModelIdentity = "source_formats_are_not_model_identity"
        case noModelRankingsTimeSavingsOrCost = "no_model_rankings_time_savings_or_cost"
    }
    public enum CoverageUnit: String, Decodable, Sendable {
        case sessionSnapshots = "session_snapshots"
        case normalizedEvents = "normalized_events"
        case toolResults = "tool_results"
    }
    public struct AnalysisRange: Decodable, Sendable {
        public let oldest, newest: String
    }
    public struct Assessments: Decodable, Sendable {
        public let assessed_snapshots, unassessed_snapshots: UInt64
        public let categories: [Category]
        public let outcomes: [Outcome]
    }
    public struct Category: Decodable, Sendable, Identifiable {
        public let category: String
        public let snapshots: UInt64
        public let evidence_snapshot_ids: [String]
        public var id: String { category }
    }
    public struct Outcome: Decodable, Sendable, Identifiable {
        public let outcome: String
        public let snapshots: UInt64
        public let evidence_snapshot_ids: [String]
        public var id: String { outcome }
    }
    public struct Metric: Decodable, Sendable, Identifiable {
        public let id: String
        public let observed_value_sum: UInt64?
        public let available_snapshots, missing_snapshots: UInt64
        public let record_coverage: LocalInsight.Coverage
        public let coverage_unit: CoverageUnit
        public let evidence_snapshot_ids: [String]
    }
    public struct Snapshot: Decodable, Sendable, Identifiable {
        public let id, source_format, analyzed_at: String
        public let evidence: [LocalInsight.Evidence]
    }

    /// Reject unsupported summary meaning before a native view presents it.
    public func validateSupportedSchema() throws {
        guard schema_version == 1,
              Set(limitations) == Set(Limitation.allCases) else {
            throw InsightsError.invalidResponse
        }
    }
}
