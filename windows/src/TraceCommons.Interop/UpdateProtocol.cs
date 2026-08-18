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
/// Whether an update is waiting, mirroring WinRT's
/// <c>Windows.ApplicationModel.PackageUpdateAvailability</c> member for
/// member and value for value.
///
/// A separate type because this assembly targets plain net8.0 and must never
/// name a Windows type -- that is what keeps its tests running on macOS and
/// Linux against the same Rust crate.
/// </summary>
public enum TcUpdateAvailability
{
    /// <summary>The package has no App Installer association.</summary>
    Unknown = 0,

    /// <summary>Up to date.</summary>
    NoUpdates = 1,

    /// <summary>An update is waiting and is optional.</summary>
    Available = 2,

    /// <summary>An update is waiting and the feed marks it required.</summary>
    Required = 3,

    /// <summary>The check itself failed.</summary>
    Error = 4,
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
            // Error codes here must match crates/trace-commons-contributor/
            // src/daemon/ipc.rs, the source of truth for this wire contract.
            // The sync-dispatch refusal in particular is
            // Response::err(req.id, ERR_UNAVAILABLE, "quiesce-requires-async")
            // (ipc.rs:892, asserted by a Rust test at ipc.rs:3158-3163) --
            // code "unavailable", not "bad_params". tc_call always reaches
            // the async dispatcher, so this shape should never arrive here,
            // but the mapping is written to match what the daemon actually
            // sends rather than an invented shape.
            TcQuiesceOutcome outcome = error.Code switch
            {
                "unknown_method" => TcQuiesceOutcome.Unsupported,
                "unavailable" when error.Message == "quiesce-requires-async"
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

    /// <summary>
    /// Whether to put the update banner on screen.
    ///
    /// An allow-list of two, not a deny-list. <c>Unknown</c> and
    /// <c>Error</c> both mean the check told us nothing, and offering an
    /// update we cannot confirm exists is how a contributor ends up
    /// restarting an app for no reason.
    /// </summary>
    public static bool ShouldOfferUpdate(TcUpdateAvailability availability) =>
        availability == TcUpdateAvailability.Available
        || availability == TcUpdateAvailability.Required;

    /// <summary>
    /// One sentence per availability, for the banner and the status line.
    /// Fixed strings only, per the repo's label-only rule for anything that
    /// reaches a screen or a log.
    /// </summary>
    public static string DescribeAvailability(TcUpdateAvailability availability) =>
        availability switch
        {
            TcUpdateAvailability.Available =>
                "A newer version is ready to install.",
            TcUpdateAvailability.Required =>
                "A required update is ready to install.",
            TcUpdateAvailability.NoUpdates =>
                "Trace Commons is up to date.",
            TcUpdateAvailability.Unknown =>
                "Updates are not managed for this installation.",
            _ =>
                "The update check did not complete.",
        };
}
