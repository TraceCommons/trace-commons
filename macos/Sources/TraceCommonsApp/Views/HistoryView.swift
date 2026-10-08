// The legacy History screen's words. The screen itself is gone (R15):
// History draws on glass from `Views/Monitor/`. What it still reads of the
// old screen's is the core's now (#1146 parity, 2026-10-07).

/// History's sentences the glass History draws, read from the core's table
/// (`shell_words_copy::history_words`, through `ShellWords`).
enum HistoryLegacyWords {
    static var withdrawalWordingDefect: String? { WithdrawalCopy.wordingDefect }
    static var typicalWait: String { ShellWords.table?.history.typicalWait ?? "" }
}

/// The server's explanation lines across every held record, distinct, in
/// first-seen order, and without the lines that carry an opaque digest.
///
/// Distinct, because the server writes the same sentence for every trace
/// held for the same reason, and this list spans every held record at once;
/// order is kept so the first reason a contributor sees is the first that
/// occurred. No opaque digests, because every receipt carries `Attributed
/// to tenant tenant_sha256:<64 hex>`: true, and unreadable. The rule keys on
/// the digest rather than the sentence, so a future line carrying a hash is
/// caught without a list to maintain. A display filter, deliberately, not a
/// server change: the receipt is an API surface other consumers read.
enum HeldExplanations {
    static func lines(in explanations: [[String]]) -> [String] {
        var seen = Set<String>()
        var out: [String] = []
        for record in explanations {
            for line in record where !line.contains("sha256:") {
                if seen.insert(line).inserted {
                    out.append(line)
                }
            }
        }
        return out
    }
}
