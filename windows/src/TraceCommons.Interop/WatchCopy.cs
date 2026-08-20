using System;

namespace TraceCommons.Interop;

/// <summary>
/// Onboarding screen 5's words, and the rule for recognising the bucket that
/// holds sessions with no resolvable project.
///
/// In the interop assembly rather than a view model for the reason
/// <see cref="SessionRootsCopy"/> gives: this is the screen that decides which
/// of a contributor's repositories are eligible to leave the machine, so it is
/// a safety property of the shell, and here it is exercised by tests on a
/// machine that cannot build WinUI at all.
///
/// Every string below is TRANSCRIBED from
/// <c>docs/superpowers/specs/2026-08-08-contributor-shell-shared-design.md</c>,
/// "### 5. What to watch". That section carried no copy until 2026-08-19, so
/// this screen shipped in all three shells as a bare title over an unlabelled
/// list. The words are now specified precisely so the three shells describe one
/// decision the same way -- do not reword them here alone.
/// </summary>
public static class WatchCopy
{
    /// <summary>The screen's heading.</summary>
    public const string Title = "What to watch";

    /// <summary>
    /// The subtitle. States the DEFAULT before the exception, on purpose: the
    /// default is what happens to a contributor who reads nothing and clicks
    /// Continue, which is most of them.
    /// </summary>
    public const string Subtitle =
        "Every project starts at ask-first: you see each session before anything is sent. "
        + "Ignore a project to leave it out entirely.";

    /// <summary>The eyebrow over the list. Rendered uppercase by the style.</summary>
    public const string Section = "Projects";

    /// <summary>
    /// The per-row state for a project that has not been ignored. This is the
    /// vocabulary Settings already uses for the same mode: two screens setting
    /// one field must not name it two ways.
    /// </summary>
    public const string AskMeFirst = "Ask me first";

    /// <summary>
    /// The state after <c>Ignore</c>. Echoes the button that produced it rather
    /// than introducing a third name for the mode.
    /// </summary>
    public const string Ignored = "Ignored";

    /// <summary>Shown when the daemon reports no projects at all.</summary>
    public const string Empty =
        "No projects yet. Sessions you run later will appear here, and in Settings.";

    /// <summary>
    /// The bucket's name, from <see cref="UnresolvedBucketCopy"/>. Settings
    /// shows the same row, so the words live in one place; see that type for
    /// why the wire's slug is not shown.
    /// </summary>
    public const string UnknownLabel = UnresolvedBucketCopy.Label;

    /// <summary>
    /// Why the bucket can never be armed, from
    /// <see cref="UnresolvedBucketCopy"/>.
    ///
    /// On this screen it REPLACES the state line rather than adding a third:
    /// "you'll always be asked" already says what <see cref="AskMeFirst"/>
    /// says. Settings keeps its state column and puts the note beneath the
    /// name, because there the state column is the row's own vocabulary and an
    /// empty cell in a list reads as a fault.
    /// </summary>
    public const string UnknownNote = UnresolvedBucketCopy.Note;

    /// <summary>
    /// What to show as a row's name: the human label for the unresolvable
    /// bucket, the daemon's label otherwise.
    /// </summary>
    /// <param name="isUnresolvedBucket">
    /// The daemon's own <c>is_unresolved_bucket</c> flag. This shell does not
    /// work the answer out for itself: the daemon decides, because the daemon
    /// is what refuses to arm the row. Recognising it any other way -- by the
    /// label, or by re-deriving the opaque id's hash -- is forbidden by
    /// <c>docs/contributor-daemon-ipc-v1_1.md</c> and was a second way to know
    /// one thing.
    /// </param>
    public static string LabelFor(bool isUnresolvedBucket, string? projectLabel)
    {
        if (isUnresolvedBucket)
        {
            return UnknownLabel;
        }

        return string.IsNullOrWhiteSpace(projectLabel) ? UnknownLabel : projectLabel;
    }

    /// <summary>
    /// The line beneath a row's name: the note for the unresolvable bucket,
    /// otherwise the mode. The note replaces the state rather than joining it.
    /// </summary>
    public static string SubLineFor(bool isUnresolvedBucket, string? mode)
    {
        if (isUnresolvedBucket)
        {
            return UnknownNote;
        }

        return string.Equals(mode, "ignore", StringComparison.Ordinal) ? Ignored : AskMeFirst;
    }
}
