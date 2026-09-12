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
