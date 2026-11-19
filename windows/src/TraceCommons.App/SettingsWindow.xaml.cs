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
        Title = MonitorWords.SettingsTitle;
        SettingsHost.Content = view;
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
            int width = System.Math.Min(work.Width, MonitorPaneLayout.ToPhysical(DesignWidth, scale));
            int height = System.Math.Min(work.Height, MonitorPaneLayout.ToPhysical(DesignHeight, scale));
            int x = work.X + (work.Width - width) / 2;
            int y = work.Y + (work.Height - height) / 2;
            AppWindow.MoveAndResize(new RectInt32(x, y, width, height));
        }

        AppWindow.Show();
        Activate();
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
