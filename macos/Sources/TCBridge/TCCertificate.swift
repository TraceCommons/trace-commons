import CTraceCommons
import Foundation

/// The certificate-held list's two readings, across the C ABI.
///
/// Handle-free, like `TCAttestation`: these describe the build rather than a
/// running daemon. Both sentences come from
/// `crates/trace-commons-contributor/src/private_inference_copy.rs`, so this
/// shell, the GTK shell and the Windows shell say one thing.
///
/// **The choice between the two readings is behind the ABI and not here.**
/// One fact -- a witness certificate is held for the bytes a queue row was
/// pinned to -- reads as "you can put this forward" for a contributor with no
/// invite and as "what you send carries signed proof" for one with an invite.
/// Three shells each writing that choice would be three chances to swap them,
/// and a swapped reading tells a contributor with no invite that their
/// session carries cryptographic proof when nothing has attested it.
///
/// **`evidenceAdmitted` is `admission_evidence_required` verbatim, never its
/// negation.** That flag is true for a contributor who signed up through NEAR
/// and therefore has NO invite. Passing `!flag` here would swap both
/// readings and compile.
///
/// Not `TCAttestation`. That answers whether a session carries a copy of the
/// model call that produced it; this answers whether a certificate is held
/// over the reviewed bytes. A session can have either without the other.
public enum TCCertificate {
    /// The sentence for one row of the list.
    ///
    /// `nil` means a caught Rust panic. The caller draws nothing rather than
    /// filling the hole in: a row in this list with no sentence is a row
    /// claiming something unstated.
    public static func rowLine(evidenceAdmitted: Bool) -> String? {
        guard let raw = tc_certificate_row_line(evidenceAdmitted ? 1 : 0) else { return nil }
        defer { tc_string_free(raw) }
        return String(cString: raw)
    }

    /// The heading over that list, on the same split and the same argument.
    public static func listTitle(evidenceAdmitted: Bool) -> String? {
        guard let raw = tc_certificate_list_title(evidenceAdmitted ? 1 : 0) else { return nil }
        defer { tc_string_free(raw) }
        return String(cString: raw)
    }
}
