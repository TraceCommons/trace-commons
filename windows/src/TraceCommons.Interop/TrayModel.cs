using System;
using System.Collections.Generic;
using System.Globalization;
using System.Linq;

namespace TraceCommons.Interop;

/// <summary>
/// Which of the four icon states the tray is in.
/// </summary>
/// <remarks>
/// The order is the shared design spec's precedence, highest first:
/// "attention (numeric badge) -> unhealthy (amber dot) -> paused (struck
/// through) -> idle". The enum values are ordered to match so the precedence
/// reads as one comparison rather than a nest of ifs.
/// </remarks>
public enum TrayIconState
{
    /// <summary>Watching, nothing owed.</summary>
    Idle = 0,

    /// <summary>Paused by the contributor. Nothing is being queued.</summary>
    Paused = 1,

    /// <summary>A health state the contributor should know about.</summary>
    Unhealthy = 2,

    /// <summary>Decisions are owed. Outranks everything else.</summary>
    Attention = 3,
}

/// <summary>
/// Everything the tray icon shows, computed from daemon state.
/// </summary>
/// <remarks>
/// <para>
/// Platform-neutral on purpose: the precedence rule and the copy are the
/// parts worth testing, and they are testable on a machine that cannot draw
/// a Windows tray icon. <c>TraceCommons.App.TrayIcon</c> renders what this
/// produces and adds nothing to it.
/// </para>
/// <para>
/// <b>Nothing here may carry a path, a token, a hash, an identity, or trace
/// content.</b> A tray tooltip is drawn by the shell, can be read over a
/// shoulder, and is captured by accessibility tooling. Counts and fixed
/// labels only -- the same rule the queue rows and the audit log are held to.
/// Project labels are the one variable string admitted, and only in the menu
/// header, because the daemon has already reduced them from a path to a
/// label for exactly this purpose.
/// </para>
/// </remarks>
public sealed class TrayModel
{
    /// <summary>
    /// The Win32 tooltip cap. <c>NOTIFYICONDATAW.szTip</c> is 128 wide
    /// characters including the terminator, and Shell_NotifyIcon silently
    /// truncates or fails on overflow rather than telling anyone.
    /// </summary>
    public const int MaxTooltipLength = 127;

    private TrayModel(TrayIconState state, int decisionsOwed, string tooltip, string menuHeader)
    {
        State = state;
        DecisionsOwed = decisionsOwed;
        Tooltip = tooltip;
        MenuHeader = menuHeader;
    }

    public TrayIconState State { get; }

    /// <summary>
    /// How many things there are to say yes or no to.
    /// </summary>
    /// <remarks>
    /// The spec is explicit that this counts decisions owed and never the
    /// queue total or anything to do with credit: "If it shows 3, there are
    /// exactly three things to say yes or no to."
    /// </remarks>
    public int DecisionsOwed { get; }

    /// <summary>The hover tooltip. Fixed labels and one count.</summary>
    public string Tooltip { get; }

    /// <summary>
    /// The disabled first line of the context menu, restating what the icon
    /// is showing so the state is readable rather than inferred from a glyph.
    /// </summary>
    public string MenuHeader { get; }

    /// <summary>
    /// Applies the spec's precedence.
    /// </summary>
    /// <param name="decisionsOwed">Pending entries awaiting a decision.</param>
    /// <param name="isPaused">Whether the contributor has paused watching.</param>
    /// <param name="isHealthy">Whether daemon health is clear.</param>
    public static TrayModel Compute(int decisionsOwed, bool isPaused, bool isHealthy)
    {
        int owed = Math.Max(0, decisionsOwed);

        TrayIconState state =
            owed > 0 ? TrayIconState.Attention
            : !isHealthy ? TrayIconState.Unhealthy
            : isPaused ? TrayIconState.Paused
            : TrayIconState.Idle;

        string detail = state switch
        {
            // Attention outranks paused and unhealthy in the icon, but the
            // tooltip still says which of them is also true: an icon that
            // silently drops "paused" would let a contributor approve three
            // sessions while believing the watcher is running, and vice
            // versa.
            TrayIconState.Attention when isPaused => $"{Waiting(owed)} Paused.",
            TrayIconState.Attention when !isHealthy => $"{Waiting(owed)} Needs attention.",
            TrayIconState.Attention => Waiting(owed),
            TrayIconState.Unhealthy => "Needs attention.",
            TrayIconState.Paused => "Paused. Nothing is being queued.",
            _ => "Watching. Nothing waiting.",
        };

        return new TrayModel(state, owed, Truncate($"Trace Commons — {detail}"), detail);
    }

    /// <summary>
    /// "3 sessions waiting for review." -- the same sentence the main window's
    /// status line uses, so the tray and the window never word the same fact
    /// two ways.
    /// </summary>
    private static string Waiting(int owed) => owed == 1
        ? "1 session waiting for review."
        : string.Format(CultureInfo.CurrentCulture, "{0} sessions waiting for review.", owed);

    /// <summary>
    /// Trims to what <c>szTip</c> can hold, on a character boundary and with
    /// an ellipsis so a cut is visible rather than looking like the whole
    /// sentence.
    /// </summary>
    internal static string Truncate(string text)
    {
        ArgumentNullException.ThrowIfNull(text);

        return text.Length <= MaxTooltipLength
            ? text
            : string.Concat(text.AsSpan(0, MaxTooltipLength - 1), "…");
    }
}

/// <summary>
/// The digest notification's text, transcribed from the shared design spec
/// rather than paraphrased.
/// </summary>
/// <remarks>
/// <para>
/// The spec writes it as:
/// </para>
/// <code>
/// Trace Commons
/// 3 sessions ready from trace-commons-server and dotfiles.
/// Nothing is sent until you review them.
/// </code>
/// <para>
/// The Linux shell's <c>notify::digest_body</c> and the macOS
/// <c>Notifier.postDigest</c> produce exactly these words; this produces them
/// too, and the tests assert against the spec's own example so a future edit
/// to one shell's wording shows up here as a failure.
/// </para>
/// <para>
/// Project labels only, never paths, and never a line of transcript. A
/// notification is rendered by the shell, may be persisted in the Windows
/// notification centre, and is exactly the wrong place for content.
/// </para>
/// </remarks>
public static class DigestText
{
    /// <summary>The notification title on every shell.</summary>
    public const string Title = "Trace Commons";

    /// <summary>
    /// The second line, which is the whole reassurance: an unread digest
    /// costs nothing.
    /// </summary>
    public const string NothingSent = "Nothing is sent until you review them.";

    /// <summary>
    /// Builds the digest body for a pending count and the project labels
    /// those entries came from.
    /// </summary>
    public static string Body(int pendingCount, IReadOnlyList<string> projectLabels)
    {
        ArgumentNullException.ThrowIfNull(projectLabels);

        string noun = pendingCount == 1 ? "session" : "sessions";
        string from = JoinProjects(projectLabels);
        return $"{pendingCount} {noun} ready{from}.\n{NothingSent}";
    }

    /// <summary>
    /// "a", "a and b", "a, b and c" -- the spec's own list form. An empty
    /// list yields an empty string, so the sentence degrades to "3 sessions
    /// ready." rather than trailing a dangling "from".
    /// </summary>
    private static string JoinProjects(IReadOnlyList<string> labels)
    {
        var named = new List<string>(labels.Count);
        foreach (string label in labels)
        {
            if (!string.IsNullOrWhiteSpace(label) && !named.Contains(label, StringComparer.Ordinal))
            {
                named.Add(label);
            }
        }

        return named.Count switch
        {
            0 => string.Empty,
            1 => $" from {named[0]}",
            2 => $" from {named[0]} and {named[1]}",
            _ => $" from {string.Join(", ", named.GetRange(0, named.Count - 1))} and {named[named.Count - 1]}",
        };
    }
}
