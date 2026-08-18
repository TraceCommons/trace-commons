using System;

namespace TraceCommons.Interop;

/// <summary>
/// The history screen's words, in one place.
///
/// Transcribed from the Linux shell's <c>copy.rs</c> so the two shells say
/// the same thing: a contributor who reads "Held for privacy review" on one
/// machine and "Under review" on another has been told two different things
/// about one state. Four rules bind everything here.
///
/// <list type="bullet">
/// <item><b>Credit is a record, never a currency.</b> No currency symbol, no
/// fiat estimate, no projection, no date, no gamification.</item>
/// <item><b>Quarantine is held, never rejected</b>, and never carries a
/// turnaround time.</item>
/// <item><b>Never name the mechanism.</b> "Privacy filter", "claim",
/// "ingest", "canary" are internal words.</item>
/// <item><b>Always state the data consequence.</b></item>
/// </list>
/// </summary>
public static class HistoryCopy
{
    /// <summary>
    /// The statuses the server reports on submission-status read-back, plus
    /// the one the daemon's cache stamps locally.
    /// </summary>
    /// <remarks>
    /// <c>withdrawn</c> is never returned by the server: only
    /// <c>daemon::history::mark_withdrawn</c> writes it, once a withdrawal
    /// has been confirmed. Matched as ordinal strings rather than parsed into
    /// an enum, because a status this build has never heard of must degrade to
    /// "waiting to be scored" rather than fail.
    /// </remarks>
    public const string StatusSubmitted = "submitted";

    public const string StatusAccepted = "accepted";

    public const string StatusQuarantined = "quarantined";

    public const string StatusWithdrawn = "withdrawn";

    public const string ScreenTitle = "History";

    /// <summary>The screen's subtitle: what this list is.</summary>
    public const string ScreenSubtitle = "Everything this account has sent, and what became of it.";

    public const string InTheCommons = "In the commons";

    public const string WaitingToBeScored = "Waiting to be scored";

    /// <summary>The section heading over the record rows.</summary>
    public const string EverythingContributed = "Everything you've contributed";

    /// <summary>
    /// The chip on a withdrawn record. The record stays on the list and reads
    /// as withdrawn; it is never dropped and never re-labelled as something
    /// that failed.
    /// </summary>
    public const string WithdrawnByYou = "Withdrawn by you";

    public const string QuarantineHeading = "Held for privacy review";

    public const string QuarantineBody =
        "A person at Trace Commons reads these before they enter the commons. It happens when "
        + "automated checks see something that might be personal or sensitive and can't decide "
        + "on its own.\n\nThese have not been rejected, and they have not been shared with "
        + "anyone but the reviewer. They are sitting still.\n\nTypical wait: we don't have a "
        + "reliable number yet.";

    /// <summary>
    /// The row-level explanation on a held record, used only when the server
    /// sent no explanation of its own. It says the same three things
    /// <see cref="QuarantineBody"/> says -- automated, not rejected, not
    /// shared -- at row length rather than at section length.
    /// </summary>
    public const string HeldRowBody =
        "Automated checks saw something that might be personal and couldn't decide on their "
        + "own. It has not been rejected, and it has not been shared with anyone but the "
        + "reviewer.";

    public const string CreditSection = "Credit";

    public const string CreditBody =
        "Contributions earn credit points, scored on how novel and information-rich a trace is. "
        + "Today credit is a record, not a currency: there is no payout, no token, no exchange "
        + "rate, and no date. The intent is that credit eventually settles to something real, "
        + "and if it does it will settle from this record. Contribute because you want the "
        + "commons to exist.";

    /// <summary>Eyebrow over the settled figure.</summary>
    public const string CreditRecorded = "Recorded";

    /// <summary>Eyebrow over the figure that is still moving.</summary>
    public const string CreditStillBeingScored = "Still being scored";

    /// <summary>
    /// What a <c>last_refreshed_at</c> of null renders as. Show staleness
    /// rather than presenting a stale cache as current: a confident zero
    /// beside a figure people care about is worse than no figure.
    /// </summary>
    public const string NotSyncedYet = "Not synced yet";

    /// <summary>The button behind <c>refresh_history</c>.</summary>
    public const string CheckForUpdates = "Check for updates";

    /// <summary>
    /// What <c>refresh_history</c> actually achieved, said accurately.
    /// </summary>
    /// <remarks>
    /// The daemon answers <c>requested: true</c> and nothing else: the
    /// background poller owns the network call, and this only asks it to run
    /// sooner. So this sentence says the ask landed, never that anything was
    /// fetched -- "Updated" would be a claim about a round trip that has not
    /// happened yet.
    /// </remarks>
    public const string CheckForUpdatesAsked =
        "Asked for an update. New results appear here as they arrive.";

    /// <summary>Said when the daemon could not serve the cache at all.</summary>
    public const string HistoryUnavailable =
        "History could not be read just now. Nothing has been lost -- this is what is on this "
        + "machine, not what is on the server.";

    /// <summary>The empty state. Not a failure, and it does not read as one.</summary>
    public const string NothingYet = "Nothing has been sent yet";

    public const string NothingYetBody =
        "Once you approve a session it appears here, with what became of it and what it earned.";

    /// <summary>
    /// The heading over <c>queue_outcome_counts</c>.
    /// </summary>
    /// <remarks>
    /// Deliberately not "why nothing is pending". This method counts entries
    /// that reached the queue and did not go out; it cannot explain a session
    /// the watcher discarded before a queue entry ever existed, and the
    /// contract says in as many words not to present it as though it can.
    /// </remarks>
    public const string OutcomesHeading = "Sessions that were offered and did not go out";

    public const string OutcomesFootnote =
        "Counted from entries that reached your queue. A session the watcher never offered is "
        + "not counted here.";

    /// <summary>
    /// The four states a record can be in, in the same words the stat cards
    /// and the chips use, so a badge on a record and a card at the top of the
    /// screen cannot say different things about one state.
    /// </summary>
    public static string StatusWord(string? status) => status switch
    {
        StatusAccepted => InTheCommons,
        StatusQuarantined => QuarantineHeading,
        StatusWithdrawn => WithdrawnByYou,
        _ => WaitingToBeScored,
    };

    /// <summary>
    /// The record's own figures, or null when there is no figure to state.
    /// </summary>
    /// <remarks>
    /// A withdrawn record keeps whatever it recorded: withdrawal does not
    /// reverse settled credit, and implying it did would be a lie about what
    /// the action achieved. Recorded credit is final; anything still being
    /// scored is stated as such rather than being added to it, and neither
    /// carries a symbol.
    /// </remarks>
    public static string? CreditLine(float? creditFinal, float pending)
    {
        if (creditFinal is { } settled)
        {
            return string.Format(
                System.Globalization.CultureInfo.CurrentCulture,
                "credit {0:0.0}",
                settled);
        }

        return pending > 0f
            ? string.Format(
                System.Globalization.CultureInfo.CurrentCulture,
                "credit {0:0.0}, still being scored",
                pending)
            : null;
    }
}
