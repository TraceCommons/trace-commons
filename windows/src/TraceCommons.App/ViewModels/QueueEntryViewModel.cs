using System;
using System.Globalization;
using TraceCommons.Interop;

namespace TraceCommons.App.ViewModels;

/// <summary>
/// One queue row, formatted for display.
///
/// A read-only projection of <see cref="QueueEntry"/>. It exposes only what a
/// row shows, and it deliberately exposes NO transcript content: the body is
/// reachable only through an explicit preview, which is the C ABI's single
/// content exemption and should stay a deliberate act rather than something a
/// list view can leak by binding to the wrong property.
/// </summary>
public sealed class QueueEntryViewModel
{
    private readonly QueueEntry _entry;

    public QueueEntryViewModel(QueueEntry entry)
    {
        _entry = entry ?? throw new ArgumentNullException(nameof(entry));
    }

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
    /// The agent that produced the session, in the words a contributor uses
    /// for it rather than the raw source token.
    ///
    /// The macOS and Linux clients both map these, and a card that reads
    /// "claude-code" where the other two read "Claude Code" is three clients
    /// naming the same thing three ways. An unrecognised token is tidied
    /// rather than replaced: the daemon may name an agent this build has
    /// never heard of, and printing it is more useful than hiding it.
    /// </summary>
    public string Source => _entry.Source switch
    {
        null or "" => "—",
        "claude-code" or "claude_code" => "Claude Code",
        "codex" => "Codex",
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
