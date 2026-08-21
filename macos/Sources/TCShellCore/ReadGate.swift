import Foundation

/// What the preview sheet says, and what it requires, at the moment of
/// consent.
///
/// ## What this used to be
///
/// `Contribute` used to wait on three things: a loaded preview, the
/// "Exactly what would be sent" tab having been on screen, and an
/// acknowledgement checkbox ticked by hand. Two of them are gone.
///
/// The checkbox was removed as friction. The transcript-tab condition went
/// with it, for a reason worth writing down: a queue row's `Submit`
/// approves the same session without opening the preview at all, so the
/// gate never stood between anybody and a blind approval. The only person
/// it ever charged was the one who chose to look, which is the opposite of
/// what it was for.
///
/// ## What did not go
///
/// The claim. `statement` carries both halves of what the checkbox made a
/// contributor assert -- scrubbing is pattern-based and may have missed
/// something, and nothing here can tell whether anyone read anything -- and
/// the sheet prints it above `Contribute` where the tick used to be asked
/// for. Dropping the friction is a product decision; dropping the sentence
/// would be the app quietly claiming less about redaction than it knows.
///
/// And the pin: an approval still has to cover a preview that actually
/// loaded. That is not friction, it is the thing the approval binds to.
///
/// ## Why it lives in TCShellCore
///
/// The same reason `SubmitToast` does. The app target links the FFI dylib,
/// so nothing in it is reachable from `swift test`; a rule and a sentence
/// that three shells have to agree on need somewhere they can actually be
/// asserted. The Linux shell holds the sentence in
/// `crates/trace-commons-contributor-gtk/src/copy.rs` and the Windows shell
/// in `windows/src/TraceCommons.Interop/ReadGate.cs`; the Rust test
/// `the_three_shells_print_the_same_statement` reads this file to make sure
/// all three still say it character for character.
public enum ReadGate {
    /// The sentence that replaced the acknowledgement checkbox.
    ///
    /// One line, one escaped literal, on purpose: the Rust parity test
    /// scans this file for this exact text, and a line break here would
    /// defeat it.
    public static let statement = "\"Exactly what would be sent\" is the exact text that would leave this machine. Pattern-based scrubbing may have missed something in it, and nothing here checks that you looked."

    /// Tooltip on an armed `Contribute`. Identical on Windows.
    public static let readyHelp = "Sends this session. Nothing else."

    /// Tooltip on a `Contribute` that has nothing to bind to yet. The sheet
    /// is either still working out what would be sent or has failed to, and
    /// either way there is no pinned envelope for an approval to cover.
    public static let notPinnedHelp =
        "This preview hasn't loaded yet, so there is nothing here to contribute."

    /// The one question the sheet asks.
    ///
    /// A single condition, deliberately. It is stated as a function rather
    /// than inlined into the view so the rule has somewhere to be tested
    /// with values; the view has no testable seam at all.
    public static func canContribute(hasPinnedPreview: Bool) -> Bool {
        hasPinnedPreview
    }

    /// The tooltip that explains the current answer.
    public static func help(hasPinnedPreview: Bool) -> String {
        canContribute(hasPinnedPreview: hasPinnedPreview) ? readyHelp : notPinnedHelp
    }
}
