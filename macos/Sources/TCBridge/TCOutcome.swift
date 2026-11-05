import CTraceCommons
import Foundation

/// Shared queue outcome and admission-refusal copy.
public enum TCOutcome {
    public static func line(label: String) -> String {
        // Legacy equivalent: tc_queue_outcome_line.
        LocalizedPresentation.line("queue_outcome_line", parameters: ["label": label]) ?? ""
    }

    public static func refusalLine(label: String) -> String? {
        // Legacy equivalent: tc_outcome_refusal_line.
        guard let line = LocalizedPresentation.line("outcome_refusal_line", parameters: ["label": label]) else { return nil }
        return line.isEmpty ? nil : line
    }
}
