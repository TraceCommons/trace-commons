import Foundation

/// One category's line in the scrubbing panel.
public struct RedactionSummaryRow: Equatable {
    /// The label family -- the part before the first `:`.
    public let family: String
    /// The family as a person reads it.
    public let display: String
    /// What this category IS. The panel's actual value to a reader who has
    /// never seen these words.
    public let description: String
    public let occurrences: Int
    public let distinct: Int
    /// The specific sub-labels this family covered, humanised. Safe to
    /// render: sub-labels are schema-shaped identifiers by construction --
    /// `log_residual_secret_locations` depends on the same property -- never
    /// contributor strings. Empty when the family had no sub-labels.
    public let detail: [String]
}

/// What scrubbing took out of this session, and what it found and left in.
///
/// Marking placeholders in the transcript answers *where*. This answers
/// *what*, without scrolling, which is the half the card's one-line figure
/// could only gesture at.
///
/// It names KINDS, never values. The value is gone by construction, and a
/// panel listing the actual strings would make the preview window the single
/// best thing on the machine to photograph.
public enum RedactionSummary {
    /// What each family is, in words. Deliberately not exhaustive -- the
    /// vocabulary is generated and open -- which is why `describe` has a
    /// neutral fallback rather than a `fatalError`.
    static let descriptions: [String: String] = [
        "local_path": "File paths from this machine.",
        "secret": """
        API keys, tokens, private keys, and high-entropy strings found next to \
        credential words.
        """,
        "privacy_filter": "Names, emails, and other personal details found in prose.",
        "sensitive_field": "Fields whose name marks them sensitive, like password or authorization.",
        "tool_sensitive_field": "Tool-call arguments whose name marks them sensitive.",
        RedactionLabels.residualPrefix: """
        Found, and still in what would be sent. Either a credential inside a \
        correction you wrote, which is kept on purpose, or a field scrubbing \
        does not reach.
        """,
    ]

    /// The neutral description for a family this build has no words for.
    ///
    /// It must still appear. Dropping an unrecognised category would
    /// understate what happened, and this panel may only ever err toward
    /// saying more than it can explain.
    static let unknownDescription = "Removed by a pattern this version has no description for."

    static func describe(_ family: String) -> String {
        descriptions[family] ?? unknownDescription
    }

    static func humanise(_ text: String) -> String {
        text.replacingOccurrences(of: "_", with: " ")
    }

    public static func rows(
        occurrences: [String: Int],
        distinct: [String: Int]
    ) -> (removed: [RedactionSummaryRow], stillPresent: [RedactionSummaryRow]) {
        var byFamily: [String: (occurrences: Int, distinct: Int, detail: [String])] = [:]
        for (label, count) in occurrences {
            let family = RedactionLabels.family(label)
            var bucket = byFamily[family] ?? (0, 0, [])
            bucket.occurrences += count
            bucket.distinct += distinct[label] ?? 0
            if label != family {
                bucket.detail.append(humanise(String(label.dropFirst(family.count + 1))))
            }
            byFamily[family] = bucket
        }

        // Built in two statements rather than one chain: as a single
        // expression the type-checker gives up on it.
        var all: [RedactionSummaryRow] = []
        all.reserveCapacity(byFamily.count)
        for (family, bucket) in byFamily {
            all.append(
                RedactionSummaryRow(
                    family: family,
                    display: humanise(family),
                    description: describe(family),
                    occurrences: bucket.occurrences,
                    distinct: bucket.distinct,
                    detail: bucket.detail.sorted()
                )
            )
        }
        all.sort { left, right in
            if left.occurrences == right.occurrences { return left.family < right.family }
            return left.occurrences > right.occurrences
        }

        return (
            removed: all.filter { RedactionLabels.isRemoval($0.family) },
            stillPresent: all.filter { !RedactionLabels.isRemoval($0.family) }
        )
    }
}
