using System;
using System.Linq;
using System.Threading.Tasks;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using TraceCommons.App.ViewModels;
using Windows.Storage.Pickers;

namespace TraceCommons.App.Controls;

public sealed partial class InsightsView : UserControl, IDisposable
{
    private readonly IntPtr _window;
    private string? _file;
    private readonly EpisodeMemberDraftBinding _episodeMemberDraft = new();
    private bool _closed;
    public InsightsViewModel ViewModel { get; } = new();
    public event EventHandler? ContributionSetupRequested;

    public InsightsView(IntPtr window)
    {
        InitializeComponent();
        _window = window;
        DataContext = ViewModel;
        Unloaded += (_, _) => { _episodeMemberDraft.Clear(); ViewModel.Cancel(); };
    }
    public Task ActivateAsync() => ViewModel.LoadAsync();
    public void Deactivate() { _episodeMemberDraft.Clear(); ViewModel.Cancel(); }
    private async void OnChoose(object sender, RoutedEventArgs args)
    {
        try
        {
            var picker = new FileOpenPicker();
            WinRT.Interop.InitializeWithWindow.Initialize(picker, _window);
            picker.FileTypeFilter.Add(".jsonl");
            picker.FileTypeFilter.Add(".json");
            picker.FileTypeFilter.Add("*");
            var file = await picker.PickSingleFileAsync();
            if (_closed || file == null) return;
            _file = file.Path;
            SelectedFile.Text = file.Name;
        }
        catch (Exception) { ViewModel.ReportError(); }
    }
    private string SourceTag => ((ComboBoxItem)Source.SelectedItem).Tag.ToString()!;
    private async void OnAnalyze(object sender, RoutedEventArgs args)
    {
        if (_file != null) await ViewModel.AnalyzeAsync(SourceTag, _file, false);
    }
    private async void OnSave(object sender, RoutedEventArgs args)
    {
        if (_file != null) await ViewModel.AnalyzeAsync(SourceTag, _file, true);
    }
    private async void OnSummaryEvidence(object sender, RoutedEventArgs args)
    {
        if (sender is not Button { Tag: string id }) return;
        await ViewModel.ExplainSummaryEvidenceAsync(id);
        BringSnapshotIntoView(id);
    }
    private async void OnRefresh(object sender, RoutedEventArgs args) => await ViewModel.RefreshAsync();
    private async void OnExplain(object sender, RoutedEventArgs args)
    {
        if (SavedList.SelectedItem is not SavedInsight selected) return;
        await ViewModel.ExplainAsync(selected.Id);
        BringSnapshotIntoView(selected.Id);
    }
    private string[] SelectedSnapshotIds() => SavedList.SelectedItems.Cast<SavedInsight>().Select(item => item.Id).ToArray();
    private async void OnCreateEpisode(object sender, RoutedEventArgs args)
    {
        await ViewModel.CreateEpisodeAsync(SelectedSnapshotIds());
        if (ViewModel.CurrentEpisodeId is { } id) ReconcileEpisodeMemberDraft(id);
    }
    private async void OnOpenEpisode(object sender, RoutedEventArgs args)
    {
        if (EpisodeList.SelectedItem is not EpisodeRow selected) return;
        await ViewModel.OpenEpisodeAsync(selected.Id);
        if (_closed || ViewModel.CurrentEpisodeId != selected.Id) return;
        ReconcileEpisodeMemberDraft(selected.Id);
        DispatcherQueue.TryEnqueue(() => { if (!_closed && IsLoaded && ViewModel.CurrentEpisodeId == selected.Id) EpisodeDetail.StartBringIntoView(); });
    }
    private async void OnSaveEpisodeMembers(object sender, RoutedEventArgs args)
    {
        var target = _episodeMemberDraft.Consume();
        if (target == null) return;
        await ViewModel.ReplaceEpisodeMembersAsync(target, SelectedSnapshotIds());
        ReconcileEpisodeMemberDraft(target.Id);
    }
    private async void OnSaveEpisodeAssessment(object sender, RoutedEventArgs args)
    {
        var target = ViewModel.CaptureEpisodeTarget();
        if (target == null) return;
        await ViewModel.SaveEpisodeAssessmentAsync(target);
        ReconcileEpisodeMemberDraft(target.Id);
    }
    private async void OnClearEpisodeAssessment(object sender, RoutedEventArgs args)
    {
        var target = ViewModel.CaptureEpisodeTarget();
        if (target == null) return;
        var dialog = new ContentDialog { XamlRoot = XamlRoot, Title = ViewModel["episode_clear_assessment"],
            Content = ViewModel["episode_clear_assessment_confirm"], PrimaryButtonText = ViewModel["episode_clear_assessment"],
            CloseButtonText = ViewModel["cancel"], DefaultButton = ContentDialogButton.Close };
        if (await dialog.ShowAsync() == ContentDialogResult.Primary && !_closed)
        {
            await ViewModel.ClearEpisodeAssessmentAsync(target);
            ReconcileEpisodeMemberDraft(target.Id);
        }
    }
    private bool ReconcileEpisodeMemberDraft(string expectedId)
    {
        if (!_episodeMemberDraft.Reconcile(ViewModel, expectedId)) return false;
        var members = ViewModel.EpisodeMembers.Select(member => member.Id).ToHashSet(StringComparer.Ordinal);
        SavedList.SelectedItems.Clear();
        foreach (var saved in ViewModel.Saved.Where(saved => members.Contains(saved.Id))) SavedList.SelectedItems.Add(saved);
        return true;
    }
    private async void OnDeleteEpisode(object sender, RoutedEventArgs args)
    {
        var target = ViewModel.CaptureEpisodeTarget();
        if (target == null) return;
        var dialog = new ContentDialog { XamlRoot = XamlRoot, Title = ViewModel["episode_delete"],
            Content = ViewModel["episode_delete_confirm"], PrimaryButtonText = ViewModel["episode_delete"],
            CloseButtonText = ViewModel["cancel"], DefaultButton = ContentDialogButton.Close };
        if (await dialog.ShowAsync() == ContentDialogResult.Primary && !_closed)
            await ViewModel.DeleteEpisodeAsync(target);
    }
    private void BringSnapshotIntoView(string id)
    {
        if (_closed || ViewModel.CurrentId != id) return;
        DispatcherQueue.TryEnqueue(() =>
        {
            if (!_closed && IsLoaded && ViewModel.CurrentId == id)
                SnapshotDetail.StartBringIntoView();
        });
    }
    private async void OnDelete(object sender, RoutedEventArgs args)
    {
        var dialog = new ContentDialog {
            XamlRoot = XamlRoot, Title = ViewModel["delete"], Content = ViewModel["delete_notice"],
            PrimaryButtonText = ViewModel["delete"], CloseButtonText = ViewModel["cancel"],
            DefaultButton = ContentDialogButton.Close
        };
        if (await dialog.ShowAsync() == ContentDialogResult.Primary && !_closed)
            await ViewModel.DeleteAsync();
    }
    private async void OnLinkGit(object sender, RoutedEventArgs args)
    {
        var target = ViewModel.CaptureEvidenceTarget();
        if (target == null) return;
        string commit = ViewModel.CommitId;
        try
        {
            var picker = new FolderPicker();
            WinRT.Interop.InitializeWithWindow.Initialize(picker, _window);
            picker.FileTypeFilter.Add("*");
            var folder = await picker.PickSingleFolderAsync();
            if (folder != null) await ViewModel.LinkGitAsync(target, folder.Path, commit);
            else ViewModel.CancelEvidenceTarget(target);
        }
        catch (Exception) { ViewModel.CancelEvidenceTarget(target); ViewModel.ReportError(); }
    }
    private async void OnLinkTestReport(object sender, RoutedEventArgs args)
    {
        var target = ViewModel.CaptureEvidenceTarget();
        if (target == null) return;
        try
        {
            var picker = new FileOpenPicker();
            WinRT.Interop.InitializeWithWindow.Initialize(picker, _window);
            picker.FileTypeFilter.Add(".json");
            var file = await picker.PickSingleFileAsync();
            if (file != null) await ViewModel.LinkTestReportAsync(target, file.Path);
            else ViewModel.CancelEvidenceTarget(target);
        }
        catch (Exception) { ViewModel.CancelEvidenceTarget(target); ViewModel.ReportError(); }
    }
    private async void OnUnlinkEvidence(object sender, RoutedEventArgs args)
    {
        if (sender is not Button { Tag: OutcomeEvidenceRow row }) return;
        var target = ViewModel.CaptureEvidenceTarget();
        if (target != null && target.SnapshotId == row.SnapshotId)
            await ViewModel.UnlinkEvidenceAsync(target, row.Id);
    }
    private async void OnAnnotate(object sender, RoutedEventArgs args) => await ViewModel.SaveAssessmentAsync();
    private async void OnClear(object sender, RoutedEventArgs args) => await ViewModel.ClearAnnotationAsync();
    private void OnCancel(object sender, RoutedEventArgs args) { _episodeMemberDraft.Clear(); ViewModel.Cancel(); }
    private void OnContributions(object sender, RoutedEventArgs args) => ContributionSetupRequested?.Invoke(this, EventArgs.Empty);
    public void Dispose() { _closed = true; _episodeMemberDraft.Clear(); ViewModel.Dispose(); }
}
