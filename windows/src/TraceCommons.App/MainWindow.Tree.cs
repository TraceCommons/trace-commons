using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Text.Json;
using System.Threading.Tasks;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using TraceCommons.App.Controls;
using TraceCommons.App.ViewModels;
using TraceCommons.Interop;
using TraceCommons.Interop.Map;
using TraceCommons.Interop.Shell;

namespace TraceCommons.App;

/// <summary>
/// The Traces tree in the main pane (the Mac's <c>TracesTreeView</c>, W3),
/// in place of the folder list. It draws the map's read of the queue and
/// acts only through this window's existing consent paths: Submit all,
/// Submit all as, Ignore, a trace's Submit and Dismiss.
/// </summary>
public sealed partial class MainWindow : ITracesTreeHost
{
    private TracesTreeView? _tree;
    private readonly HashSet<string> _submittingTraceIds = new(StringComparer.Ordinal);

    private FolderAutomatic? _automatic;

    /// <summary>The folder rows' Automatic writes, through the arming offer's call and refusal line.</summary>
    FolderAutomatic ITracesTreeHost.AutomaticWrites => _automatic ??= NewFolderAutomatic();

    private FolderAutomatic NewFolderAutomatic()
    {
        var writes = new FolderAutomatic(
            (method, payload) => ViewModel.Host.CallAsync(method, payload),
            // A confirmed write reads everything again, as before: the core's
            // answer is the authority. What the reads draw is cheap when little
            // changed: the tree builds only the rows whose values moved, and
            // the map keeps its scene when its read is the same as the last.
            async () =>
            {
                await ViewModel.RefreshAsync();
                if (_map is not null)
                {
                    await _map.ReadAsync();
                }
            },
            () => MonitorWords.ProjectModeFailed);
        writes.Changed += () => _tree?.Refresh();
        ViewModel.Host.Account.BoundaryCrossed += writes.AccountBoundary;
        return writes;
    }

    /// <summary>
    /// A folder row's Automatic switch: the write, or for the bucket of
    /// traces with no project, nothing; the note under it already says why
    /// (TracesViews.swift:685-691), and the switch is drawn back as it was.
    /// </summary>
    async void ITracesTreeHost.SetFolderAutomatic(TracesTree.FolderNode folder, bool on)
    {
        if (folder.IsBucket)
        {
            _tree?.Redraw(folder.Id);
            return;
        }

        // The row is drawn as the core has it once the write is done: a
        // write refused, or one that changed nothing, puts the switch back.
        await ((ITracesTreeHost)this).AutomaticWrites.SetAsync(folder, on);
        _tree?.Redraw(folder.Id);
    }

    /// <summary>The person's folder order, kept beside the daemon's state as the map style is.</summary>
    private static string FolderOrderPath() =>
        Path.Combine(DaemonHost.DefaultConfigDir(), "shell", "traces.folderOrder");

    private void InitializeTree(TracesMapView map)
    {
        (IReadOnlyList<string> order, bool arranged) = LoadFolderOrder();
        _tree = new TracesTreeView(new TracesTreeState(order, arranged), this);
        TracesTreeHost.Child = _tree;
        WireAutomaticGrantRead();

        // The empty state (in the tree, as the Mac's emptyTree is) and the
        // Windows sections that sat in rows of their own beside the folder
        // list now follow the tree in its scroll, so the tree has the
        // tab's room as on the Mac (round 2, box: at 615 px they left it none).
        foreach (FrameworkElement section in new FrameworkElement[] { QueueEmptyState, CertificateSection, ScrubCaveatLine, WeekBandSection })
        {
            if (!TracesContent.Children.Remove(section))
            {
                continue;
            }

            _tree.AddFooter(section);
        }
        _tree.Scroller.ViewChanged += OnQueueScrollViewChanged;
        map.SnapshotRead += snapshot =>
        {
            _tree.Read(snapshot, MapFeed.CoreScansWhenUnset);
            RefreshMaySubmit();
            QueueEmptyState.Visibility = snapshot.Ready && snapshot.Tree(MapFeed.CoreScansWhenUnset, showsIgnored: true).Folders.Count == 0
                ? Visibility.Visible
                : Visibility.Collapsed;
        };
        map.FolderSelected += projectId => _tree.Select(new TreeSelection.Folder(projectId));

        // The queue's own rows carry what the tree draws beside the map's
        // read: sizes, holds, and what Submit may send.
        // A refresh clears and refills both a row at a time: the tree draws
        // once after it, building only the rows that changed.
        ViewModel.Groups.CollectionChanged += (_, _) => _tree.RequestRefresh();
        ViewModel.Pending.CollectionChanged += (_, _) =>
        {
            _tree.RequestRefresh();
            RefreshMaySubmit();
        };
    }

    /// <summary>
    /// Each waiting trace's <see cref="QueueEntryViewModel.MaySubmit"/> from
    /// the tree's read, so the inspector's trace cards offer Submit exactly
    /// where the tree's rows do (TracesStore.swift:801-806).
    /// </summary>
    private void RefreshMaySubmit()
    {
        TracesTreeState? state = _tree?.State;
        foreach (QueueEntryViewModel entry in ViewModel.Pending)
        {
            entry.MaySubmit = !IsTraceSubmitting(entry.EntryId)
                && state is not null && state.MaySubmit(entry.EntryId, entry.CanContribute, entry.StateCode, entry.ReasonCode);
        }
    }

    private bool IsTraceSubmitting(string entryId) => _submittingTraceIds.Contains(entryId);

    bool ITracesTreeHost.IsTraceSubmitting(string entryId) => IsTraceSubmitting(entryId);

    private void RefreshTraceSubmitControls()
    {
        RefreshMaySubmit();
        _tree?.RequestRefresh();
    }

    QueueGroupViewModel? ITracesTreeHost.Group(string projectId) =>
        ViewModel.Groups.FirstOrDefault(g => g.ProjectId == projectId);

    QueueEntryViewModel? ITracesTreeHost.Entry(string entryId) =>
        ViewModel.Pending.FirstOrDefault(e => e.EntryId == entryId);

    /// <summary>
    /// A row selected: the inspector's pane follows it to its folder;
    /// nothing selected goes back to the folders (TracesInspectorHost.swift).
    /// </summary>
    void ITracesTreeHost.Selected(TreeSelection? selection)
    {
        TracesTree tree = TreeNow();
        SetGraphTool(TracesTreeRows.ToolOf(selection, tree));
        switch (selection)
        {
            case TreeSelection.Folder folder:
                ViewModel.OpenFolder(folder.ProjectId);
                break;
            case TreeSelection.Session session when TracesTreeRows.FolderOf(session.EntryId, tree) is { } projectId:
                ViewModel.OpenFolder(projectId);
                break;
            default:
                ViewModel.CloseFolder();
                break;
        }

        // A selected trace is a demand: its card is the inspector's
        // (TracesInspectorHost.swift:182-185). A folder is not.
        _treeReview = (selection as TreeSelection.Session)?.EntryId;
        ApplyDemand();

        // A trace opens the inspector, every time; a folder never does
        // (MonitorWindowView.swift:828-831): its traces open in the tree.
        if (TracesTreeRows.OpensInspector(selection))
        {
            OpenInspectorForFolder();
        }
    }

    /// <summary>The trace selected in the tree, the inspector demand's <c>review</c> key.</summary>
    private string? _treeReview;

    /// <summary>Return on a trace: select it and show the inspector its review lives in (TracesViews.swift:15-30).</summary>
    void ITracesTreeHost.Review(string entryId) =>
        ((ITracesTreeHost)this).Selected(new TreeSelection.Session(entryId));

    async void ITracesTreeHost.SubmitFolder(QueueGroupViewModel group, string? outcome) =>
        await ViewModel.SubmitProjectAsync(group.ProjectId, outcome);

    async void ITracesTreeHost.IgnoreFolder(QueueGroupViewModel group) => await ConfirmIgnoreAsync(group);

    /// <summary>A trace's Submit, asked again at the press against the live row (TracesStore.swift:810-815).</summary>
    async void ITracesTreeHost.SubmitTrace(QueueEntryViewModel entry)
    {
        QueueEntryViewModel? live = ViewModel.LiveEntry(entry.EntryId);
        TracesTreeState? state = _tree?.State;
        if (live is null || state is null || !state.MaySubmit(live.EntryId, live.CanContribute, live.StateCode, live.ReasonCode)
            || !_submittingTraceIds.Add(live.EntryId))
        {
            return;
        }

        try
        {
            RefreshTraceSubmitControls();
            await ViewModel.SubmitEntryAsync(live);
        }
        finally
        {
            _submittingTraceIds.Remove(live.EntryId);
            RefreshTraceSubmitControls();
        }
    }

    /// <summary>
    /// Ron's Dismiss confirmation (TracesViews.swift:890-922): the trace by
    /// when and size, Keep it, and Dismiss, never on Return. Every word is
    /// the core's.
    /// </summary>
    async void ITracesTreeHost.DismissTrace(QueueEntryViewModel entry)
    {
        TreeWords words = TreeWords.Core.Value;
        MapEntry? mapped = TreeNow().AllSessions.FirstOrDefault(e => e.EntryId == entry.EntryId);
        string when = mapped is null ? "—" : TracesTreeRows.When(mapped, TimeZoneInfo.Local, CultureInfo.CurrentCulture);
        string body = TreeWords.Fill(
            TreeWords.Fill(words["dismiss_session_body"], "when", when),
            "size", TracesTreeRows.Size(entry.SizeBytes, CultureInfo.CurrentCulture));
        var dialog = new ContentDialog
        {
            XamlRoot = Content.XamlRoot,
            Title = words["dismiss_session_title"],
            Content = body,
            PrimaryButtonText = words.Word("dismiss_action"),
            CloseButtonText = words["dismiss_session_keep"],
            DefaultButton = ContentDialogButton.Close,
        };
        ContentDialogResult outcome = await DialogGuard.ShowOnceAsync(dialog);
        if (outcome == ContentDialogResult.Primary)
        {
            await ViewModel.DismissAsync(entry);
        }
    }

    void ITracesTreeHost.OrderChanged(IReadOnlyList<string> order, bool arranged)
    {
        if (Program.IsSampleLaunch)
        {
            return;
        }

        try
        {
            string path = FolderOrderPath();
            Directory.CreateDirectory(Path.GetDirectoryName(path)!);
            File.WriteAllText(path, JsonSerializer.Serialize(new FolderOrderFile { Order = order.ToList(), Arranged = arranged }));
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException)
        {
            // The order still holds for this run.
        }
    }

    private static (IReadOnlyList<string> Order, bool Arranged) LoadFolderOrder()
    {
        try
        {
            string path = FolderOrderPath();
            if (!Program.IsSampleLaunch && File.Exists(path)
                && JsonSerializer.Deserialize<FolderOrderFile>(File.ReadAllText(path)) is { } saved)
            {
                return (saved.Order ?? new List<string>(), saved.Arranged);
            }
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException or JsonException)
        {
            // An unreadable order is no order.
        }

        return (Array.Empty<string>(), false);
    }

    private TracesTree TreeNow() => _tree?.State.Tree ?? TracesTree.Empty;

    private sealed class FolderOrderFile
    {
        public List<string>? Order { get; set; }

        public bool Arranged { get; set; }
    }
}
