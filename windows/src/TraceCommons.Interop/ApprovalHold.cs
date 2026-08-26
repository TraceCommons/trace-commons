using System;
using System.Collections.Generic;
using System.Globalization;
using System.Linq;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace TraceCommons.Interop;

/// <summary>
/// The <c>approve</c> payload -- one entry, a whole project, or an unrecognised
/// id refused as <c>bad_params</c> -- and the arithmetic behind the undo bar
/// and the submit toast.
///
/// Approving is followed by a real undo, backed by <c>cancel</c>, for as long
/// as the daemon's own hold lasts. It is trivially cheap and it converts a
/// misclick from permanent into a non-event, which is the whole argument for
/// it.
///
/// The countdown is the DAEMON'S deadline, not a timer this app invented. A
/// five-second bar drawn over a hold that already expired would offer an undo
/// that cannot work; one drawn over a longer hold would retire an undo that
/// still would.
///
/// An unrecognised <c>entry_id</c> or <c>project_id</c> is refused by the
/// daemon as <c>bad_params</c>, which arrives as an error frame: <see cref="Parse"/>
/// returns null for it exactly as it does for any other error, and a caller
/// must show that as a refusal rather than as a toast with zero counts --
/// there is no <see cref="SubmitToast"/> to render because there is no result
/// to read one from.
/// </summary>
public sealed class ApprovalHold
{
    [JsonPropertyName("approved")]
    public ulong Approved { get; set; }

    /// <summary>How many approved sessions carried a flagged (PII) label.</summary>
    [JsonPropertyName("flagged")]
    public ulong Flagged { get; set; }

    /// <summary>
    /// Redaction counts by category, summed across every entry this call
    /// built a preview for. The toast names only the total -- see
    /// <see cref="RedactionsTotal"/> -- never a category; the preview sheet is
    /// where a contributor sees which detector fired.
    /// </summary>
    [JsonPropertyName("redactions")]
    public Dictionary<string, uint> Redactions { get; set; } = new();

    /// <summary>
    /// Entries this call did not approve, and why, in response order. Neither
    /// the wire label nor the entry id is ever shown to a contributor
    /// directly -- <see cref="Toast"/> renders the human label instead.
    /// </summary>
    [JsonPropertyName("skipped")]
    public List<SubmitSkip> Skipped { get; set; } = new();

    [JsonPropertyName("hold_secs")]
    public long HoldSecs { get; set; }

    /// <summary>
    /// When the hold runs out, RFC 3339, or null.
    ///
    /// Null is a real answer and means no undo may be offered: nothing was
    /// approved, or the hold is configured off, and in either case the send
    /// cannot be recalled. The bar says so plainly rather than showing a
    /// button that would fail.
    /// </summary>
    [JsonPropertyName("hold_until")]
    public string? HoldUntil { get; set; }

    /// <summary>
    /// Parses an approve result. A payload that cannot be read yields null,
    /// which the caller treats exactly as it treats a null
    /// <see cref="HoldUntil"/>: the approval stands, no undo is offered.
    /// Inventing a deadline would be worse than admitting there is none.
    /// </summary>
    public static ApprovalHold? Parse(DaemonResponse response)
    {
        ArgumentNullException.ThrowIfNull(response);
        return response.ResultAs<ApprovalHold>();
    }

    /// <summary>The hold deadline, or null when there is no offerable undo.</summary>
    public DateTimeOffset? Deadline =>
        DateTimeOffset.TryParse(
            HoldUntil,
            CultureInfo.InvariantCulture,
            DateTimeStyles.RoundtripKind,
            out DateTimeOffset parsed)
            ? parsed
            : null;

    /// <summary>
    /// Whole seconds left at <paramref name="now"/>, floored at zero. Zero
    /// means the undo is gone -- including when the daemon sent no deadline
    /// at all.
    /// </summary>
    public int RemainingSeconds(DateTimeOffset now)
    {
        if (Deadline is not DateTimeOffset deadline)
        {
            return 0;
        }

        double seconds = Math.Ceiling((deadline - now).TotalSeconds);
        return seconds <= 0 ? 0 : seconds > int.MaxValue ? int.MaxValue : (int)seconds;
    }

    /// <summary>Whether an undo may still be offered at <paramref name="now"/>.</summary>
    public bool IsLive(DateTimeOffset now) => RemainingSeconds(now) > 0;

    /// <summary>
    /// Whether this response is the correction-credential refusal.
    ///
    /// Distinguished from every other skip because it is the only one the
    /// contributor caused and the only one they can fix, and because the
    /// advice that goes with it -- rotate the credential, it has already
    /// been typed -- is not advice any other refusal carries. A caller that
    /// sees this shows <see cref="CorrectionCopy.CredentialHeadline"/> and
    /// its body instead of the ordinary toast.
    /// </summary>
    public bool WasRefusedForACorrectionCredential =>
        Skipped.Any(skip => skip.ReasonLabel == CorrectionCopy.CredentialRefusalLabel);

    /// <summary>The sum of <see cref="Redactions"/>, which is all the toast ever names.</summary>
    public ulong RedactionsTotal => Redactions.Values.Aggregate(0UL, (total, count) => total + count);

    /// <summary>
    /// The one sentence a contributor sees after a one-click submit, and
    /// whether Undo goes with it. See <see cref="SubmitToast"/> for the
    /// wording contract; this is the only place that assembles its inputs
    /// from an actual daemon response.
    /// </summary>
    public SubmitToast Toast => SubmitToast.Render(
        Approved,
        RedactionsTotal,
        Flagged,
        Skipped.Select(skip => skip.ReasonLabel).ToList());

    /// <summary>
    /// Which of <paramref name="candidateEntryIds"/> this call actually
    /// approved -- the ones not named in <see cref="Skipped"/>.
    ///
    /// The response never lists approved ids directly (only a count), because
    /// a batch approval does not otherwise need them. A project-group submit
    /// does: recalling it through the per-entry <c>cancel</c> method needs to
    /// know exactly which entries went out, and the only way to learn that is
    /// to start from the ids offered and subtract the ones this response says
    /// were skipped.
    /// </summary>
    public IReadOnlyList<string> ApprovedEntryIds(IEnumerable<string> candidateEntryIds)
    {
        ArgumentNullException.ThrowIfNull(candidateEntryIds);
        var skippedIds = new HashSet<string>(
            Skipped.Select(skip => skip.EntryId),
            StringComparer.Ordinal);
        return candidateEntryIds.Where(id => !skippedIds.Contains(id)).ToList();
    }
}

/// <summary>
/// One entry the daemon did not approve, and the wire reason it gave.
///
/// Neither field is shown to a contributor directly: <see cref="ApprovalHold.Toast"/>
/// translates <see cref="ReasonLabel"/> through <see cref="SubmitToast.ReasonLabel"/>
/// and never surfaces <see cref="EntryId"/> at all -- an id in a toast is
/// noise a contributor cannot act on.
/// </summary>
public sealed class SubmitSkip
{
    [JsonPropertyName("entry_id")]
    public string EntryId { get; set; } = string.Empty;

    [JsonPropertyName("reason_label")]
    public string ReasonLabel { get; set; } = string.Empty;
}
