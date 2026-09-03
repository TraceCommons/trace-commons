import Foundation

/// Reading the daemon's redaction count map, which does not mean what its
/// heading says it means.
///
/// `DeterministicTraceRedactor` sets `redaction_counts` to the WHOLE
/// redaction report. That report is mostly what you would expect -- one
/// entry per pattern that fired, counting values it took out -- but it also
/// carries `residual_secret_at:{path}`, which `note_residual_secret_location`
/// increments when a secret was **detected and NOT removed**: a credential
/// inside a correction the contributor wrote, which is preserved on purpose,
/// or a field the typed redaction traversal never visits, which is a real
/// gap.
///
/// Every shell renders that map under the heading "Removed by pattern". So a
/// session carrying a surviving secret has been reporting it as a thing that
/// was taken out -- the exact opposite of what happened, on the one screen
/// where somebody is deciding whether to send it.
///
/// This type is the single place that knows the difference. Both halves
/// matter and neither is optional:
///
/// * a survivor must not be counted as a removal, and
/// * a survivor must still be SHOWN. Filtering it out of the figure and
///   saying nothing else would trade a wrong statement for silence about a
///   secret that is still in the payload, which on a consent surface is not
///   an improvement.
public enum RedactionLabels {
    /// The label family marking a secret that was found and left in place.
    public static let residualPrefix = "residual_secret_at"

    /// What the card shows when nothing fired. `ScrubbingCaveat` supplies
    /// the sentence that says what that does and does not prove.
    public static let nothingMatched = "nothing matched"

    /// The part of a label before its first `:`.
    ///
    /// The count vocabulary is namespaced and OPEN -- `secret:{pattern_name}`,
    /// `privacy_filter:{label}` and `tool_sensitive_field:{action}` are all
    /// generated at redaction time -- so nothing here may assume a closed set
    /// of labels. Families are the only stable thing to reason about.
    public static func family(_ label: String) -> String {
        guard let colon = label.firstIndex(of: ":") else { return label }
        return String(label[label.startIndex..<colon])
    }

    /// Whether a label counts something that actually left the payload.
    public static func isRemoval(_ label: String) -> Bool {
        family(label) != residualPrefix
    }

    /// The counts for things that were genuinely removed.
    public static func removals(_ counts: [String: Int]) -> [String: Int] {
        counts.filter { isRemoval($0.key) }
    }

    /// Total occurrences removed. Never includes survivors.
    public static func removedTotal(_ counts: [String: Int]) -> Int {
        removals(counts).values.reduce(0, +)
    }

    /// "185 local path (12 distinct)  ·  3 secret"
    ///
    /// The daemon reports two maps. `redaction_counts` counts OCCURRENCES --
    /// how many times a pattern fired. `redactions_distinct` counts VALUES --
    /// how many different strings those firings covered, because the redactor
    /// mints one placeholder per distinct value and reuses it. One path
    /// referenced two hundred times is two hundred occurrences and one value,
    /// and a card that reports only the first overstates how much of the
    /// session was touched.
    ///
    /// Lives here rather than on the view because it is the only part of that
    /// card with a right and a wrong answer, and `swift test` cannot reach a
    /// SwiftUI body.
    ///
    /// Ordered by count so the biggest number is first, which is what a
    /// person scanning a column of cards is looking for; ties break on the
    /// label so the order is stable between two redraws.
    public static func line(occurrences: [String: Int], distinct: [String: Int]) -> String {
        let removed = removals(occurrences)
        if removed.isEmpty { return nothingMatched }
        return removed
            .sorted { $0.value == $1.value ? $0.key < $1.key : $0.value > $1.value }
            .map { label, count in
                let words = label.replacingOccurrences(of: "_", with: " ")
                // Only when it says something the occurrence count did not:
                // equal counts are the same fact twice, and a distinct count
                // above its occurrence count is impossible from a correct
                // daemon and not worth rendering from an incorrect one.
                guard let values = distinct[label], values > 0, values < count else {
                    return "\(count) \(words)"
                }
                return "\(count) \(words) (\(values) distinct)"
            }
            .joined(separator: "  ·  ")
    }

    /// Where secrets were found and left in the payload, with how many at
    /// each site, ordered for a stable rendering.
    ///
    /// The sites are schema-shaped identifiers -- `events.3.correction`, not
    /// a filesystem path and not transcript text. That is a property the
    /// redactor guarantees at the point these labels are minted, and it is
    /// what makes them safe to put on screen.
    public static func survivors(_ counts: [String: Int]) -> [(site: String, count: Int)] {
        counts
            .filter { !isRemoval($0.key) }
            .map { label, count in
                let prefix = residualPrefix + ":"
                let site = label.hasPrefix(prefix) ? String(label.dropFirst(prefix.count)) : ""
                return (site: site, count: count)
            }
            .sorted { $0.site < $1.site }
    }

    /// How many secrets were found and left in what would be sent.
    public static func survivorTotal(_ counts: [String: Int]) -> Int {
        counts.filter { !isRemoval($0.key) }.values.reduce(0, +)
    }

    /// The line shown when a session carries survivors, in the attention
    /// tone. Nil when there are none.
    ///
    /// Deliberately says "still in what would be sent" rather than naming a
    /// number of secrets: the count is of detection SITES, and one site can
    /// hold more than one value. Overstating precision here would be its own
    /// small lie.
    public static func survivorLine(_ counts: [String: Int]) -> String? {
        let total = survivorTotal(counts)
        guard total > 0 else { return nil }
        return total == 1
            ? "1 secret found here is still in what would be sent"
            : "\(total) secrets found here are still in what would be sent"
    }
}
