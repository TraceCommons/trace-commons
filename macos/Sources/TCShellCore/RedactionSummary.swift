import Foundation

/// One category's line in the scrubbing panel, as the core grouped it.
public struct RedactionSummaryRow: Decodable, Equatable, Sendable {
    /// The label family -- the part before the first `:`.
    public let family: String
    /// The family as a person reads it.
    public let display: String
    /// What this category IS. The panel's actual value to a reader who has
    /// never seen these words.
    public let description: String
    public let occurrences: Int
    public let distinct: Int
    /// The specific sub-labels this family covered. Safe to render: they are
    /// schema-shaped identifiers by construction, never contributor strings.
    /// Empty when the family had no sub-labels.
    public let detail: [String]

    public init(
        family: String,
        display: String,
        description: String,
        occurrences: Int,
        distinct: Int,
        detail: [String]
    ) {
        self.family = family
        self.display = display
        self.description = description
        self.occurrences = occurrences
        self.distinct = distinct
        self.detail = detail
    }

    /// "185 local path (12 distinct)", or "2 secret".
    ///
    /// The suffix appears only when it says something the occurrence count
    /// did not. `distinct` is zero for every family the placeholder map does
    /// not write to -- and a zero there means "no distinct count is
    /// available", never "no distinct values". Printing `(0 distinct)` beside
    /// a non-zero occurrence count would read as "nothing was removed",
    /// which is the one direction this panel must not fail in.
    public var countLine: String {
        guard distinct > 0, distinct < occurrences else {
            return "\(occurrences) \(display)"
        }
        return "\(occurrences) \(display) (\(distinct) distinct)"
    }
}

/// What scrubbing took out of this session, and what it found and left in,
/// decoded from `tc_redaction_summary_json` (`redaction_summary::summary_copy`).
///
/// The grouping by family, the split between what was removed and what is
/// still present, the order and every description are the core's. They used
/// to be computed here, with a table of descriptions transcribed from the
/// core; now this shell renders the two lists it is given, under their own
/// headings, and decides none of it.
///
/// It names KINDS, never values. The value is gone by construction.
public enum RedactionSummary {
    private struct Payload: Decodable {
        let removed: [RedactionSummaryRow]
        let stillPresent: [RedactionSummaryRow]

        enum CodingKeys: String, CodingKey {
            case removed
            case stillPresent = "still_present"
        }
    }

    /// The two lists, or nil if the payload will not parse. Nil is not "no
    /// rows": a panel that cannot read the core's answer says nothing rather
    /// than "nothing matched".
    public static func rows(
        fromJSON json: String?
    ) -> (removed: [RedactionSummaryRow], stillPresent: [RedactionSummaryRow])? {
        guard let data = json?.data(using: .utf8),
            let payload = try? JSONDecoder().decode(Payload.self, from: data)
        else {
            return nil
        }
        return (removed: payload.removed, stillPresent: payload.stillPresent)
    }
}
