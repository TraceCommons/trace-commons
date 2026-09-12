using System;
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
    private bool _closed;
    public InsightsViewModel ViewModel { get; } = new();
    public event EventHandler? ContributionSetupRequested;

    public InsightsView(IntPtr window)
    {
        InitializeComponent();
        _window = window;
        DataContext = ViewModel;
        Unloaded += (_, _) => ViewModel.Cancel();
    }
    public Task ActivateAsync() => ViewModel.LoadAsync();
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
    private async void OnRefresh(object sender, RoutedEventArgs args) => await ViewModel.RefreshAsync();
    private async void OnExplain(object sender, RoutedEventArgs args)
    {
        if (SavedList.SelectedItem is SavedInsight selected) await ViewModel.ExplainAsync(selected.Id);
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
    private async void OnAnnotate(object sender, RoutedEventArgs args) => await ViewModel.SaveAssessmentAsync();
    private async void OnClear(object sender, RoutedEventArgs args) => await ViewModel.ClearAnnotationAsync();
    private void OnCancel(object sender, RoutedEventArgs args) => ViewModel.Cancel();
    private void OnContributions(object sender, RoutedEventArgs args) => ContributionSetupRequested?.Invoke(this, EventArgs.Empty);
    public void Dispose() { _closed = true; ViewModel.Dispose(); }
}
