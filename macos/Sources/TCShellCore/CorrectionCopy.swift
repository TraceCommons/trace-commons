import Foundation

/// Copy for the correction control, shared with the other two shells.
///
/// It lives in `TCShellCore` for the reason `VerdictCopy` and `ReadGate` do:
/// the app target links the FFI dylib and is awkward to assert against, and
/// this text is written three times across three shells. The Linux original
/// is `crates/trace-commons-contributor-gtk/src/copy.rs`
/// (`CORRECTION_QUESTION`, `CORRECTION_PLACEHOLDER`, `CORRECTION_CAPTION`,
/// `CORRECTION_CREDENTIAL_HEADLINE`, `CORRECTION_CREDENTIAL_BODY`); these
/// are those strings character for character, and a Rust-side test reads
/// this file to prove it.
public enum CorrectionCopy {
    /// The field's prompt. Shown only under `Partly` and `Failed` -- a run
    /// the contributor has just called successful has nothing to correct.
    public static let question = "What did it get wrong?"

    /// The placeholder inside the box, so the caption below can spend all of
    /// its words on the thing that actually matters.
    public static let placeholder = "Optional"

    /// **The disclosure, and the most load-bearing string in this module.**
    ///
    /// Everything else a contributor writes or captures is scrubbed on this
    /// machine and scrubbed again on the server. A correction is the one
    /// exception: redaction would destroy the thing it exists to carry, so
    /// it is stored exactly as typed, with only credential detection
    /// standing between it and the corpus.
    ///
    /// The published policy page promises local redaction and a server-side
    /// re-application of it, and does not yet carve this out. Until that
    /// clause is published, this sentence is the ONLY disclosure a
    /// contributor gets that their own words are stored verbatim. Do not
    /// shorten it for layout; change the layout.
    public static let caption =
        "Stored exactly as you write it. Unlike the rest of the trace, a correction is not scrubbed here or on the server -- so leave out anything you would not want in the corpus: someone else's personal information, employer-confidential material, or anything you are not free to share."

    /// The credential refusal, headline and body.
    ///
    /// Its own message rather than a line in the generic submit toast: it is
    /// the only submit failure the contributor caused and the only one they
    /// can fix, and the second half is advice they will not get anywhere
    /// else. A credential typed into a box has been typed; taking it out of
    /// the text does not un-type it, so the sentence says to rotate it.
    ///
    /// Neither string quotes the correction, and neither names what matched.
    public static let credentialHeadline =
        "Nothing was sent. Your correction looks like it contains a credential."

    public static let credentialBody =
        "A correction is stored as you write it, so this one was refused rather than masked. Take the credential out and submit again -- and rotate it, because it has already been typed here."

    /// The daemon's fixed label for that refusal.
    ///
    /// Matched, never rendered: what a contributor reads is
    /// `credentialHeadline` and `credentialBody`. The wire spelling of
    /// `envelope::REASON_CORRECTION_CREDENTIAL` in the contributor crate.
    public static let credentialRefusalLabel = "correction-credential-detected"

    /// The longest correction the daemon accepts, in characters
    /// (`envelope::MAX_CORRECTION_CHARS`). Enforced at the keyboard so the
    /// refusal happens where the person can see it and shorten what they
    /// wrote, rather than as a `correction-too-long` after the fact.
    public static let maxCharacters = 2000

    /// What is actually sent for a box holding `text`: the trimmed text, or
    /// `nil` when there is nothing in it.
    ///
    /// An empty string is NOT the absence of a correction. Sending one would
    /// declare `correction_included` on the envelope for content that is not
    /// there, which is exactly the declaration/payload disagreement the
    /// consent flags exist to prevent. So blank means no key at all.
    public static func toSend(_ text: String) -> String? {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : trimmed
    }
}
