using System;
using System.Collections.Generic;
using System.Text.Json;

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
    /// The core's one name for a folder mode, by wire mode: the disclosure
    /// bundle's <c>folder_mode_labels</c>
    /// (<c>project_copy::FOLDER_MODE_LABELS</c>). Owner decision, 2026-10-02:
    /// a mode reads by one name, Ask me, Automatic or Never, on every surface of every
    /// shell, and this shell types none of the three. <c>ask</c>, this shell's
    /// spelling of <c>notify_only</c> (<see cref="ProjectManualMode"/>), reads
    /// as it. Empty for a mode the table does not name, or if the core could
    /// not produce the bundle, so a row then says nothing rather than
    /// something typed here.
    /// </summary>
    public static string ModeLabel(string? mode) =>
        FolderModeLabelsFromCore.Value.TryGetValue(mode == "ask" ? "notify_only" : mode ?? string.Empty, out string? label)
            ? label
            : string.Empty;

    /// <summary>
    /// The per-row state for a project that has not been ignored: the core's
    /// name for <c>notify_only</c>, the word Settings and the pill use too.
    /// </summary>
    public static string AskMeFirst => ModeLabel("notify_only");

    /// <summary>The state of an ignored project: the core's name for <c>ignore</c>.</summary>
    public static string Ignored => ModeLabel("ignore");

    /// <summary>
    /// The state of a project that uploads without asking: the core's name for
    /// <c>auto_upload</c>.
    ///
    /// Onboarding never arms a project, but a contributor who armed one in
    /// Settings and later walks this screen still sees the row, and a consent
    /// surface that leaves the armed state blank is the one row that must not
    /// go quiet. Settings reads it from here too, so the armed mode has one name.
    /// </summary>
    public static string Armed => ModeLabel("auto_upload");

    /// <summary>
    /// What the row's button says when an ignored or armed project returns
    /// to manual review: the name of the mode it sets, so two screens driving
    /// one field do not name one transition two ways.
    /// </summary>
    public static string RestoreAction => AskMeFirst;

    /// <summary>The button on an ask-first row: the name of the mode it sets.</summary>
    public static string IgnoreAction => Ignored;

    private static readonly Lazy<Dictionary<string, string>> FolderModeLabelsFromCore =
        new(ReadFolderModeLabels);

    private static Dictionary<string, string> ReadFolderModeLabels()
    {
        var table = new Dictionary<string, string>(StringComparer.Ordinal);
        string? json = NativeMethods.TakeOwnedString(NativeMethods.tc_contributor_disclosure_copy_json());
        if (string.IsNullOrEmpty(json)) return table;
        try
        {
            using JsonDocument doc = JsonDocument.Parse(json);
            if (doc.RootElement.ValueKind != JsonValueKind.Object
                || !doc.RootElement.TryGetProperty("folder_mode_labels", out JsonElement labels)
                || labels.ValueKind != JsonValueKind.Object)
            {
                return table;
            }

            foreach (JsonProperty row in labels.EnumerateObject())
            {
                if (row.Value.ValueKind == JsonValueKind.String && row.Value.GetString() is { } word)
                {
                    table[row.Name] = word;
                }
            }
        }
        catch (JsonException)
        {
        }

        return table;
    }

    /// <summary>
    /// What is said when the daemon refused the write. Settings' sentence,
    /// shared so both surfaces report one refusal the same way.
    /// </summary>
    public const string WriteFailed = "That project setting couldn't be changed.";

    /// <summary>
    /// What is said when the write was accepted but the stored state could not
    /// be read back -- the row vanished from the re-read, or the re-read itself
    /// failed. Neither "changed" nor "unchanged" is known to be true, so this
    /// says only what is known: this surface cannot see the stored answer, and
    /// Settings is where it lives.
    /// </summary>
    public const string WriteUnconfirmed =
        "That project setting was sent, but its state couldn't be read back just now. "
        + "Check it in Settings.";

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

        return mode switch
        {
            "ignore" => Ignored,

            // An armed row says that it is armed. Falling through to
            // the ask-first name here would tell a contributor the opposite of
            // what the daemon will do with their next session, and rendering
            // nothing would leave the one row that most needs a state line
            // without one.
            "auto_upload" => Armed,

            // Anything else, including a mode this build does not know, is
            // ask-first: that is the claim that cannot overstate consent, and
            // no row is left blank to reach it.
            _ => AskMeFirst,
        };
    }

    /// <summary>
    /// What the row's button says, or <c>null</c> when there is no action to
    /// offer because <see cref="ProjectManualMode.Next"/> has no transition out
    /// of the mode. A caller holding <c>null</c> hides the control: a disabled
    /// button with no words on it is a zero-width target that says nothing.
    ///
    /// Both project surfaces read this. The transition is the same one on both,
    /// so the words are too.
    /// </summary>
    public static string? ActionFor(string? mode) => ProjectManualMode.Next(mode) switch
    {
        "ignore" => IgnoreAction,
        "ask" => RestoreAction,
        _ => null,
    };
}
