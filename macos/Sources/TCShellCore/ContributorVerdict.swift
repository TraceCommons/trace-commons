import Foundation

/// The contributor's own answer to "did this session do what you asked?",
/// carried as `approve`'s optional `outcome` parameter.
///
/// Three cases and no fourth. The absence of an answer is NOT a case here:
/// it is `nil`, and it means the `outcome` key is omitted from the call
/// entirely. The daemon distinguishes an absent parameter
/// (`TaskSuccess::Unknown`, approval proceeds) from an unrecognised one
/// (refused with `bad_params` / `outcome-invalid`, approving nothing), so a
/// shell that sends `null` or `""` for "no answer" fails the approval it
/// meant to make. See `docs/contributor-daemon-ipc-v1_1.md`, "The `outcome`
/// verdict".
///
/// The raw values are the wire words and must stay lowercase.
public enum ContributorVerdict: String, CaseIterable, Sendable {
    case worked
    case partly
    case failed

    /// What the contributor sees on the control. The wire value is never
    /// shown, and the label is never sent.
    public var label: String {
        switch self {
        case .worked: return VerdictCopy.worked
        case .partly: return VerdictCopy.partly
        case .failed: return VerdictCopy.failed
        }
    }
}

/// Copy for the verdict question, shared with the other two shells.
///
/// It lives in `TCShellCore` for the reason `ReadGate` does: the app target
/// links the FFI dylib and so is awkward to assert against, and this text is
/// written three times across three shells. The Linux original is
/// `crates/trace-commons-contributor-gtk/src/copy.rs` (`VERDICT_QUESTION`,
/// `VERDICT_WORKED`, `VERDICT_PARTLY`, `VERDICT_FAILED`, `VERDICT_CAPTION`,
/// `SUBMIT_ALL_AS`, `SUBMIT_ALL_AS_TOOLTIP`); these are those strings
/// character for character.
public enum VerdictCopy {
    public static let question = "Did this session do what you asked?"
    public static let worked = "Worked"
    public static let partly = "Partly"
    public static let failed = "Failed"

    /// Load-bearing, not decoration. The spec exempts the outcome fields
    /// from the sheet's "exactly what would be sent" guarantee, and this
    /// sentence is the only place that exemption is disclosed to the
    /// contributor. Do not reword, shorten, or drop it.
    public static let caption =
        "Optional. This is recorded as the trace outcome; the preview above does not show it."

    /// The bulk verdict control beside `Submit all`. The plain button stays
    /// a one-click unanswered submit; this is the opt-in path for answering
    /// once for the whole group.
    public static let submitAllAs = "Submit all as..."
    public static let submitAllAsTooltip =
        "Record the same outcome for every session in this group."
}
