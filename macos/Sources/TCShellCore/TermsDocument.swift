import Foundation

/// The published Terms of Service (`trace_commons.terms.v1`), as the core
/// fetched and checked it (`tc_terms_document_json`). The core refuses an
/// unknown schema, an empty document and any link that is not `https://` or
/// `mailto:` before this type ever sees it; decoding here only reads the
/// shape. An unknown block kind fails the decode, so a document this build
/// cannot draw is never drawn in part.
///
/// Design: `docs/superpowers/specs/2026-11-09-terms-scroll-acceptance-design.md`.
public struct TermsDocument: Decodable, Equatable, Sendable {
    public let version: String
    public let effective: String
    /// `false` for a draft: never starts an upload hold, and the reader
    /// shows the core's not-in-effect line.
    public let inEffect: Bool
    public let title: String
    public let blocks: [Block]

    public init(version: String, effective: String, inEffect: Bool, title: String, blocks: [Block]) {
        self.version = version
        self.effective = effective
        self.inEffect = inEffect
        self.title = title
        self.blocks = blocks
    }

    /// One block of the document. Inline text is Markdown limited to
    /// emphasis, strong, code spans and links.
    public enum Block: Decodable, Equatable, Sendable {
        case heading(String)
        case paragraph(String)
        case list([String])
        case table(header: [String], rows: [[String]])
        case callout(String)

        private enum CodingKeys: String, CodingKey {
            case kind, text, items, header, rows
        }

        public init(from decoder: Decoder) throws {
            let c = try decoder.container(keyedBy: CodingKeys.self)
            let kind = try c.decode(String.self, forKey: .kind)
            switch kind {
            case "heading":
                self = .heading(try c.decode(String.self, forKey: .text))
            case "paragraph":
                self = .paragraph(try c.decode(String.self, forKey: .text))
            case "list":
                self = .list(try c.decode([String].self, forKey: .items))
            case "table":
                self = .table(
                    header: try c.decode([String].self, forKey: .header),
                    rows: try c.decode([[String]].self, forKey: .rows))
            case "callout":
                self = .callout(try c.decode(String.self, forKey: .text))
            default:
                throw DecodingError.dataCorruptedError(
                    forKey: .kind, in: c, debugDescription: "unknown block kind")
            }
        }
    }

    private enum CodingKeys: String, CodingKey {
        case version, effective, title, blocks
        case inEffect = "in_effect"
    }

    /// Decode a document, or nil.
    public static func decode(_ data: Data) -> TermsDocument? {
        try? JSONDecoder().decode(TermsDocument.self, from: data)
    }
}

/// A document and the lowercase hex SHA-256 of the exact bytes the site
/// served, which is what an acceptance names.
public struct FetchedTerms: Decodable, Equatable, Sendable {
    public let document: TermsDocument
    public let contentSHA256: String

    public init(document: TermsDocument, contentSHA256: String) {
        self.document = document
        self.contentSHA256 = contentSHA256
    }

    private enum CodingKeys: String, CodingKey {
        case document
        case contentSHA256 = "content_sha256"
    }

    /// `tc_terms_document_json`'s answer, or nil.
    public static func decode(_ json: String) -> FetchedTerms? {
        try? JSONDecoder().decode(FetchedTerms.self, from: Data(json.utf8))
    }
}

/// What the person accepted: the version and hash they read, and when.
public struct AcceptedTerms: Equatable, Sendable {
    public let version: String
    public let contentSHA256: String
    public let acceptedAt: Date

    public init(version: String, contentSHA256: String, acceptedAt: Date) {
        self.version = version
        self.contentSHA256 = contentSHA256
        self.acceptedAt = acceptedAt
    }

    /// Accepting `terms` now.
    public init(_ terms: FetchedTerms, at acceptedAt: Date) {
        self.init(
            version: terms.document.version, contentSHA256: terms.contentSHA256,
            acceptedAt: acceptedAt)
    }

    /// The daemon's `terms_accept` params: `version`, `content_sha256` and
    /// `accepted_at` in RFC 3339 UTC.
    public var ipcParams: [String: Any] {
        let format = ISO8601DateFormatter()
        format.formatOptions = [.withInternetDateTime]
        format.timeZone = TimeZone(identifier: "UTC")
        return [
            "version": version,
            "content_sha256": contentSHA256,
            "accepted_at": format.string(from: acceptedAt),
        ]
    }
}
