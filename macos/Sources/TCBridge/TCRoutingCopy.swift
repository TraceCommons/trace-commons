import CTraceCommons
import Foundation

/// The routing surface's words, read from the Rust rather than written here.
///
/// Handle-free for the same reason `TCScrubInfo` is: this describes the
/// build, not a running daemon.
///
/// Nothing in this file is a word. The vocabulary crosses as JSON and the
/// sentences cross already assembled, so there is no template for this shell
/// to fill in and therefore no fourth place the wording can drift. Decoding
/// lives in `TCShellCore`, where it is unit-tested without linking the dylib.
public enum TCRoutingCopy {
    /// Every fixed string on the surface, as a JSON object, or nil if the
    /// ABI reported a caught panic.
    ///
    /// GENERATED from `trace_commons_contributor::routing_copy`. Never
    /// transcribe one of these words into Swift: exactly one of them claims
    /// privacy, and a hand-written copy of that claim stops matching the day
    /// the claim changes, with nothing to notice.
    public static func copyJSON() -> String? {
        guard let raw = tc_routing_copy() else { return nil }
        defer { tc_string_free(raw) }
        return String(cString: raw)
    }

    /// "That file could not be used", assembled on the Rust side.
    ///
    /// `path` is nil when nothing resolved at all, which is a different
    /// sentence and not an error.
    public static func tokenLine(path: String?) -> String? {
        let raw: UnsafeMutablePointer<CChar>?
        if let path {
            raw = path.withCString { tc_routing_token_line($0) }
        } else {
            raw = tc_routing_token_line(nil)
        }
        guard let raw else { return nil }
        defer { tc_string_free(raw) }
        return String(cString: raw)
    }

    /// "Nothing answered", assembled on the Rust side. `port` is nil when no
    /// port was tried; the sentence for that names none.
    public static func unreachableLine(port: UInt16?) -> String? {
        guard let raw = tc_routing_unreachable_line(Int32(port ?? 0)) else { return nil }
        defer { tc_string_free(raw) }
        return String(cString: raw)
    }

    /// "Last checked ...", assembled on the Rust side around this shell's own
    /// humanised time. The time is the only part this shell renders, because
    /// it is a rendering of a timestamp and not wording about routing.
    ///
    /// Returns nil rather than a half-sentence when `when` is empty: the ABI
    /// refuses that case, and "Last checked " with nothing after it is worse
    /// than no line at all.
    public static func lastChecked(when: String) -> String? {
        guard !when.isEmpty else { return nil }
        guard let raw = when.withCString({ tc_routing_last_checked($0) }) else { return nil }
        defer { tc_string_free(raw) }
        return String(cString: raw)
    }
}
