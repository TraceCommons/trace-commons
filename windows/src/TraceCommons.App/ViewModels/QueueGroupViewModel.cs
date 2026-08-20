using System;
using System.Collections.ObjectModel;
using System.Globalization;
using TraceCommons.Interop;

namespace TraceCommons.App.ViewModels;

/// <summary>
/// One project's header in the grouped queue, and the rows under it.
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

    /// <summary>The header's row count text, e.g. "3 sessions".</summary>
    public string CountText => _group.Count == 1
        ? "1 session"
        : string.Format(CultureInfo.CurrentCulture, "{0} sessions", _group.Count);

    /// <summary>
    /// Whether the header offers "Submit all". False for a single-entry
    /// group: its one row's own Submit button already does what the group
    /// action would, so a second control offering the identical decision
    /// would be noise rather than a second choice. Decided entirely by
    /// <see cref="ProjectQueueGroup.ShowSubmitAll"/>.
    /// </summary>
    public bool ShowSubmitAll => _group.ShowSubmitAll;

    /// <summary>"Submit all (3)" -- the header action's label, when shown.</summary>
    public string SubmitAllText =>
        string.Format(CultureInfo.CurrentCulture, "Submit all ({0})", _group.Count);

    /// <summary>The rows in this project, in queue order.</summary>
    public ObservableCollection<QueueEntryViewModel> Entries { get; }
}
