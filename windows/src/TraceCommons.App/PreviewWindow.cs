using System;
using Microsoft.UI.Xaml;
using TraceCommons.App.Controls;
using TraceCommons.App.ViewModels;

namespace TraceCommons.App;

/// <summary>
/// The window that holds the preview sheet.
///
/// It has no XAML of its own: <see cref="PreviewSheet"/> carries the whole
/// sheet, and this exists only to put it in a window and to free it when that
/// window closes. The split is not cosmetic -- the sheet's transcript panel
/// uses <c>x:Load</c>, whose generated code calls <c>FindName</c> on the root
/// element, and a WinUI <c>Window</c> is not a <c>FrameworkElement</c> and has
/// none. The read gate depends on that deferral, so the markup lives in a
/// control.
/// </summary>
public sealed class PreviewWindow : Window
{
    private readonly PreviewSheet _sheet;

    public PreviewWindow(DaemonHost host, QueueEntryViewModel entry)
    {
        Title = "Look inside";

        _sheet = new PreviewSheet(host, entry);
        _sheet.Decided += OnDecided;
        _sheet.CloseRequested += Close;
        Content = _sheet;

        Closed += OnClosed;
    }

    /// <summary>
    /// Raised once the contributor has decided, forwarded from the sheet. The
    /// queue window owns the undo, because recovery has to be on the screen
    /// they land on rather than behind a sheet that has closed.
    /// </summary>
    public event Action<QueueEntryViewModel, PreviewDecision>? Decided;

    private void OnDecided(QueueEntryViewModel entry, PreviewDecision decision) =>
        Decided?.Invoke(entry, decision);

    /// <summary>
    /// Frees the preview with the window, which is what bounds the ABI's one
    /// content exemption to a sheet that is open.
    /// </summary>
    private void OnClosed(object sender, WindowEventArgs args)
    {
        _sheet.Decided -= OnDecided;
        _sheet.CloseRequested -= Close;
        _sheet.Dispose();
    }
}
