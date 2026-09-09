import CTraceCommons
import Foundation

/// What the outcome list says about a contribution the commons refused.
///
/// **`nil` means "not one of these", and the caller uses its own table.** Five
/// labels are answered across the ABI; everything else on that surface is
/// still this shell's own `OutcomeCopy`, pending the rest of
/// `queue_outcome_counts` moving into the Rust crate.
///
/// The reason this exists rather than a case in `OutcomeCopy`: that table's
/// default is `Held`, which for a refusal is vague where the Linux shell's
/// default was outright false, and `HealthCopy.swift` is pinned by the
/// shell-wording ratchet, so it cannot gain a sentence in place. The ratchet
/// is doing its job -- new copy belongs in the crate. See #810.
///
/// The label is the queue entry's `reason_label` in the server's own spelling,
/// with underscores.
public enum TCOutcome {
    public static func refusalLine(label: String) -> String? {
        guard let raw = label.withCString({ tc_outcome_refusal_line($0) }) else { return nil }
        defer { tc_string_free(raw) }
        let line = String(cString: raw)
        return line.isEmpty ? nil : line
    }
}
