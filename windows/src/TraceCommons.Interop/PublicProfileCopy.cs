using System;
using System.Globalization;
using System.Text;

namespace TraceCommons.Interop;

/// <summary>
/// Every sentence this app says about claiming, editing and withdrawing a
/// public handle, and the one decision behind them.
///
/// This lives in the interop assembly rather than in a view model for the
/// same reason <see cref="WithdrawCopy"/> does: what may honestly be said
/// about an outward-facing consent act is a safety property of the shell, not
/// a presentation detail, and here it is exercised by tests on a machine that
/// cannot build WinUI at all.
///
/// <para><b>Where this copy comes from.</b> The shared design spec
/// (<c>docs/superpowers/specs/2026-08-08-contributor-shell-shared-design.md</c>)
/// specifies the consent-scope checkbox and nothing else about this surface:
/// it draws no handle-claiming screen and writes no sentence about what a
/// claim did. The Linux shell's
/// <c>crates/trace-commons-contributor-gtk/src/copy.rs</c> is therefore the
/// source of truth, and these constants mirror it word for word -- including
/// its <c>--</c> spelling of the dash, which is what this assembly already
/// uses. The macOS mirror is
/// <c>macos/Sources/TraceCommonsApp/Views/PublicProfileCopy.swift</c>. Two
/// shells that word an outward-facing consent action differently are two
/// different promises about what becomes public, so a change to any one of
/// the three belongs in all three.</para>
///
/// <para><b>Nothing here is ever logged.</b> A handle and a bio are public by
/// construction -- that is what claiming them means -- but they are still
/// contributor identity, and the repo's hash-only discipline keeps them off
/// every log line and audit row regardless. The daemon's error labels, by
/// contrast, are fixed strings and carry nothing: the daemon deliberately
/// does not forward the underlying failure, because it can hold a server
/// response body or a URL.</para>
/// </summary>
public static class PublicProfileCopy
{
    /// <summary>
    /// The bio budget, in UTF-8 bytes, as
    /// <c>trace_commons_protocol::community_handle</c> counts it.
    /// </summary>
    /// <remarks>
    /// Bytes rather than characters, and that distinction is the whole reason
    /// this is a named constant with a counter beside it: a bio of 200
    /// emoji is 800 bytes and would be refused by a server this window told
    /// was happy. The rule itself is NOT re-implemented here -- the daemon
    /// and the server share one copy of it -- so this counts and reports,
    /// and refuses nothing.
    /// </remarks>
    public const int BioByteLimit = 280;

    // --- The section (5.6) ------------------------------------------------

    public const string Heading = "Your public profile";

    public const string ListHandlePublicly = "List my handle publicly";

    public const string Footnote =
        "Attribution only -- being listed grants no data use at all. Leaving the roster removes "
        + "you from future snapshots.";

    public const string HandleLabel = "Handle";

    public const string BioLabel = "Bio -- 280 bytes, plaintext, no HTML";

    public const string SaveProfile = "Save profile";

    public const string LeaveRoster = "Leave the roster";

    /// <summary>
    /// The date is the daemon's, formatted at the call site; only the
    /// sentence around it lives here.
    /// </summary>
    public static string OnRosterSince(string date) => $"On the roster since {date}";

    // --- The go-public dialog (5.7) ---------------------------------------

    public const string GoPublicTitle = "Go public?";

    public const string GoPublicHeadline = "Put your handle on the public roster?";

    public const string PublishedHeading = "What gets published";

    public const string PublishedBody =
        "Your handle -- real handles only, no pseudonyms. Aggregate counts: accepted, novelty "
        + "credit, accept rate. The date you went public. Your bio, if you write one.";

    public const string NeverHeading = "What never does";

    public const string NeverBody =
        "Your traces or anything in them. Per-trace data of any kind. Anything about sessions "
        + "you didn't send.";

    public const string GoPublicAcknowledgement =
        "I understand my handle and aggregate counts become public. Leaving the roster removes "
        + "me from future snapshots.";

    public const string GoPublicConfirm = "Go public";

    public const string GoPublicFootnote =
        "Nothing is pre-checked, and Go public stays off until the acknowledgement is on. This "
        + "changes attribution only -- it grants no data use.";

    /// <summary>
    /// The handle field inside the dialog. The panel's <see cref="HandleLabel"/>
    /// names the same thing, so the same constant would do -- except that here
    /// the field is empty and has to say what to put in it, and "Handle" over
    /// an empty box does not.
    /// </summary>
    public const string GoPublicHandleLabel = "The handle to publish";

    /// <summary>
    /// The optional bio, said as optional. <see cref="BioLabel"/> carries the
    /// budget and the format; what it cannot carry is that leaving this empty
    /// is a complete answer rather than an unfinished form.
    /// </summary>
    public const string GoPublicBioLabel = "Bio, if you want one -- 280 bytes, plaintext, no HTML";

    /// <summary>
    /// The one way this product declines to do something now: "Not now",
    /// never "Cancel" and never "No".
    /// </summary>
    public const string NotNow = "Not now";

    // --- What a claim or a withdrawal actually did (5.6) ------------------
    //
    // Every sentence below states what is true of the PUBLIC surface first,
    // because that is the thing the contributor just changed and the thing
    // they cannot inspect from this window. What this device managed to write
    // down about it is a second, lesser fact and is worded as one.

    /// <summary>A claim the server accepted.</summary>
    public const string Published =
        "You're on the roster. Your handle and aggregate counts are public now.";

    /// <summary>
    /// A claim the server accepted and this device then failed to write down.
    /// </summary>
    /// <remarks>
    /// This is what <c>handle_persisted: false</c> means, and it is
    /// emphatically not a failed claim: the server has taken the handle by the
    /// time that flag exists at all, so the profile is public whatever
    /// happened on this machine afterwards. Telling a contributor their handle
    /// did not go up when it did is the one error this surface must never
    /// make -- it is a false statement about a public, outward-facing act, and
    /// they would walk away believing they are unlisted. So the sentence leads
    /// with the publication and describes the local loss for exactly what it
    /// is: this window will misreport the state until the next successful
    /// save, and nothing public changes either way.
    /// </remarks>
    public const string PublishedNotCached =
        "You're on the roster -- your handle and aggregate counts are public now. This device "
        + "couldn't keep its own copy of the profile, so this window will show you as unlisted "
        + "again until you save it once more. That doesn't change anything about what is public.";

    /// <summary>A withdrawal the server accepted.</summary>
    public const string LeftRoster =
        "You've left the roster. Your handle isn't published any more, and future snapshots "
        + "won't include you.";

    /// <summary>
    /// A withdrawal the server accepted and this device then failed to write
    /// down. The mirror of <see cref="PublishedNotCached"/>, and stated for
    /// the same reason: the row is gone from the server regardless, so the
    /// withdrawal is not in doubt -- only what this window will show next.
    /// </summary>
    public const string LeftRosterNotCached =
        "You've left the roster -- your handle isn't published any more, and future snapshots "
        + "won't include you. This device couldn't clear its own copy of the profile, so this "
        + "window may show the old handle again until it can.";

    /// <summary>
    /// What to say about a claim the server accepted.
    /// </summary>
    /// <remarks>
    /// <c>handle_persisted</c> is NOT whether the claim worked. The server has
    /// taken the handle by the time this flag exists at all; the flag reports
    /// whether the daemon managed to write its own local copy of it. So both
    /// branches report a published profile, and the false branch adds only the
    /// weaker thing that is actually true.
    /// </remarks>
    public static string PublishedSentence(bool handlePersisted) =>
        handlePersisted ? Published : PublishedNotCached;

    /// <summary>The mirror, for a withdrawal the server accepted.</summary>
    public static string LeftRosterSentence(bool handlePersisted) =>
        handlePersisted ? LeftRoster : LeftRosterNotCached;

    /// <summary>
    /// A claim the daemon or the server refused, from the daemon's fixed
    /// label.
    /// </summary>
    /// <remarks>
    /// Every branch says that nothing was published, because in every one of
    /// them nothing was: the refusal happens before or instead of the
    /// <c>PUT</c>. The validation rules themselves are not re-implemented
    /// here -- the daemon and the server share one copy of them in
    /// <c>community_handle</c>, and a second copy in this window is how a
    /// handle this shell accepts becomes a handle the server refuses. These
    /// sentences only translate the verdict.
    ///
    /// An unrecognised label falls to the generic sentence rather than being
    /// echoed. A label from a newer daemon is still a fixed string, but this
    /// is the one place a server-supplied message could reach a screen, and
    /// the rule is that it never does.
    /// </remarks>
    public static string FailureSentence(string? label)
    {
        string reason = label switch
        {
            "handle-required" => "There's no handle in the box yet.",
            "handle-too-short" => "That handle is too short -- it needs at least 3 characters.",
            "handle-too-long" => "That handle is too long -- 32 characters at most.",
            "handle-invalid-character" =>
                "A handle can only use letters, numbers, hyphens and underscores.",
            "handle-invalid-boundary" =>
                "A handle has to start and end with a letter or a number.",
            "handle-consecutive-separators" =>
                "A handle can't have two hyphens or underscores in a row.",
            "handle-reserved" => "That handle is reserved and can't be claimed.",
            "bio-too-long" => "That bio is over the 280-byte budget.",
            "bio-invalid-character" => "That bio has a character the roster doesn't take.",

            // Not reachable from this window -- it always sends a bio key,
            // null or a string -- and handled anyway, so a contract change
            // surfaces as a sentence rather than as the fallback below.
            "bio-required-or-null" or "bio-invalid" =>
                "The bio wasn't sent in a form the roster takes.",
            "not-logged-in" => "This device isn't connected to Trace Commons.",

            // The underlying failure is never forwarded by the daemon -- it
            // can carry a server response body or a URL -- so there is nothing
            // more specific to say than that it did not go through.
            _ => "The request didn't go through.",
        };

        return $"{reason} Nothing was published and nothing changed. You can try again.";
    }

    /// <summary>
    /// The same, for a withdrawal: "nothing was published" is the wrong second
    /// clause when what failed was an attempt to <em>un</em>-publish, and a
    /// contributor who read it could conclude they had been taken off the
    /// roster when they are still on it.
    /// </summary>
    public static string LeaveFailureSentence(string? label)
    {
        string reason = label == "not-logged-in"
            ? "This device isn't connected to Trace Commons."
            : "The request didn't go through.";

        return $"{reason} You're still on the roster and your handle is still published. You can "
            + "try again.";
    }

    /// <summary>
    /// "74/280", from what is actually in the box.
    /// </summary>
    /// <remarks>
    /// UTF-8 bytes, because that is the unit the budget is denominated in.
    /// Counting characters here would let this window report 200/280 for a
    /// bio the server refuses at 800 bytes, which is the counter lying about
    /// the only thing it exists to report.
    /// </remarks>
    public static string BioCounter(string? bio) => string.Format(
        CultureInfo.InvariantCulture,
        "{0}/{1}",
        bio is null ? 0 : Encoding.UTF8.GetByteCount(bio),
        BioByteLimit);
}
