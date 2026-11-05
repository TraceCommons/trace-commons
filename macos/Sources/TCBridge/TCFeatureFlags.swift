import CTraceCommons
import Foundation
public enum TCFeatureFlags {
    /// Immutable build identity. Freeing the FFI string is owned here.
    public static func buildProfileJSON() -> String? {
        guard let raw = tc_feature_profile_json() else { return nil }
        defer { tc_string_free(raw) }
        return String(cString: raw)
    }
}
extension TCDaemon {
    /// Existing IPC frame; no networking or configuration evaluation in Swift.
    public func featureFlagsSnapshotJSON() -> String { call("feature_flags_snapshot") }
}
