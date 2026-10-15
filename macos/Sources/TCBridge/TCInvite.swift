import CTraceCommons
import Foundation

/// The host an invite names, read across the C ABI so the shell can show the
/// instance before a contributor commits, without parsing the invite itself.
///
/// Only the host crosses: the invite's code stays in the core. Every
/// rejection is the same nil, because the invite path has one failure
/// state and no caller may tell the causes apart.
public enum TCInvite {
    /// `tc_invite_issuer_host`: the invite's host, or nil for anything the
    /// core does not accept as an invite.
    public static func issuerHost(_ invite: String) -> String? {
        guard let raw = invite.withCString({ tc_invite_issuer_host($0) }) else {
            return nil
        }
        defer { tc_string_free(raw) }
        return String(cString: raw)
    }
}
