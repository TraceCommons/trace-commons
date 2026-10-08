import CTraceCommons
import Foundation

/// The Flow 1 grant's preconditions, decided in the contributor core and
/// read across the C ABI rather than restated here.
///
/// Handle-free: `flow1::grant_request` is a pure function of the progress
/// the shell passes in. Decoding lives in `TCShellCore.Flow1GrantRequest`,
/// where it is tested without the dylib.
///
/// Nothing in this file is a word, and nothing in it is a branch.
public enum TCFlow1 {
    /// `tc_flow1_grant_request_json`: `{ready, blockers,
    /// witness_signing_address}` for a `Flow1Progress` encoded as JSON.
    /// Nil when the ABI returned NULL (an unreadable progress).
    public static func grantRequestJSON(progressJSON: String) -> String? {
        guard let raw = progressJSON.withCString({ tc_flow1_grant_request_json($0) }) else {
            return nil
        }
        defer { tc_string_free(raw) }
        return String(cString: raw)
    }
}
