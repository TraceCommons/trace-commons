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

    /// One tool's word, chosen by the shared branch table.
    ///
    /// `wiring` is `TC_TOOL_WIRING_*`; `RoutingToolWiring.abiValue` in
    /// `TCShellCore` is what produces it. Nil when the ABI would produce no
    /// word -- a caught panic, or a source mode it could not read.
    ///
    /// THE BRANCH TABLE CROSSES, NOT ONLY THE WORDS. Reimplementing this
    /// `switch` in Swift is what let the branching drift in three places
    /// while every string it returned stayed identical.
    public static func toolWord(sourceMode: String, wiring: Int32) -> String? {
        guard let raw = sourceMode.withCString({ tc_routing_tool_word($0, wiring) }) else {
            return nil
        }
        defer { tc_string_free(raw) }
        return String(cString: raw)
    }

    /// How that word is painted, from the same two inputs: `TC_TOOL_TONE_*`.
    ///
    /// Never fails. A styling call that could fail would leave this shell
    /// choosing a tone for itself, which is what crossing the boundary is
    /// meant to stop. Do not recover this by comparing the rendered word
    /// against the private one.
    public static func toolTone(sourceMode: String, wiring: Int32) -> Int32 {
        sourceMode.withCString { tc_routing_tool_tone($0, wiring) }
    }

    /// The daemon's routing state, in words. Nil only on a caught panic; a
    /// state this build has never heard of reads as the off line.
    public static func stateLine(state: String) -> String? {
        guard let raw = state.withCString({ tc_routing_state_line($0) }) else { return nil }
        defer { tc_string_free(raw) }
        return String(cString: raw)
    }

    /// How firmly that sentence reads: `TC_ROUTING_TONE_*`.
    ///
    /// The last routing branch table that was still written out natively in
    /// each shell. Never fails -- a state this build has never heard of
    /// answers the neutral tone, exactly as its sentence claims nothing.
    public static func stateTone(state: String) -> Int32 {
        state.withCString { tc_routing_state_tone($0) }
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
