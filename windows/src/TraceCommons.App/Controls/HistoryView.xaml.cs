using System;
using System.Threading.Tasks;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using TraceCommons.App.ViewModels;
using TraceCommons.Interop;

namespace TraceCommons.App.Controls;

/// <summary>
/// The History screen's markup and wiring.
///
/// Thin by design, like <see cref="PreviewSheet"/>: every decision lives in
/// <see cref="HistoryViewModel"/>, and the part that is contract rather than
/// product -- what a withdrawal confirmation may honestly say -- lives one
/// layer further down in <see cref="WithdrawCopy"/>, so it is tested off
/// Windows. This file wires clicks and shows one dialog.
/// </summary>
public sealed partial class HistoryView : UserControl
{
    public HistoryView(DaemonHost host)
    {
        InitializeComponent();

        ViewModel = new HistoryViewModel(host);
        Loaded += OnFirstLoaded;
    }

    public HistoryViewModel ViewModel { get; }

    /// <summary>
    /// Reads history once the view is on screen rather than in the
    /// constructor. Three IPC calls back this screen and the first one runs
    /// before the pane has been laid out otherwise, which reads as the nav
    /// click having done nothing.
    /// </summary>
    private async void OnFirstLoaded(object sender, RoutedEventArgs e)
    {
        Loaded -= OnFirstLoaded;
        await ViewModel.RefreshAsync();
    }

    private async void OnCheckForUpdates(object sender, RoutedEventArgs e)
    {
        await ViewModel.CheckForUpdatesAsync();
    }

    /// <summary>
    /// Withdrawal, from a row.
    /// </summary>
    /// <remarks>
    /// The confirmation is shown BEFORE the call and is keyed on the record's
    /// local status, because that is all this machine has: the server computes
    /// <c>distribution_reach</c> during the withdrawal, from live export
    /// membership. <see cref="WithdrawCopy.Confirmation"/> decides what may
    /// honestly be said for each stage, and this file does not second-guess
    /// it. Afterwards the row reports the tier the server actually applied.
    ///
    /// Nothing is logged from this handler. The submission id is an account
    /// identifier and the outcome is a fact about someone's own contribution;
    /// neither belongs in a log line.
    /// </remarks>
    private async void OnWithdraw(object sender, RoutedEventArgs e)
    {
        if (RecordOf(sender) is not HistoryRecordViewModel record)
        {
            return;
        }

        if (!await WithdrawDialog.ConfirmAsync(XamlRoot, record.Stage))
        {
            return;
        }

        await ViewModel.WithdrawAsync(record);
    }

    /// <summary>
    /// Which record a click came from.
    /// </summary>
    /// <remarks>
    /// Tag first, DataContext second, matching the queue's own resolution.
    /// The pair is deliberate rather than defensive habit: the record a click
    /// refers to is the one thing on this card that must never be ambiguous,
    /// because acting on the wrong row here means deleting a contribution the
    /// contributor meant to keep.
    /// </remarks>
    private static HistoryRecordViewModel? RecordOf(object sender) =>
        sender is FrameworkElement element
            ? element.Tag as HistoryRecordViewModel ?? element.DataContext as HistoryRecordViewModel
            : null;
}
