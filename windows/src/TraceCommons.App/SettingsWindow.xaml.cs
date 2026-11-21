using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using TraceCommons.App.Controls;
using TraceCommons.Interop.Shell;
using Windows.Graphics;

namespace TraceCommons.App;

/// <summary>
/// The Settings window (spec W2-D2 (a)). The main window keeps it by
/// reference, never finds it by title (checklist M1), and opens it on the
/// monitor the main window is on.
/// </summary>
public sealed partial class SettingsWindow : Window
{
    /// <summary><c>GlassTokens.Size.modalWidth</c> (GlassTokens.swift:596).</summary>
    public const double DesignWidth = 780;

    /// <summary>TraceCommonsAppMain.swift:167.</summary>
    public const double DesignHeight = 640;

    private bool _placed;

    public SettingsWindow(SettingsView view)
    {
        InitializeComponent();
        WindowClocks.Watch(this);
        Title = WindowTitles.Settings;
        SettingsHost.Content = view;

        // Settings > General's appearance, here as on the Monitor window.
        WindowAppearance.Follow(this, SettingsRoot);

        // Escape closes the window, as the Mac's onExitCommand does
        // (SettingsModal.swift:57).
        var escape = new Microsoft.UI.Xaml.Input.KeyboardAccelerator { Key = Windows.System.VirtualKey.Escape };
        escape.Invoked += (_, args) =>
        {
            args.Handled = true;
            Close();
        };
        SettingsRoot.KeyboardAccelerators.Add(escape);
    }

    /// <summary>
    /// Shows the window, the first time sized 780 by 640 at the monitor's
    /// scale and held to the work area of the monitor <paramref name="owner"/>
    /// is on, before it is first shown.
    /// </summary>
    public void ShowNearOwner(AppWindow owner)
    {
        if (!_placed)
        {
            _placed = true;
            DisplayArea display = DisplayArea.GetFromWindowId(owner.Id, DisplayAreaFallback.Nearest);
            RectInt32 work = display.WorkArea;
            double scale = MonitorScale.For(owner);
            // 780 by 640 is the Mac's content size, so it is the client
            // area's here: sizing the outer frame left a 766 by 633 client
            // (W2 box, 2026-11-19).
            int width = System.Math.Min(work.Width, MonitorPaneLayout.ToPhysical(DesignWidth, scale));
            int height = System.Math.Min(work.Height, MonitorPaneLayout.ToPhysical(DesignHeight, scale));
            AppWindow.ResizeClient(new SizeInt32(width, height));
            SizeInt32 outer = AppWindow.Size;
            int x = work.X + System.Math.Max(0, (work.Width - outer.Width) / 2);
            int y = work.Y + System.Math.Max(0, (work.Height - outer.Height) / 2);
            AppWindow.Move(new PointInt32(x, y));
        }

        AppWindow.Show();
        Activate();

        // Focus the first control, not the bare input site: a nameless
        // first stop that Escape never reached (W25 audit, box az9).
        ToolWindows.FocusFirst(SettingsRoot, FocusState.Programmatic, () => AppWindow.IsVisible);
    }
}

/// <summary>A window's monitor scale (its DPI over 96), from Win32.</summary>
internal static class MonitorScale
{
    [System.Runtime.InteropServices.DllImport("user32.dll")]
    private static extern uint GetDpiForWindow(System.IntPtr hwnd);

    public static double For(AppWindow window)
    {
        System.IntPtr hwnd = Microsoft.UI.Win32Interop.GetWindowFromWindowId(window.Id);
        uint dpi = GetDpiForWindow(hwnd);
        return dpi == 0 ? 1.0 : dpi / 96.0;
    }
}
