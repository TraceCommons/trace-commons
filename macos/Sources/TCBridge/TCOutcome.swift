import CTraceCommons
import Foundation

/// Shared queue outcome and admission-refusal copy.
public enum TCOutcome {
    public static func line(label: String) -> String {
        guard let raw = label.withCString({ tc_queue_outcome_line($0) }) else { return "" }
        defer { tc_string_free(raw) }
        return String(cString: raw)
    }

    public static func refusalLine(label: String) -> String? {
        guard let raw = label.withCString({ tc_outcome_refusal_line($0) }) else { return nil }
        defer { tc_string_free(raw) }
        let line = String(cString: raw)
        return line.isEmpty ? nil : line
    }
}
