using System;

namespace TraceCommons.Interop;

/// <summary>
/// The health banner's words, transcribed from the shared design's
/// failure-state table (2026-08-08, "Failure states") and split into the
/// title/body pair the visual design draws for the Windows frame
/// (2026-08-17 section 5.1 item 3).
///
/// Two rules bind every sentence here, and both are the design's:
/// <list type="bullet">
/// <item><b>Never name the mechanism.</b> "Privacy filter", "claim",
/// "ingest", "canary" and "PII" are internal words. The label carries them;
/// the sentence must not.</item>
/// <item><b>Always state the data consequence.</b> "Nothing has been lost",
/// "your queue is safe", "rather than going out unscanned".</item>
/// </list>
/// </summary>
/// <remarks>
/// <para>
/// This is a pure one-label-to-one-banner mapping and it must stay that way.
/// <c>status.health.last_error_label</c> carries a single label that the
/// daemon has already resolved through its own precedence order
/// (<c>daemon::health::precedence</c>: not-logged-in outranks the near-AI
/// notice, which outranks the self-test failure, and so on down). A client
/// that reconstructed that order would eventually disagree with the daemon
/// about what is wrong, and a contributor would be told one thing by the tray
/// and a different thing by the window. So this renders whichever label
/// arrives and never ranks, merges, or synthesises one. The Linux shell's
/// <c>render_health</c> carries the same note for the same reason.
/// </para>
/// <para>
/// It lives in the interop assembly rather than in a view model for the same
/// reason <see cref="ReadGate"/> does: it is the wording half of a safety
/// surface, and here it is exercised by tests on a machine that cannot build
/// WinUI at all.
/// </para>
/// </remarks>
public sealed class HealthCopy : IEquatable<HealthCopy>
{
    private HealthCopy(string title, string detail, string? actionLabel)
    {
        Title = title;
        Detail = detail;
        ActionLabel = actionLabel;
    }

    /// <summary>The banner's first line, 13px/600 in the frame.</summary>
    public string Title { get; }

    /// <summary>The rest of the sentence, and where the consequence is stated.</summary>
    public string Detail { get; }

    /// <summary>
    /// The banner's action, for the two labels the design gives one.
    /// </summary>
    /// <remarks>
    /// Null for everything else, and deliberately so. The other conditions
    /// clear on their own, and a button that cannot change the condition it
    /// sits beside teaches a contributor that the buttons in this app do
    /// nothing -- which is a lesson they would then apply to Undo.
    /// </remarks>
    public string? ActionLabel { get; }

    /// <summary>
    /// The banner for a health label, or null when there is nothing wrong.
    /// </summary>
    /// <remarks>
    /// A null or empty label is health, not an unknown condition: the daemon
    /// expresses "fine" as the absence of a label.
    /// </remarks>
    public static HealthCopy? ForLabel(string? label)
    {
        if (string.IsNullOrEmpty(label))
        {
            return null;
        }

        return label switch
        {
            "not-logged-in" => new HealthCopy(
                "Not connected.",
                "Sessions are being queued, but nothing can be sent until you reconnect. "
                + "Nothing has been lost.",
                "Reconnect"),
            "near-ai-notice-not-acknowledged" => new HealthCopy(
                "One thing to confirm.",
                "You chose the extra privacy scan, which sends message text to NEAR AI. "
                + "Confirm you're OK with that and contributions resume.",
                "Review and confirm"),
            "privacy-filter-canary-failed" => new HealthCopy(
                "The privacy scan failed its own self-test,",
                "so nothing is being sent through it. This is deliberate -- a scan we can't "
                + "verify doesn't get used.",
                null),
            "pii-filter-unavailable" => new HealthCopy(
                "The extra privacy scan isn't reachable.",
                "Your traces are waiting rather than going out unscanned. Retrying "
                + "automatically.",
                null),
            "claim-mint-failed" or "ingest-unreachable" => new HealthCopy(
                "Can't reach Trace Commons right now.",
                "Your queue is safe; it'll retry on its own.",
                null),
            "queue-full" => new HealthCopy(
                "Trace Commons has stopped queuing new sessions",
                "-- 500 are already waiting. Review or clear some to start again.",
                null),
            "daily-cap-reached" => new HealthCopy(
                "Daily limit reached.",
                "The rest goes out tomorrow.",
                null),

            // An unrecognised label is still a real condition, and the daemon
            // is free to grow labels this build has never heard of. Say the
            // thing that holds for every blocking label rather than inventing
            // a cause, and never render the raw label as the explanation: a
            // label is an internal name, and showing it would break the
            // never-name-the-mechanism rule by the most direct route there is.
            _ => new HealthCopy(
                "Contributions are on hold.",
                "Something is stopping traces from being sent. Nothing has been lost, and "
                + "nothing has gone out.",
                null),
        };
    }

    public bool Equals(HealthCopy? other) =>
        other is not null
        && Title == other.Title
        && Detail == other.Detail
        && ActionLabel == other.ActionLabel;

    public override bool Equals(object? obj) => Equals(obj as HealthCopy);

    public override int GetHashCode() => HashCode.Combine(Title, Detail, ActionLabel);
}
