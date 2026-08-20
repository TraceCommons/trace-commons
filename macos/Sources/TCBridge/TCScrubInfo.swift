import CTraceCommons
import Foundation

/// `tc_scrub_detector_names`: what the local scrubber removes.
///
/// Handle-free for the same reason `TCDiscovery` is -- the screen that
/// consumes it is the first one a contributor sees, before any daemon exists.
///
/// Raw JSON crosses this boundary; decoding and prettification live in
/// `TCShellCore`, where they are unit-tested without linking the FFI dylib.
public enum TCScrubInfo {
    /// The detector names as a JSON array of strings, or nil if the ABI
    /// reported a caught panic.
    ///
    /// The list is GENERATED from the scrubber's own table. It is never
    /// transcribed into Swift: a hand-written list of what is removed is a
    /// privacy claim that stops being true the day a detector is added, and
    /// nothing would fail when it did.
    public static func detectorNamesJSON() -> String? {
        guard let raw = tc_scrub_detector_names() else { return nil }
        defer { tc_string_free(raw) }
        return String(cString: raw)
    }
}
