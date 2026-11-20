using System;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using TraceCommons.Interop.Shell;
using Windows.Graphics;
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
        // Settings > General's appearance, as on every window (WindowAppearance).
        WindowAppearance.Follow(this, _sheet);
        WindowClocks.Watch(this);
        FitToSheet(null);

        Closed += OnClosed;
    }

    /// <summary>The Mac's sheet measure, 760 by 620 (PreviewSheet.swift:729-731).</summary>
    private const double SheetWidth = 760, SheetHeight = 620;

    /// <summary>
    /// Opens at the Mac's sheet measure, inside the work area and centred in
    /// it. The system's default size (754 by 512 on a 1024 by 768 screen)
    /// was under the sheet's own floor, so its action row sat below the
    /// window and Tab reached Not this one and Close where nothing could be
    /// seen (box tab walk, 2026-11-19).
    /// </summary>
    /// <summary>Places the window on the monitor of <paramref name="owner"/>, as About is.</summary>
    public void PlaceNear(AppWindow owner) => FitToSheet(owner);

    private void FitToSheet(AppWindow? owner)
    {
        DisplayArea display = DisplayArea.GetFromWindowId((owner ?? AppWindow).Id, DisplayAreaFallback.Nearest);
        RectInt32 work = display.WorkArea;
        double scale = MonitorScale.For(AppWindow);
        AppWindow.ResizeClient(new SizeInt32(
            Math.Min(work.Width, MonitorPaneLayout.ToPhysical(SheetWidth, scale)),
            Math.Min(work.Height, MonitorPaneLayout.ToPhysical(SheetHeight, scale))));

        // The frame adds to the client: take what overhangs the work area
        // off the client again.
        SizeInt32 outer = AppWindow.Size;
        if (outer.Height > work.Height || outer.Width > work.Width)
        {
            SizeInt32 client = AppWindow.ClientSize;
            AppWindow.ResizeClient(new SizeInt32(
                Math.Max(1, client.Width - Math.Max(0, outer.Width - work.Width)),
                Math.Max(1, client.Height - Math.Max(0, outer.Height - work.Height))));
            outer = AppWindow.Size;
        }

        AppWindow.Move(new PointInt32(
            work.X + Math.Max(0, (work.Width - outer.Width) / 2),
            work.Y + Math.Max(0, (work.Height - outer.Height) / 2)));
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
