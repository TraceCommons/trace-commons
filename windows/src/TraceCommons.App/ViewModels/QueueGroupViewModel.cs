using System;
using System.Collections.ObjectModel;
using System.Globalization;
using TraceCommons.Interop;

namespace TraceCommons.App.ViewModels;

/// <summary>
/// One project's folder row in the queue, and the sessions inside it.
///
/// A read-only projection of <see cref="ProjectQueueGroup"/> plus the rows
/// that belong to it -- the same relationship <see cref="QueueEntryViewModel"/>
/// has to <see cref="TraceCommons.Interop.QueueEntry"/>. Every decision this
/// type could get wrong (which project an entry belongs to, group order,
/// whether "Submit all" appears) has already been made by
/// <see cref="QueueGrouping.ByProject"/> and is covered by its tests in
/// <c>TraceCommons.Interop.Tests</c>; this only carries that decision to the
/// XAML that renders it.
/// </summary>
public sealed class QueueGroupViewModel
{
    private readonly ProjectQueueGroup _group;

    public QueueGroupViewModel(ProjectQueueGroup group, ObservableCollection<QueueEntryViewModel> entries)
    {
        _group = group ?? throw new ArgumentNullException(nameof(group));
        Entries = entries ?? throw new ArgumentNullException(nameof(entries));
    }

    /// <summary>
    /// The id every row in <see cref="Entries"/> shares. Sent as
    /// <c>approve</c>'s <c>project_id</c> by "Submit all" -- never
    /// <see cref="ProjectLabel"/>, which is display text only.
    /// </summary>
    public string ProjectId => _group.ProjectId;

    public string ProjectLabel => _group.ProjectLabel;

    /// <summary>
    /// The project's folder, <c>~</c>-abbreviated, beneath the label.
    /// </summary>
    /// <remarks>
    /// The reason the folder-first layout is worth building. A disambiguated
    /// label keeps two projects DISTINCT (<c>api</c> and <c>api (3f9c)</c>)
    /// but can never make them IDENTIFIABLE, and a contributor deciding what
    /// to upload from which repository needs the second. Display only: never
    /// logged, notified, or persisted -- see
    /// <see cref="ProjectQueueGroup.ProjectPath"/>.
    /// </remarks>
    public string ProjectPath => _group.ProjectPath;

    /// <summary>
    /// Whether there is a path to draw. False against a daemon predating the
    /// field, where the row renders its label alone rather than a blank line.
    /// </summary>
    public bool HasProjectPath => _group.ProjectPath.Length > 0;

    /// <summary>
    /// The folder's total session bytes on disk. Never a would-send figure --
    /// see <see cref="ProjectQueueGroup.SizeBytes"/>.
    /// </summary>
    public string SizeText => QueueEntryViewModel.FormatBytes(_group.SizeBytes);

    /// <summary>The folder row's count text, e.g. "3 sessions".</summary>
    public string CountText => _group.Count == 1
        ? "1 session"
        : string.Format(CultureInfo.CurrentCulture, "{0} sessions", _group.Count);

    /// <summary>
    /// Whether the folder row offers "Submit all". True at every count,
    /// including one: the row that used to make it redundant is now a level
    /// down, so hiding it would mean opening a folder to do the thing the
    /// folder is offering. Decided entirely by
    /// <see cref="ProjectQueueGroup.ShowSubmitAll"/>, which records the
    /// history.
    /// </summary>
    public bool ShowSubmitAll => _group.ShowSubmitAll;

    /// <summary>"Submit all (3)" -- the header action's label, when shown.</summary>
    public string SubmitAllText =>
        string.Format(CultureInfo.CurrentCulture, "Submit all ({0})", _group.Count);

    /// <summary>
    /// "Submit all as..." and its tooltip: the opt-in verdict path beside
    /// "Submit all", shown under exactly the same condition
    /// (<see cref="ShowSubmitAll"/>) because it approves exactly the same
    /// set. Bound from <see cref="VerdictCopy"/>, not typed here.
    /// </summary>
    public string SubmitAllAsText => VerdictCopy.SubmitAllAs;

    /// <summary>See <see cref="SubmitAllAsText"/>.</summary>
    public string SubmitAllAsTooltip => VerdictCopy.SubmitAllAsTooltip;

    /// <summary>The three verdict labels the menu offers.</summary>
    public string VerdictWorkedLabel => VerdictCopy.Worked;

    /// <summary>See <see cref="VerdictWorkedLabel"/>.</summary>
    public string VerdictPartlyLabel => VerdictCopy.Partly;

    /// <summary>See <see cref="VerdictWorkedLabel"/>.</summary>
    public string VerdictFailedLabel => VerdictCopy.Failed;

    /// <summary>
    /// Whether the folder row offers "Ignore project". Always true: declining
    /// a whole project is not something any row-level control does, so the
    /// folder action belongs at every group size.
    /// </summary>
    public bool ShowIgnoreProject => true;

    /// <summary>
    /// "Ignore project" and its tooltip, bound rather than typed into the
    /// XAML. The whole point of <see cref="ProjectIgnoreCopy"/> is that this
    /// string exists in three shells and drifts; a copy of it sitting in
    /// markup is outside everything that keeps it from drifting.
    /// </summary>
    public string IgnoreProjectText => ProjectIgnoreCopy.ButtonLabel;

    /// <summary>See <see cref="IgnoreProjectText"/>.</summary>
    public string IgnoreProjectTooltip => ProjectIgnoreCopy.Tooltip;

    /// <summary>
    /// The number of waiting sessions this project would lose if ignored --
    /// what <see cref="ProjectIgnoreCopy.ConfirmationBody"/> is told.
    /// </summary>
    public int PendingCount => _group.Count;

    /// <summary>The sessions in this project, in queue order.</summary>
    public ObservableCollection<QueueEntryViewModel> Entries { get; }
}
