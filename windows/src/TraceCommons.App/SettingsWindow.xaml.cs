using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
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
        view.Loaded += FocusFirstOnce;
    }

    /// <summary>
    /// Focus starts on the first control in reading order, at the top of the
    /// window, as an AppKit window's initial key view is its first. Left to
    /// itself WinUI put it on a button far down the page, off-screen and
    /// with no ring to show where it was (box, 2026-11-19). The first in Tab
    /// order is no better: Settings' Tab order is not its reading order, and
    /// its first stop is off-screen too (box, 2026-11-19), so the visual tree
    /// is walked in drawing order instead. Programmatic, so no ring is drawn
    /// until a key is pressed.
    /// </summary>
    private static void FocusFirstOnce(object sender, RoutedEventArgs e)
    {
        var view = (FrameworkElement)sender;
        view.Loaded -= FocusFirstOnce;
        if (FirstInReadingOrder(view) is Control first)
        {
            first.Focus(FocusState.Programmatic);
        }
    }

    private static Control? FirstInReadingOrder(DependencyObject root)
    {
        int count = Microsoft.UI.Xaml.Media.VisualTreeHelper.GetChildrenCount(root);
        for (int i = 0; i < count; i++)
        {
            DependencyObject child = Microsoft.UI.Xaml.Media.VisualTreeHelper.GetChild(root, i);
            if (child is UIElement { Visibility: not Visibility.Visible })
            {
                continue;
            }

            if (child is Control { IsTabStop: true, IsEnabled: true, ActualHeight: > 0 } control)
            {
                return control;
            }

            if (FirstInReadingOrder(child) is Control found)
            {
                return found;
            }
        }

        return null;
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
