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

    /// <remarks>
    /// <paramref name="liveEntry"/> is REQUIRED AND HAS NO DEFAULT. Making it
    /// optional would mean a caller that forgot it silently got the stale
    /// behaviour back -- the exact defect this parameter exists to remove,
    /// re-armed as a default, with no test failure and no warning to whoever
    /// added the call site. Required, it fails at compile time where the
    /// mistake is. A caller that genuinely has no queue to resolve against
    /// passes <c>liveEntry: null</c> and says why, so choosing it is visible
    /// in the code and indistinguishable from nothing.
    /// </remarks>
    public PreviewWindow(
        DaemonHost host,
        QueueEntryViewModel entry,
        Func<string, QueueEntryViewModel?>? liveEntry)
    {
        Title = "Look inside";

        _sheet = new PreviewSheet(host, entry, liveEntry);
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

    /// <summary>
    /// The queue changed underneath this window. Forwarded to the sheet,
    /// whose gate is computed from the live entry rather than the copy it
    /// opened with. See <c>PreviewSheetViewModel.LiveEntry</c>.
    /// </summary>
    public void QueueChanged() => _sheet.ViewModel.QueueChanged();

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
