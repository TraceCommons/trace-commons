using System;
using System.Linq;
using System.Threading.Tasks;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using TraceCommons.App.ViewModels;
using TraceCommons.Interop;
using Windows.Storage.Pickers;

namespace TraceCommons.App.Controls;

public sealed partial class InsightsView : UserControl, IDisposable
{
    private readonly IntPtr _window;
    private string? _file;
    private readonly EpisodeMemberDraftBinding _episodeMemberDraft = new();
    private bool _closed;
    public InsightsViewModel ViewModel { get; }
    public event EventHandler? ContributionSetupRequested;

    /// <param name="viewModel">
    /// The tabs' state; a sample window passes one over the refusing sample
    /// service and the Mac's sample benchmarks.
    /// </param>
    public InsightsView(IntPtr window, InsightsViewModel? viewModel = null)
    {
        ViewModel = viewModel ?? new InsightsViewModel();
        InitializeComponent();
        _window = window;
        DataContext = ViewModel;
        _segments = new[] { Segment0, Segment1, Segment2, Segment3, Segment4 };
        _pages = new FrameworkElement[] { Page0, Page1, Page2, Page3, Page4 };
        ShowPage(0);
        Unloaded += (_, _) => { _episodeMemberDraft.Clear(); ViewModel.Cancel(); };
    }
    public async Task ActivateAsync()
    {
        await ViewModel.LoadAsync();
        _activated = true;
        await ViewModel.SelectTabAsync(_tab);
    }
    // Set once the core's words are loaded. The first tab is shown while the
    // control is built, before then, and that read would present with no
    // words; ActivateAsync reads the tab on screen instead.
    private bool _activated;
    private readonly ToggleButton[] _segments;
    private readonly FrameworkElement[] _pages;
    private int _tab;
    // A segment shows its page. Pressing the one already shown keeps it
    // checked: exactly one tab is always the selected one, as on the Mac
    // (Navigation.swift:101-104 sets the selection, never clears it).
    private async void OnSegment(object sender, RoutedEventArgs args)
    {
        int tab = Array.IndexOf(_segments, sender);
        if (tab < 0) return;
        bool changed = tab != _tab;
        ShowPage(tab);
        // The tabs read their figures when shown; Analyze reads nothing new.
        if (changed && !_closed && _activated) await ViewModel.SelectTabAsync(_tab);
    }
    private void ShowPage(int tab)
    {
        _tab = tab;
        for (int i = 0; i < _pages.Length; i++)
        {
            bool shown = i == tab;
            _pages[i].Visibility = shown ? Visibility.Visible : Visibility.Collapsed;
            _segments[i].IsChecked = shown;
            // The Mac's label weight: semibold selected, medium otherwise
            // (Navigation.swift:109).
            _segments[i].FontWeight = shown ? Microsoft.UI.Text.FontWeights.SemiBold : Microsoft.UI.Text.FontWeights.Medium;
        }
    }
    // A week picker's own index follows the model; only a choice the model
    // is not already showing reads again.
    private async void OnOverviewWeek(object sender, SelectionChangedEventArgs args)
    {
        int index = OverviewWeek.SelectedIndex;
        if (!_closed && index >= 0 && index != ViewModel.Overview.SelectedWeekIndex)
            await ViewModel.Overview.SelectWeekAsync(index);
    }
    private async void OnPatternsWeek(object sender, SelectionChangedEventArgs args)
    {
        int index = PatternsWeek.SelectedIndex;
        if (!_closed && index >= 0 && index != ViewModel.Patterns.SelectedWeekIndex)
            await ViewModel.Patterns.SelectWeekAsync(index);
    }
    private async void OnSessionChoice(object sender, SelectionChangedEventArgs args)
    {
        int index = SessionChoice.SelectedIndex;
        if (!_closed && index >= 0 && index != ViewModel.Sessions.SelectedIndex)
            await ViewModel.Sessions.SelectAsync(index);
    }
    private async void OnOverviewCard(object sender, RoutedEventArgs args)
    {
        if (sender is Button { Tag: string card }) await ViewModel.Overview.ToggleInputsAsync(card);
    }
    private async void OnPatternSessions(object sender, RoutedEventArgs args)
    {
        if (sender is FrameworkElement { Tag: string pattern }) await ViewModel.Patterns.ToggleSessionsAsync(pattern);
    }
    private async void OnBenchmarksChoice(object sender, SelectionChangedEventArgs args)
    {
        if (_closed) return;
        await ViewModel.Benchmarks.ChooseAsync(BenchPeriod.SelectedIndex, BenchPopulation.SelectedIndex);
    }

    /// <summary>A share row read as one element, as <c>.accessibilityElement(children: .combine)</c> does.</summary>
    public static string ShareName(string task, string label) => task + ", " + label;

    public static string PickName(string title, string model, string figure, string next) =>
        string.Join(", ", title, model, figure, next);

    public static string RowName(string model, string traces, string kept, string cost, string time) =>
        string.Join(", ", model, traces, kept, cost, time);

    /// <summary>A share bar's fill as a fraction of its track, at least 4 wide (InsightsBenchmarksTab.swift:368-371).</summary>
    public static readonly DependencyProperty FractionProperty = DependencyProperty.RegisterAttached(
        "Fraction", typeof(double), typeof(InsightsView), new PropertyMetadata(0.0, OnFractionChanged));

    public static double GetFraction(DependencyObject element) => (double)element.GetValue(FractionProperty);

    public static void SetFraction(DependencyObject element, double value) => element.SetValue(FractionProperty, value);

    private static void OnFractionChanged(DependencyObject element, DependencyPropertyChangedEventArgs args)
    {
        if (element is not FrameworkElement bar || VisualTreeHelperParent(bar) is not FrameworkElement track) return;
        // One SizeChanged handler per track, however often the fraction
        // changes; it reads the fraction current when the track resizes.
        if (!Tracked.TryGetValue(track, out _))
        {
            Tracked.Add(track, bar);
            track.SizeChanged += (_, _) => SizeBar(bar, track);
        }

        SizeBar(bar, track);
    }

    /// <summary>The tracks whose bar already follows their width.</summary>
    private static readonly System.Runtime.CompilerServices.ConditionalWeakTable<FrameworkElement, FrameworkElement> Tracked = new();

    private static void SizeBar(FrameworkElement bar, FrameworkElement track) =>
        bar.Width = Math.Max(4, track.ActualWidth * GetFraction(bar));

    private static DependencyObject? VisualTreeHelperParent(FrameworkElement element) => element.Parent;

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
        // SelectionMode is Multiple (for episode-member picking), so
        // SelectedItem is implementation-defined once more than one row is
        // checked. SelectedItems is the well-defined collection; take its
        // first entry rather than an unspecified one.
        if (SavedList.SelectedItems.Cast<SavedInsight>().FirstOrDefault() is not { } selected) return;
        await ViewModel.ExplainAsync(selected.Id);
        BringSnapshotIntoView(selected.Id);
    }
    private string[] SelectedSnapshotIds() => SavedList.SelectedItems.Cast<SavedInsight>().Select(item => item.Id).ToArray();
    private async void OnLoadQuestionCards(object sender, RoutedEventArgs args) => await ViewModel.LoadQuestionCardsAsync(
        CardSavedList.SelectedItems.Cast<SavedInsight>().Select(item => item.Id),
        CardEpisodeList.SelectedItems.Cast<EpisodeRow>().Select(item => item.Id));
    private async void OnCardEvidence(object sender, RoutedEventArgs args)
    {
        if (sender is not Button { Tag: string id }) return;
        await ViewModel.OpenCardEvidenceAsync(id);
        BringSnapshotIntoView(id);
    }
    private async void OnCardEpisode(object sender, RoutedEventArgs args)
    {
        if (sender is not Button { Tag: string id }) return;
        await ViewModel.OpenCardEpisodeAsync(id);
        if (!_closed && ViewModel.CurrentEpisodeId == id) ReconcileEpisodeMemberDraft(id);
    }
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
        var dialog = new ContentDialog { XamlRoot = XamlRoot, Title = ViewModel["episode_clear_assessment_title"],
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
            XamlRoot = XamlRoot, Title = ViewModel["delete_title"], Content = ViewModel["delete_notice"],
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
