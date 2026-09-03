using System;
using System.ComponentModel;
using System.Globalization;
using TraceCommons.Interop;

namespace TraceCommons.App.ViewModels;

/// <summary>
/// One queue row, formatted for display.
///
/// Mostly a read-only projection of <see cref="QueueEntry"/>: it exposes what
/// a row shows, and it deliberately exposes NO raw transcript content -- the
/// full body is reachable only through an explicit preview sheet, which is
/// the C ABI's single content exemption and should stay a deliberate act
/// rather than something a list view can leak by binding to the wrong
/// property.
///
/// The one mutable, notifying piece is <see cref="Preview"/>: the card's own
/// preview, requested through the daemon's bounded scheduler
/// (<c>preview_request</c>) and filled in later by a <c>preview_ready</c>
/// event. It carries an opening-prompt excerpt and a scrubbing receipt when
/// ready, which the sheet's own <c>PreviewSummary</c> already carries in full
/// -- this is the same content exemption, on a second, smaller surface, so it
/// follows the same rule: display only, never a log line.
/// </summary>
public sealed class QueueEntryViewModel : INotifyPropertyChanged
{
    private readonly QueueEntry _entry;
    private PreviewCardOutcome? _preview;

    public QueueEntryViewModel(QueueEntry entry)
    {
        _entry = entry ?? throw new ArgumentNullException(nameof(entry));
    }

    public event PropertyChangedEventHandler? PropertyChanged;

    /// <summary>
    /// What the daemon's preview scheduler has said about this card, or null
    /// before <c>preview_request</c> has been made or answered -- which reads
    /// the same as <see cref="IsPreviewPending"/>, so a fresh row and one
    /// still queued or running look identical: both are honestly "not yet
    /// known", and a card must not guess between them.
    /// </summary>
    public PreviewCardOutcome? Preview
    {
        get => _preview;
        set
        {
            if (ReferenceEquals(_preview, value))
            {
                return;
            }

            _preview = value;
            Raise(nameof(Preview));
            Raise(nameof(IsPreviewPending));
            Raise(nameof(HasOpeningPrompt));
            Raise(nameof(OpeningPromptText));
            Raise(nameof(IsTooLargeToPreview));
            Raise(nameof(TooLargeText));
            Raise(nameof(HasScrubbingReceipt));
            Raise(nameof(ScrubbingReceiptText));
            Raise(nameof(MatchedNothing));
        }
    }

    /// <summary>No answer yet: never requested, or queued, or running.</summary>
    public bool IsPreviewPending => _preview is null || _preview.IsPending;

    public bool HasOpeningPrompt =>
        _preview is { IsReady: true, Summary.OpeningPrompt.Length: > 0 };

    public string OpeningPromptText => _preview?.Summary?.OpeningPrompt ?? string.Empty;

    public bool IsTooLargeToPreview => _preview?.IsTooLarge == true;

    /// <summary>
    /// "too large to preview (367.5 MB)" -- the fixed line plus a stat of the
    /// file, and NOTHING resembling a would-send figure. See
    /// <see cref="PreviewCardOutcome.TooLargeText"/> and the design spec's
    /// "Rejected alternatives": a number derived from anything but the
    /// envelope that would actually be sent is a false number on a consent
    /// surface.
    /// </summary>
    public string TooLargeText
    {
        get
        {
            if (_preview is not { IsTooLarge: true } preview)
            {
                return string.Empty;
            }

            return string.Format(
                CultureInfo.CurrentCulture,
                "{0} ({1})",
                PreviewCardOutcome.TooLargeText,
                FormatBytes(preview.RawSessionBytes));
        }
    }

    public bool HasScrubbingReceipt => _preview is { IsReady: true, Summary: not null };

    public string ScrubbingReceiptText => _preview?.Summary?.RedactionReceipt ?? string.Empty;

    /// <summary>
    /// Whether this session's preview reports that no pattern fired.
    /// </summary>
    /// <remarks>
    /// False until a preview lands, which is the honest reading: not knowing
    /// is not the same as knowing nothing matched, and a rail that raised an
    /// alarm about every card still loading would be raising it about
    /// nothing. Counts REMOVALS, so a session whose only count is a surviving
    /// secret reads as nothing matched here too, which is true and is the
    /// state the gold chip exists to say.
    /// </remarks>
    public bool MatchedNothing =>
        _preview is { IsReady: true, Summary: not null }
        && RedactionLabels.Total(_preview.Summary.Redactions) == 0;

    /// <summary>
    /// Whether part of this conversation was left out to fit its byte budget.
    /// </summary>
    /// <remarks>
    /// A load-time fact on the entry, not a property of the preview, so it is
    /// as true while the card still reads "Loading preview…" as after one
    /// lands. See <see cref="SubagentText"/>.
    /// </remarks>
    public bool WasTrimmed => _entry.SubagentsDropped > 0;

    private void Raise(string name) => PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(name));

    /// <summary>The daemon's identifier for this entry; used to open a preview.</summary>
    public string EntryId => _entry.EntryId;

    /// <summary>
    /// The daemon's identifier for this session's project, as
    /// <c>entry_value</c> publishes it -- the one a project-group submit must
    /// send as <c>approve</c>'s <c>project_id</c>. Never the display string
    /// <see cref="ProjectLabel"/>, which the daemon does not treat as an
    /// identifier. May be empty for an entry the daemon did not associate
    /// with a project.
    /// </summary>
    public string ProjectId => _entry.ProjectId ?? string.Empty;

    /// <summary>
    /// The project this session belongs to. Falls back to the id, then to a
    /// fixed label -- never to a path, which the daemon does not send and this
    /// row must not invent.
    /// </summary>
    public string ProjectLabel =>
        !string.IsNullOrWhiteSpace(_entry.ProjectLabel) ? _entry.ProjectLabel!
        : !string.IsNullOrWhiteSpace(_entry.ProjectId) ? _entry.ProjectId!
        : "Unknown project";

    /// <summary>
    /// Where this session actually ran, when that is not the project root.
    /// </summary>
    /// <remarks>
    /// Key normalization walks up to the enclosing repository, so two sibling
    /// subdirectories of one repo become one folder. That is the merge the
    /// folder-first queue wanted, and this line is what pays for it: the
    /// sessions are grouped by repo and still say individually where they
    /// ran.
    ///
    /// Empty both when the daemon predates the field and when the session ran
    /// at the root -- the daemon sends null in the second case rather than
    /// repeating the project's own path, so the row draws this only when it
    /// says something. Display only, like every path on this surface.
    /// </remarks>
    public string SessionPath => _entry.SessionPath ?? string.Empty;

    /// <summary>Whether there is a session path worth a line of its own.</summary>
    public bool HasSessionPath => SessionPath.Length > 0;

    /// <summary>
    /// The agent that produced the session, in the words a contributor uses
    /// for it rather than the raw source token.
    ///
    /// The macOS and Linux clients both map these, and a card that reads
    /// "claude-code" where the other two read "Claude Code" is three clients
    /// naming the same thing three ways. An unrecognised token is tidied
    /// rather than replaced: the daemon may name an agent this build has
    /// never heard of, and printing it is more useful than hiding it.
    ///
    /// What the transcript DECLARES wins over the adapter that stores it.
    /// An imported Antigravity conversation is a trajectory file, and
    /// calling it "Letta trajectory" names the format rather than the tool
    /// the contributor used.
    /// </summary>
    public string Source => (_entry.DeclaredSource ?? _entry.Source) switch
    {
        null or "" => "—",
        "claude-code" or "claude_code" => "Claude Code",
        "codex" => "Codex",
        "gemini-cli" or "gemini_cli" => "Gemini CLI",
        "antigravity" => "Antigravity",
        "trajectory" or "letta_trajectory" => "Letta trajectory",
        string other when string.IsNullOrWhiteSpace(other) => "—",
        string other => CultureInfo.CurrentCulture.TextInfo.ToTitleCase(
            other.Replace('_', ' ').Replace('-', ' ')),
    };

    public string State => string.IsNullOrWhiteSpace(_entry.State) ? "—" : _entry.State!;

    /// <summary>
    /// Why the entry is in its state. Already written to be read by a
    /// contributor, so it is shown verbatim rather than remapped here -- a
    /// second vocabulary in the UI would drift from the daemon's.
    /// </summary>
    public string? ReasonLabel => _entry.ReasonLabel;

    public bool HasReason => !string.IsNullOrWhiteSpace(_entry.ReasonLabel);

    /// <summary>Human-readable size, binary units to match what Explorer shows.</summary>
    public string SizeText => FormatBytes(_entry.SizeBytes);

    /// <summary>
    /// What this one card actually covers, and -- the half the contract makes
    /// mandatory -- whether any of it was left out to fit. See
    /// <see cref="SubagentCopy"/>.
    ///
    /// Not a property of the preview: both counts are load-time facts carried
    /// on the entry itself, so this line is as true while the card still reads
    /// "Loading preview…" as it is after one lands. A trimmed conversation
    /// must not be able to reach a decision through a card that never got a
    /// preview.
    /// </summary>
    public string SubagentText => SubagentCopy.Line(_entry.SubagentCount, _entry.SubagentsDropped);

    /// <summary>
    /// Whether there is anything to say at all. A session that delegated
    /// nothing and dropped nothing carries no line about subagents -- never a
    /// row reading zero.
    /// </summary>
    public bool HasSubagentText => SubagentText.Length > 0;

    /// <summary>
    /// When the session was discovered, in the viewer's local time. The daemon
    /// sends an RFC 3339 timestamp; an unparsable one degrades to a dash
    /// rather than to the epoch, which would read as a real date.
    /// </summary>
    public string DiscoveredText =>
        DateTimeOffset.TryParse(
            _entry.DiscoveredAt,
            CultureInfo.InvariantCulture,
            DateTimeStyles.RoundtripKind,
            out DateTimeOffset parsed)
            ? parsed.ToLocalTime().ToString("g", CultureInfo.CurrentCulture)
            : "—";

    /// <summary>
    /// Retry state, shown only when the daemon is actually retrying, so a
    /// healthy row carries no noise.
    /// </summary>
    public bool HasAttempts => _entry.Attempts > 0;

    public string AttemptsText => _entry.Attempts == 1
        ? "1 attempt"
        : $"{_entry.Attempts} attempts";

    internal static string FormatBytes(long bytes)
    {
        if (bytes < 0)
        {
            return "—";
        }

        string[] units = { "B", "KB", "MB", "GB" };
        double value = bytes;
        int unit = 0;

        while (value >= 1024 && unit < units.Length - 1)
        {
            value /= 1024;
            unit++;
        }

        // No decimal place on bytes, one everywhere else: "1.4 MB" is useful,
        // "1437.0 B" is not.
        return unit == 0
            ? string.Format(CultureInfo.CurrentCulture, "{0:0} {1}", value, units[unit])
            : string.Format(CultureInfo.CurrentCulture, "{0:0.#} {1}", value, units[unit]);
    }
}
