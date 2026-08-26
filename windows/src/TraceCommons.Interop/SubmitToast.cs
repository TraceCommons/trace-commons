using System;
using System.Collections.Generic;
using System.Globalization;
using System.Linq;

namespace TraceCommons.Interop;

/// <summary>
/// The submit toast: the daemon's counts, said in one sentence.
///
/// One-click submit sends a session nobody previewed, so the toast is the
/// only place a contributor learns what happened -- what was approved, what
/// scrubbing did to it, what was held, and what was not approved. That makes
/// the wording a contract rather than a presentation detail, and it is fixed
/// in <c>docs/superpowers/specs/2026-08-20-one-click-submit-design.md</c>
/// under "The toast: normative copy". The strings below are transcribed from
/// that section, not paraphrased from it.
///
/// Three shells render this sentence and they must render it identically.
/// The Linux copy is <c>crates/trace-commons-contributor-gtk/src/toast.rs</c>
/// (with its clause strings in <c>copy.rs</c>), the macOS copy is
/// <c>macos/Sources/TCShellCore/SubmitToast.swift</c>, and all three assert
/// the spec's four worked examples for exactly one reason: a sentence
/// reworded in one client is the drift that section exists to prevent.
///
/// This lives in the interop assembly rather than in a view model for the
/// same reason <see cref="WithdrawCopy"/> does -- it is a property of the
/// shell rather than a presentation detail, and here it is exercised by
/// tests on a machine that cannot build WinUI at all. Deliberately pure:
/// counts in, a string and a bool out, no I/O and no UI types.
/// </summary>
public sealed class SubmitToast
{
    private SubmitToast(string line, bool offerUndo)
    {
        Line = line;
        OfferUndo = offerUndo;
    }

    /// <summary>The whole sentence, ready to display.</summary>
    public string Line { get; }

    /// <summary>
    /// Whether to offer Undo alongside it.
    ///
    /// True only when something was actually approved. Offering Undo on any
    /// successful response was correct while every approval succeeded and is
    /// wrong now that entries can be skipped: a skipped entry with an undo
    /// timer behind it reads as approved.
    /// </summary>
    public bool OfferUndo { get; }

    /// <summary>
    /// Render the toast from an <c>approve</c> response.
    /// </summary>
    /// <param name="approved">How many sessions were approved.</param>
    /// <param name="redactions">
    /// The sum of the response's <c>redactions</c> map. The toast names a
    /// count, never a category: the preview sheet is where a contributor
    /// sees which detector fired.
    /// </param>
    /// <param name="flagged">How many were held.</param>
    /// <param name="skipped">
    /// The response's wire reason labels, in response order. The rendered
    /// sentence uses the human labels, distinct, in the spec table's order,
    /// so neither a wire label nor an entry id ever reaches a contributor.
    /// </param>
    public static SubmitToast Render(
        ulong approved,
        ulong redactions,
        ulong flagged,
        IReadOnlyList<string> skipped)
    {
        ArgumentNullException.ThrowIfNull(skipped);

        var clauses = new List<string> { ApprovedClause(approved), ScrubClause(redactions) };

        var joined = FlaggedAndSkippedClause(flagged, skipped);
        if (joined is not null)
        {
            clauses.Add(joined);
        }

        return new SubmitToast(string.Join(" ", clauses), approved > 0);
    }

    // --- The clauses ---------------------------------------------------

    /// <summary>
    /// Clause 1: what happened.
    ///
    /// Corrected 2026-08-20: this used to say "Sent", and that was false.
    /// At toast time nothing has left the machine -- the approval is
    /// recorded and the watcher sends it on its next sweep. A toast reading
    /// "Sent." while still offering Undo contradicted itself.
    /// </summary>
    internal static string ApprovedClause(ulong approved) => approved switch
    {
        0 => "Nothing approved.",
        1 => "Approved.",
        _ => string.Format(
            CultureInfo.InvariantCulture, "Approved {0}.", approved),
    };

    /// <summary>
    /// Clause 2: what scrubbing did.
    ///
    /// Always present, including when it did nothing. A count of zero is a
    /// fact the contributor is owed, not an absence to omit -- and it is the
    /// case worth weighing, which is why it is never silently dropped.
    /// </summary>
    internal static string ScrubClause(ulong totalRedactions) => totalRedactions switch
    {
        0 => "Scrubbing matched nothing.",
        _ => string.Format(
            CultureInfo.InvariantCulture, "Scrubbing removed {0}.", totalRedactions),
    };

    /// <summary>
    /// Clauses 3 and 4: what was flagged, and what was not approved.
    ///
    /// Corrected 2026-08-20: these used to be two independent clauses. The
    /// spec now joins them into one, comma-separated, with each half
    /// present only when non-zero -- <c>null</c> when both are zero, i.e.
    /// the whole clause is absent.
    ///
    /// The skip count is entries; the reason list is distinct reasons.
    /// Those are different numbers whenever several entries were skipped
    /// for the same reason, and the sentence says both because a
    /// contributor needs the first to know how much is still queued and the
    /// second to know what to do about it.
    ///
    /// <see cref="SkipReasonUnknown"/> now happens to share text with one of
    /// the table's own labels (<c>not-pinned</c>), so the reason list is
    /// deduplicated by its rendered text rather than assembled
    /// positionally -- otherwise a batch mixing <c>not-pinned</c> and an
    /// unrecognised label would print "could not be prepared" twice.
    /// </summary>
    internal static string? FlaggedAndSkippedClause(ulong flagged, IReadOnlyList<string> skipped)
    {
        string? flaggedHalf = flagged > 0
            ? string.Format(CultureInfo.InvariantCulture, "{0} flagged", flagged)
            : null;

        string? skippedHalf = null;
        if (skipped.Count > 0)
        {
            var reasons = new List<string>();
            foreach (var (_, human) in SkipReasons)
            {
                if (!reasons.Contains(human) && skipped.Any(wire => ReasonLabel(wire) == human))
                {
                    reasons.Add(human);
                }
            }

            if (!reasons.Contains(SkipReasonUnknown)
                && skipped.Any(wire => ReasonLabel(wire) == SkipReasonUnknown))
            {
                reasons.Add(SkipReasonUnknown);
            }

            skippedHalf = string.Format(
                CultureInfo.InvariantCulture,
                "{0} not approved: {1}",
                skipped.Count,
                string.Join(", ", reasons));
        }

        return (flaggedHalf, skippedHalf) switch
        {
            (null, null) => null,
            (string f, null) => f + ".",
            (null, string s) => s + ".",
            (string f, string s) => f + ", " + s + ".",
        };
    }

    // --- The reason table ----------------------------------------------

    /// <summary>
    /// The human label for each wire reason an entry can be skipped for, in
    /// the spec table's order -- which is also the order they are listed in
    /// when several apply.
    ///
    /// The wire spellings are the daemon's and belong to the protocol; the
    /// human halves belong to the contributor. Nothing here ever shows the
    /// left-hand column, and nothing here shows an entry id: an id in a
    /// toast is noise a contributor cannot act on.
    ///
    /// A list of pairs rather than a dictionary, because the order is part
    /// of the contract and a dictionary has none.
    /// </summary>
    public static IReadOnlyList<(string Wire, string Human)> SkipReasons { get; } =
        new[]
        {
            ("not-enrolled", "not connected to a commons"),
            ("not-pending", "already decided"),
            ("not-pinned", "could not be prepared"),
            ("envelope-too-large", "too large to send"),
            ("session-file-vanished", "the session file is gone"),
            ("preview-failed", "could not be read"),
            // Listed so a toast can never render this one as "could not be
            // prepared", which would be a false account of a refusal the
            // contributor caused and can fix. The sheet says the rest --
            // see CorrectionCopy.CredentialHeadline -- and this is what any
            // other surface reporting the same batch says.
            ("correction-credential-detected", "your correction contains a credential"),
        };

    /// <summary>
    /// What an unrecognised wire label is called instead of itself.
    ///
    /// Corrected 2026-08-20: this used to read "could not be sent"; that
    /// word belonged to the old, now-false "Sent" clause 1. Fixed to "could
    /// not be prepared", which happens to coincide with <c>not-pinned</c>'s
    /// own label -- both are, honestly, the least specific true statement
    /// available.
    ///
    /// The spec's table is closed today, but a daemon newer than the shell
    /// can send a label this build has never been taught, and the one thing
    /// that must not then happen is the shell echoing protocol vocabulary at
    /// a contributor. So an unknown label degrades to the least specific
    /// true statement available, and is listed last.
    /// </summary>
    public const string SkipReasonUnknown = "could not be prepared";

    /// <summary>
    /// Translate one wire reason label. Never returns its argument.
    /// </summary>
    public static string ReasonLabel(string wire)
    {
        foreach (var (candidate, human) in SkipReasons)
        {
            if (candidate == wire)
            {
                return human;
            }
        }

        return SkipReasonUnknown;
    }
}
