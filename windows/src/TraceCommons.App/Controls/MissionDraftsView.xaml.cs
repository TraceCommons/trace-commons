using System;
using System.ComponentModel;
using System.Threading.Tasks;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using TraceCommons.App.ViewModels;
using TraceCommons.Interop;
using Windows.Storage.Pickers;

namespace TraceCommons.App.Controls;

public sealed partial class MissionDraftsView : UserControl, IDisposable, INotifyPropertyChanged
{
    private readonly IntPtr _window;
    private string? _file;
    private string _selectedFileName = string.Empty;
    private long _generation;
    private bool _closed;
    public MissionDraftsViewModel ViewModel { get; } = new();
    public event PropertyChangedEventHandler? PropertyChanged;
    public string SelectedFileName { get => _selectedFileName; private set { _selectedFileName = value; PropertyChanged?.Invoke(this, new(nameof(SelectedFileName))); } }
    public bool CanImport => _file != null;

    public MissionDraftsView(IntPtr window)
    {
        InitializeComponent(); _window = window; DataContext = ViewModel;
        Unloaded += (_, _) => ViewModel.Deactivate();
    }
    public Task ActivateAsync() { _generation++; return ViewModel.LoadAsync(); }
    public void Deactivate() { _generation++; ViewModel.Deactivate(); }

    private async void OnChoose(object sender, RoutedEventArgs args)
    {
        try
        {
            long ticket = _generation;
            var picker = new FileOpenPicker();
            WinRT.Interop.InitializeWithWindow.Initialize(picker, _window);
            picker.FileTypeFilter.Add(".json");
            var file = await picker.PickSingleFileAsync();
            if (_closed || ticket != _generation || file == null) return;
            _file = file.Path; SelectedFileName = file.Name;
            PropertyChanged?.Invoke(this, new(nameof(CanImport))); ViewModel.ReportFileSelected();
        }
        catch (Exception) { ViewModel.ReportError(); }
    }
    private async void OnImport(object sender, RoutedEventArgs args) { if (_file != null) await ViewModel.ImportAsync(_file); }
    private async void OnRefresh(object sender, RoutedEventArgs args) => await ViewModel.RefreshAsync();
    private async void OnShow(object sender, RoutedEventArgs args) { if (DraftList.SelectedItem is MissionDraftSummary draft) await ViewModel.ShowAsync(draft.Id); }
    private async void OnDelete(object sender, RoutedEventArgs args)
    {
        if (DraftList.SelectedItem is not MissionDraftSummary draft) return;
        long ticket = _generation;
        var dialog = new ContentDialog { XamlRoot = XamlRoot, Title = ViewModel["delete_confirm_title"], Content = ViewModel["delete_confirm"], PrimaryButtonText = ViewModel["delete"], CloseButtonText = ViewModel["cancel"], DefaultButton = ContentDialogButton.Close };
        if (await dialog.ShowAsync() == ContentDialogResult.Primary && !_closed && ticket == _generation) await ViewModel.DeleteAsync(draft.Id);
    }
    public void Dispose() { if (_closed) return; _closed = true; _generation++; ViewModel.Dispose(); }
}
