using System;
using System.Globalization;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace TraceCommons.Interop;

/// <summary>
/// The <c>approve</c> payload, and the arithmetic behind the undo bar.
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
/// </summary>
public sealed class ApprovalHold
{
    [JsonPropertyName("approved")]
    public int Approved { get; set; }

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
}
