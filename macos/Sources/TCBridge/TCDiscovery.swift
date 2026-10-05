import CTraceCommons
import Foundation

/// `tc_discover_sources`: what session stores are on this machine.
///
/// Handle-free on purpose. It runs BEFORE any daemon exists, because the
/// screen that uses it is the one clearing the refusal that stops a daemon
/// from starting -- so there is nothing to hold a handle to yet.
///
/// This returns the raw JSON rather than decoded models so the C boundary
/// stays in this target and the decoding stays in `TCShellCore`, where it is
/// unit-tested without linking the FFI dylib at all.
public enum TCDiscovery {
    /// The discovery array as JSON, or nil if the ABI reported a caught
    /// panic. Never throws: a roots screen that cannot describe the machine
    /// still has to render, because a contributor can always name a folder
    /// by hand.
    public static func sourcesJSON() -> String? {
        guard let raw = tc_discover_sources() else { return nil }
        defer { tc_string_free(raw) }
        return String(cString: raw)
    }

    /// `tc_describe_folder`: the kinds whose layout a picked folder matches,
    /// as a JSON array of rows shaped like `sourcesJSON()`'s, decoded with
    /// `SourceCandidate.decodeList(from:)`. Empty for a folder that matches
    /// none. Nil only when the ABI reported an error, which for a Swift
    /// string argument means a caught panic.
    public static func describeFolderJSON(_ path: String) -> String? {
        guard let raw = path.withCString({ tc_describe_folder($0) }) else { return nil }
        defer { tc_string_free(raw) }
        return String(cString: raw)
    }
}
