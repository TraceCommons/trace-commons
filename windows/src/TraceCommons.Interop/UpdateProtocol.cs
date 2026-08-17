using System;
using System.Text.Json.Serialization;

namespace TraceCommons.Interop;

/// <summary>What the daemon said when asked to quiesce.</summary>
public enum TcQuiesceOutcome
{
    /// <summary>Nothing is in flight and the queue is parked. Safe to update.</summary>
    Quiesced,

    /// <summary>The daemon refused for a reason other than a timeout.</summary>
    Busy,

    /// <summary>An upload was still in flight when the timeout expired.</summary>
    TimedOut,

    /// <summary>This daemon does not implement quiesce.</summary>
    Unsupported,

    /// <summary>No daemon answered, or the answer made no sense.</summary>
    Unavailable,
}

/// <summary>
/// The daemon's answer to <c>quiesce</c>, reduced to the one question the
/// caller has: may an update proceed.
/// </summary>
public sealed class QuiesceOutcome
{
    internal QuiesceOutcome(TcQuiesceOutcome outcome, long waitedMs)
    {
        Outcome = outcome;
        WaitedMs = waitedMs;
    }

    public TcQuiesceOutcome Outcome { get; }

    /// <summary>How long the daemon waited for the drain, in milliseconds.</summary>
    public long WaitedMs { get; }

    /// <summary>
    /// True for exactly one outcome. Written as an explicit equality rather
    /// than "not a failure" so that a future enum member defaults to
    /// refusing: an update is not something to fall into.
    /// </summary>
    public bool CanUpdate => Outcome == TcQuiesceOutcome.Quiesced;
}

/// <summary>
/// Reading the daemon's quiesce answer, and turning a refusal into something
/// a contributor can act on.
///
/// Deliberately pure: no native call, no WinRT, no platform. It lives in the
/// interop assembly so its tests run on macOS and Linux with the rest.
/// </summary>
public static class UpdateProtocol
{
    private sealed class QuiescePayload
    {
        [JsonPropertyName("quiesced")]
        public bool Quiesced { get; set; }

        [JsonPropertyName("waited_ms")]
        public long WaitedMs { get; set; }
    }

    /// <summary>
    /// Maps a raw <c>quiesce</c> response onto <see cref="QuiesceOutcome"/>.
    ///
    /// Every shape that is not an explicit <c>quiesced: true</c> maps to a
    /// refusal. That includes a well-formed result frame saying
    /// <c>quiesced: false</c>, which the daemon is not expected to send --
    /// but "unexpected" and "safe to update through" are different claims,
    /// and only one of them is ours to make.
    /// </summary>
    public static QuiesceOutcome ReadQuiesce(DaemonResponse response)
    {
        ArgumentNullException.ThrowIfNull(response);

        if (response.IsError)
        {
            DaemonError error = response.Error!;
            TcQuiesceOutcome outcome = error.Code switch
            {
                "unknown_method" => TcQuiesceOutcome.Unsupported,
                "bad_params" when error.Message == "quiesce-requires-async"
                    => TcQuiesceOutcome.Unsupported,
                "busy" when error.Message == "quiesce-timeout"
                    => TcQuiesceOutcome.TimedOut,
                "busy" => TcQuiesceOutcome.Busy,
                _ => TcQuiesceOutcome.Unavailable,
            };

            return new QuiesceOutcome(outcome, 0);
        }

        QuiescePayload? payload = response.ResultAs<QuiescePayload>();
        if (payload is null || !payload.Quiesced)
        {
            return new QuiesceOutcome(TcQuiesceOutcome.Unavailable, 0);
        }

        return new QuiesceOutcome(TcQuiesceOutcome.Quiesced, payload.WaitedMs);
    }

    /// <summary>
    /// One sentence per refusal, for the update banner.
    ///
    /// Fixed strings with no interpolation of anything the daemon said: this
    /// text goes on screen and into whatever the contributor screenshots, and
    /// the repo's rule is that such surfaces carry labels, never payload.
    /// </summary>
    public static string DescribeRefusal(TcQuiesceOutcome outcome) => outcome switch
    {
        TcQuiesceOutcome.Quiesced =>
            "Ready to update.",
        TcQuiesceOutcome.TimedOut =>
            "An upload is still finishing. The update will install the next time you open the app.",
        TcQuiesceOutcome.Busy =>
            "The daemon is busy. The update will install the next time you open the app.",
        TcQuiesceOutcome.Unsupported =>
            "This version cannot pause uploads to update safely. Windows will install the update the next time you open the app.",
        _ =>
            "The daemon is not available. The update will install the next time you open the app.",
    };
}
