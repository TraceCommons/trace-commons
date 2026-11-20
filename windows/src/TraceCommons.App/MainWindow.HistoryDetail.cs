using System.Collections.Specialized;
using System.ComponentModel;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using TraceCommons.App.Controls;
using TraceCommons.App.ViewModels;
using TraceCommons.Interop;

namespace TraceCommons.App;

/// <summary>
/// W21: an opened History row's detail in the inspector, as the Mac draws
/// it while a History row is open (<c>HistoryInspectorPane</c>,
/// HistoryInspector.swift:124-131; <c>HistorySelection.opened</c>, :136-146):
/// only on History's page, and only while the opened row is still in the
/// list. Otherwise the inspector is what it was.
/// </summary>
/// <remarks>
/// The inspector's own markup is the W2 shell's. This slice adds one layer
/// over it from code, so the shell's XAML is untouched: the detail sits over
/// the folder's sessions while it is shown, on the inspector's own ground.
/// </remarks>
public sealed partial class MainWindow
{
    private ContentControl? _historyInspectorHost;
    private HistoryDetailInspector? _historyDetail;
    private string? _openedSubmission;

    /// <summary>The History screen, with its rows wired to open their detail.</summary>
    private HistoryView NewHistoryView()
    {
        var view = new HistoryView(_host);
        view.RecordOpened += OpenHistoryRecord;
        view.ViewModel.OpenFolderRecords.CollectionChanged += OnHistoryRecordsChanged;
        ViewModel.PropertyChanged += OnHistoryShellChanged;
        Activated += OnHistoryWindowActivated;
        return view;
    }

    /// <summary>
    /// Coming back to the window reads the shown detail again, as the Mac
    /// does when the app becomes active (SessionDetailView.swift:51-53).
    /// </summary>
    private async void OnHistoryWindowActivated(object sender, WindowActivatedEventArgs args)
    {
        if (args.WindowActivationState != WindowActivationState.Deactivated
            && _historyDetail is { } detail
            && _historyInspectorHost?.Visibility == Visibility.Visible)
        {
            await detail.LoadAsync();
        }
    }

    private void OpenHistoryRecord(HistoryRecordViewModel record)
    {
        EnsureHistoryInspectorHost();
        if (_openedSubmission != record.SubmissionId || _historyDetail is null)
        {
            _openedSubmission = record.SubmissionId;
            _historyDetail = new HistoryDetailInspector(_host, record, SentTraining);
            _historyInspectorHost!.Content = _historyDetail;
        }

        ShowHistoryDetail();
        OpenInspectorForFolder();
    }

    /// <summary>
    /// The inspector gets a host for the detail over its existing content,
    /// once. The existing content keeps its own bindings and visibility.
    /// </summary>
    private void EnsureHistoryInspectorHost()
    {
        if (_historyInspectorHost is not null)
        {
            return;
        }

        UIElement? existing = InspectorPane.Child;
        InspectorPane.Child = null;
        var layers = new Grid();
        if (existing is not null)
        {
            layers.Children.Add(existing);
        }

        _historyInspectorHost = new ContentControl
        {
            HorizontalContentAlignment = HorizontalAlignment.Stretch,
            VerticalContentAlignment = VerticalAlignment.Stretch,
            IsTabStop = false,
            Visibility = Visibility.Collapsed,
        };
        layers.Children.Add(_historyInspectorHost);
        InspectorPane.Child = layers;
    }

    private void ShowHistoryDetail()
    {
        if (_historyInspectorHost is null)
        {
            return;
        }

        bool shown = ViewModel.ShowingHistory && _historyDetail is not null;
        _historyInspectorHost.Visibility = shown ? Visibility.Visible : Visibility.Collapsed;
    }

    private void OnHistoryShellChanged(object? sender, PropertyChangedEventArgs e)
    {
        if (e.PropertyName == nameof(MainViewModel.ShowingHistory))
        {
            ShowHistoryDetail();
        }
    }

    /// <summary>
    /// The list was redrawn: the opened row follows its refreshed record, so
    /// its tag agrees with the row. Records are never dropped from this list
    /// (a withdrawn one stays, HistoryView.xaml), so the detail stays open.
    /// </summary>
    private void OnHistoryRecordsChanged(object? sender, NotifyCollectionChangedEventArgs e)
    {
        if (_historyDetail is not null
            && sender is System.Collections.Generic.IEnumerable<HistoryRecordViewModel> rows
            && HistoryInspection.Opened(_openedSubmission, onHistory: true, rows, row => row.SubmissionId) is { } row
            && !ReferenceEquals(row, _historyDetail.Record))
        {
            _historyDetail.Refresh(row);
        }
    }
}
