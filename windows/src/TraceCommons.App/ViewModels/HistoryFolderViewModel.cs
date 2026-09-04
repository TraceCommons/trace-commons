using System;
using System.Globalization;
using TraceCommons.Interop;

namespace TraceCommons.App.ViewModels;

/// <summary>
/// One folder row in history: a project, and how much of it was contributed.
/// </summary>
/// <remarks>
/// The same relationship <see cref="QueueGroupViewModel"/> has to
/// <see cref="ProjectQueueGroup"/>. The grouping rule is
/// <see cref="HistoryFolders.Group"/>'s, in TraceCommons.Interop, where it is
/// tested; this only carries that decision to the markup.
/// </remarks>
public sealed class HistoryFolderViewModel
{
    private readonly HistoryFolder _folder;

    public HistoryFolderViewModel(HistoryFolder folder, string projectPath)
    {
        _folder = folder ?? throw new ArgumentNullException(nameof(folder));
        ProjectPath = projectPath ?? throw new ArgumentNullException(nameof(projectPath));
    }

    /// <summary>
    /// The key this folder is opened by. A project id, or a label-derived key
    /// for records written before the id existed.
    /// </summary>
    public string Key => _folder.Key;

    public string ProjectLabel => _folder.ProjectLabel;

    /// <summary>
    /// The project's folder, resolved client-side by matching this folder's
    /// project id against the live <c>list_projects</c> rows.
    /// </summary>
    /// <remarks>
    /// Empty for a record whose project the daemon no longer knows, and for
    /// every record written before project ids reached history -- the honest
    /// outcome, and no fallback path is needed. History itself carries no
    /// path: it is one of the three sinks a path must never reach.
    /// </remarks>
    public string ProjectPath { get; }

    /// <summary>Whether there is a path to draw beneath the label.</summary>
    public bool HasProjectPath => ProjectPath.Length > 0;

    /// <summary>How many contributions this project has.</summary>
    public string CountText => _folder.Records.Count == 1
        ? "1 contribution"
        : string.Format(
            CultureInfo.CurrentCulture,
            "{0} contributions",
            _folder.Records.Count);
}
