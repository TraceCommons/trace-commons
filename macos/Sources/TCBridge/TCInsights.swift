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
        guard let result, error == nil,
              let text = String(validatingCString: result) else { throw InsightsError.operationFailed }
        do { return try JSONDecoder().decode(InsightsResponse.self, from: Data(text.utf8)) }
        catch { throw InsightsError.invalidResponse }
    }
}

public enum InsightsError: Error { case requestTooLarge, operationFailed, invalidResponse }
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
        public init(_ type: String, source: String? = nil, file: String? = nil,
                    save: Bool? = nil, id: String? = nil, category: String? = nil, outcome: String? = nil) {
            self.type = type; self.source = source; self.file = file; self.save = save
            self.id = id; self.category = category; self.outcome = outcome
        }
    }
}
public struct InsightsResponse: Decodable, Sendable {
    public let type: String
    public let insight: LocalInsight?
    public let insights: [LocalInsight]?
    public let deleted: Bool?
    public let copy: [String: String]?
    public let summary: SavedInsightsSummary?
}
public struct LocalInsight: Decodable, Sendable, Identifiable {
    public let id, source_format, boundary, analyzed_at: String
    public let report: Report
    public let estimated_cost_usd: Double?
    public let cost_unavailable_reason: String
    public let manual_annotation: Annotation?
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
