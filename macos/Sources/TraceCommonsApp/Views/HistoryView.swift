// The legacy History screen's words, at the path the wording ratchet
// (`ShellWordingTests`) records them under.
//
// The screen itself is gone (R15): History draws on glass from
// `Views/Monitor/`, and those files author no sentence. What it still reads
// of the old screen's is held here, verbatim, until the core exports it.

// MARK: - Copy

/// History's own sentences. Withdrawal's live in `WithdrawalCopy`; these are
/// the ones about a state rather than an act.
enum HistoryCopy {
    /// What "held for privacy review" means, on the row it applies to. Held,
    /// never rejected, and with no turnaround time stated -- there is no
    /// number this app could honour.
    static let heldExplanation =
        "Automated checks saw something that might be personal and couldn't decide on "
        + "their own. It has not been rejected, and it has not been shared with anyone "
        + "but the reviewer."
}

/// History's sentences the glass History draws, held once.
enum HistoryLegacyWords {
    static let withdrawalWordingDefect = "Do not trust the withdrawal wording on this screen."
    static let typicalWait = "Typical wait: we don't have a reliable number yet."
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
