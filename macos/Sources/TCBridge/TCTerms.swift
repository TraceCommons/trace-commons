import CTraceCommons
import Foundation

/// The core's refusal label for a terms fetch: `terms-unavailable`,
/// `terms-host-refused`, `terms-too-large`, a document refusal such as
/// `terms-schema-unknown`, or `panic`. A label, never shown as text.
public struct TCTermsRefusal: Error, Equatable, Sendable {
    public let label: String
    public init(label: String) { self.label = label }
}

/// The published Terms of Service, fetched by the core across the C ABI so
/// every shell reads the same bytes and the same hash. Needs no daemon.
public enum TCTerms {
    /// `tc_terms_document_json`: `{"document": ..., "content_sha256": ...}`
    /// as JSON, decoded by `TCShellCore.FetchedTerms`. Blocks for up to the
    /// fetch's timeouts, so call it off the main actor.
    public static func documentJSON() -> Result<String, TCTermsRefusal> {
        var err: UnsafeMutablePointer<CChar>? = nil
        guard let raw = tc_terms_document_json(&err) else {
            let label = err.map { String(cString: $0) } ?? "terms-unavailable"
            if let err { tc_string_free(err) }
            return .failure(TCTermsRefusal(label: label))
        }
        defer { tc_string_free(raw) }
        return .success(String(cString: raw))
    }
}
