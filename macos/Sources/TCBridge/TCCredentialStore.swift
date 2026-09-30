import CTraceCommons
import Foundation

/// Whether this process can reach the Cloud credential store.
///
/// Handle-free, like `TCWitness`: this is a release-pipeline probe, not a
/// daemon call, and it neither reads nor writes a stored credential.
public enum TCCredentialStore {

    /// `0` reachable, `1` unentitled, `2` on any other failure.
    ///
    /// Only a *signed* bundle can answer this meaningfully -- entitlements
    /// are a property of the code signature, which `cargo test` never has.
    public static func selfCheck() -> Int32 {
        tc_credential_store_self_check()
    }
}
