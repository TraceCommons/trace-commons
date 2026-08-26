namespace TraceCommons.Interop;

/// <summary>
/// What the correction control says. Word for word what the Linux and macOS
/// shells print -- <c>crates/trace-commons-contributor-gtk/src/copy.rs</c> is
/// where these originate, and a test on the Rust side reads this file to
/// prove the three have not drifted.
/// </summary>
public static class CorrectionCopy
{
    /// <summary>
    /// The field's prompt. Shown only under "Partly" and "Failed": a run the
    /// contributor has just called successful has nothing to correct, and
    /// that gate is a guard as much as it is semantics -- it halves the
    /// surface for correction-shaped credit farming.
    /// </summary>
    public const string Question = "What did it get wrong?";

    /// <summary>
    /// The placeholder inside the box, so the caption below can spend all of
    /// its words on the thing that actually matters.
    /// </summary>
    public const string Placeholder = "Optional";

    /// <summary>
    /// <b>The disclosure, and the most load-bearing string in this file.</b>
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
    /// </summary>
    public const string Caption =
        "Stored exactly as you write it. Unlike the rest of the trace, a correction is not scrubbed here or on the server -- so leave out anything you would not want in the corpus: someone else's personal information, employer-confidential material, or anything you are not free to share.";

    /// <summary>
    /// The credential refusal, headline and body.
    ///
    /// Its own message rather than a line in the generic submit toast: it is
    /// the only submit failure the contributor caused and the only one they
    /// can fix, and the second half is advice they will not get anywhere
    /// else. A credential typed into a box has been typed; taking it out of
    /// the text does not un-type it, so the sentence says to rotate it.
    ///
    /// Neither string quotes the correction, and neither names what matched.
    /// </summary>
    public const string CredentialHeadline =
        "Nothing was sent. Your correction looks like it contains a credential.";

    public const string CredentialBody =
        "A correction is stored as you write it, so this one was refused rather than masked. Take the credential out and submit again -- and rotate it, because it has already been typed here.";

    /// <summary>
    /// The daemon's fixed label for that refusal. Matched, never rendered:
    /// what a contributor reads is <see cref="CredentialHeadline"/> and
    /// <see cref="CredentialBody"/>. The wire spelling of
    /// <c>envelope::REASON_CORRECTION_CREDENTIAL</c> in the contributor
    /// crate.
    /// </summary>
    public const string CredentialRefusalLabel = "correction-credential-detected";

    /// <summary>
    /// The longest correction the daemon accepts, in characters
    /// (<c>envelope::MAX_CORRECTION_CHARS</c>). Enforced at the keyboard so
    /// the refusal happens where the person can see it and shorten what they
    /// wrote, rather than as a <c>correction-too-long</c> after the fact.
    /// </summary>
    public const int MaxCharacters = 2000;
}
