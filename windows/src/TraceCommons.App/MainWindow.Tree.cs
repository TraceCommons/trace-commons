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

    /// <summary>The person's folder order, kept beside the daemon's state as the map style is.</summary>
    private static string FolderOrderPath() =>
        Path.Combine(DaemonHost.DefaultConfigDir(), "shell", "traces.folderOrder");

    private void InitializeTree(TracesMapView map)
    {
        (IReadOnlyList<string> order, bool arranged) = LoadFolderOrder();
        _tree = new TracesTreeView(new TracesTreeState(order, arranged), this);
        TracesTreeHost.Child = _tree;

        // The Windows sections that sat in rows of their own beside the
        // folder list now follow the tree in its scroll, so the tree has the
        // tab's room as on the Mac (round 2, box: at 615 px they left it none).
        foreach (FrameworkElement section in new FrameworkElement[] { CertificateSection, ScrubCaveatLine, WeekBandSection })
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
            QueueEmptyState.Visibility = snapshot.Ready && snapshot.Tree(MapFeed.CoreScansWhenUnset, showsIgnored: true).Folders.Count == 0
                ? Visibility.Visible
                : Visibility.Collapsed;
        };
        map.FolderSelected += projectId => _tree.Select(new TreeSelection.Folder(projectId));

        // The queue's own rows carry what the tree draws beside the map's
        // read: sizes, holds, and what Submit may send.
        ViewModel.Groups.CollectionChanged += (_, _) => _tree.Refresh();
        ViewModel.Pending.CollectionChanged += (_, _) => _tree.Refresh();
    }

    QueueGroupViewModel? ITracesTreeHost.Group(string projectId) =>
        ViewModel.Groups.FirstOrDefault(g => g.ProjectId == projectId);

    QueueEntryViewModel? ITracesTreeHost.Entry(string entryId) =>
        ViewModel.Pending.FirstOrDefault(e => e.EntryId == entryId);

    /// <summary>
    /// A row selected: the inspector shows its folder, open on it; nothing
    /// selected goes back to the folders (TracesInspectorHost.swift).
    /// </summary>
    void ITracesTreeHost.Selected(TreeSelection? selection)
    {
        TracesTree tree = TreeNow();
        SetGraphTool(TracesTreeRows.ToolOf(selection, tree));
        switch (selection)
        {
            case TreeSelection.Folder folder:
                ViewModel.OpenFolder(folder.ProjectId);
                OpenInspectorForFolder();
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
    }

    /// <summary>The trace selected in the tree, the inspector demand's <c>review</c> key.</summary>
    private string? _treeReview;

    /// <summary>Return on a trace: select it and show the inspector its review lives in (TracesViews.swift:15-30).</summary>
    void ITracesTreeHost.Review(string entryId) =>
        ((ITracesTreeHost)this).Selected(new TreeSelection.Session(entryId));

    async void ITracesTreeHost.SubmitFolder(QueueGroupViewModel group, string? outcome) =>
        await ViewModel.SubmitProjectAsync(group.ProjectId, outcome);

    async void ITracesTreeHost.IgnoreFolder(QueueGroupViewModel group) => await ConfirmIgnoreAsync(group);

    async void ITracesTreeHost.SubmitTrace(QueueEntryViewModel entry) => await ViewModel.SubmitEntryAsync(entry);

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
