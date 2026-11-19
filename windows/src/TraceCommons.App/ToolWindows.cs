using System;
using System.Runtime.InteropServices;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Windows.Graphics;

namespace TraceCommons.App;

/// <summary>
/// What the tray flyout and the edge rail share (W13, one-pass checklist M1
/// to M3): a borderless, topmost tool window kept out of Alt-Tab and the
/// taskbar, shown without taking activation, placed in physical pixels on the
/// monitor it belongs to before it is first shown, and found by its handle,
/// never by its title.
/// </summary>
internal static class ToolWindows
{
    /// <summary>Makes <paramref name="window"/> a tool window: no frame, always on top, not in the switchers, never activated by showing.</summary>
    public static IntPtr Prepare(Window window)
    {
        AppWindow app = window.AppWindow;
        var presenter = OverlappedPresenter.Create();
        presenter.SetBorderAndTitleBar(false, false);
        presenter.IsResizable = false;
        presenter.IsMaximizable = false;
        presenter.IsMinimizable = false;
        presenter.IsAlwaysOnTop = true;
        app.SetPresenter(presenter);
        app.IsShownInSwitchers = false;

        IntPtr hwnd = WinRT.Interop.WindowNative.GetWindowHandle(window);
        long style = GetWindowLongPtr(hwnd, GWL_EXSTYLE).ToInt64();
        SetWindowLongPtr(hwnd, GWL_EXSTYLE, new IntPtr(style | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST));
        return hwnd;
    }

    /// <summary>Moves and sizes in physical pixels, then shows without activating.</summary>
    public static void PlaceAndShow(Window window, (int X, int Y, int Width, int Height) frame)
    {
        window.AppWindow.MoveAndResize(new RectInt32(frame.X, frame.Y, frame.Width, frame.Height));
        window.AppWindow.Show(false);
    }

    /// <summary>The DPI scale of the monitor that holds the point (physical pixels), and its bounds and work area.</summary>
    public static (double Scale, (int X, int Y, int Width, int Height) Bounds, (int X, int Y, int Width, int Height) Work) MonitorAt(int x, int y)
    {
        DisplayArea area = DisplayArea.GetFromPoint(new PointInt32(x, y), DisplayAreaFallback.Nearest);
        IntPtr monitor = MonitorFromPoint(new POINT { X = x, Y = y }, MONITOR_DEFAULTTONEAREST);
        double scale = GetDpiForMonitor(monitor, 0, out uint dpiX, out _) == 0 && dpiX > 0 ? dpiX / 96.0 : 1.0;
        RectInt32 o = area.OuterBounds, w = area.WorkArea;
        return (scale, (o.X, o.Y, o.Width, o.Height), (w.X, w.Y, w.Width, w.Height));
    }

    /// <summary>The primary monitor's: the edge rail's screen, as the Mac's is its first screen (EdgeRailController.swift:104).</summary>
    public static (double Scale, (int X, int Y, int Width, int Height) Work) Primary()
    {
        DisplayArea area = DisplayArea.Primary;
        RectInt32 w = area.WorkArea;
        (double scale, _, _) = MonitorAt(w.X + w.Width / 2, w.Y + w.Height / 2);
        return (scale, (w.X, w.Y, w.Width, w.Height));
    }

    /// <summary>The cursor, in physical screen pixels: where a tray click was.</summary>
    public static (int X, int Y) Cursor()
    {
        GetCursorPos(out POINT p);
        return (p.X, p.Y);
    }

    /// <summary>The window that has the keyboard now, to give it back later.</summary>
    public static IntPtr Foreground() => GetForegroundWindow();

    /// <summary>
    /// Gives the keyboard back to <paramref name="hwnd"/> only if it is still
    /// there, visible and not minimised; never restores, raises past its
    /// place or switches desktops (checklist M2).
    /// </summary>
    public static void ReturnFocus(IntPtr hwnd)
    {
        bool exists = hwnd != IntPtr.Zero && IsWindow(hwnd);
        if (Interop.Shell.FocusReturn.ShouldReturn(exists, exists && IsWindowVisible(hwnd), exists && IsIconic(hwnd), sameDesktop: true))
        {
            SetForegroundWindow(hwnd);
        }
    }

    /// <summary>Takes the keyboard after a click inside: the window may now be activated.</summary>
    public static void TakeKeyboard(IntPtr hwnd)
    {
        long style = GetWindowLongPtr(hwnd, GWL_EXSTYLE).ToInt64();
        SetWindowLongPtr(hwnd, GWL_EXSTYLE, new IntPtr(style & ~WS_EX_NOACTIVATE));
        SetForegroundWindow(hwnd);
    }

    /// <summary>Gives up the keyboard as the close starts: showing never activates it again.</summary>
    public static void ReleaseKeyboard(IntPtr hwnd)
    {
        long style = GetWindowLongPtr(hwnd, GWL_EXSTYLE).ToInt64();
        SetWindowLongPtr(hwnd, GWL_EXSTYLE, new IntPtr(style | WS_EX_NOACTIVATE));
    }

    private const int GWL_EXSTYLE = -20;
    private const long WS_EX_TOOLWINDOW = 0x00000080;
    private const long WS_EX_TOPMOST = 0x00000008;
    private const long WS_EX_NOACTIVATE = 0x08000000;
    private const uint MONITOR_DEFAULTTONEAREST = 2;

    [StructLayout(LayoutKind.Sequential)]
    private struct POINT
    {
        public int X;
        public int Y;
    }

    [DllImport("user32.dll", EntryPoint = "GetWindowLongPtrW")]
    private static extern IntPtr GetWindowLongPtr(IntPtr hwnd, int index);

    [DllImport("user32.dll", EntryPoint = "SetWindowLongPtrW")]
    private static extern IntPtr SetWindowLongPtr(IntPtr hwnd, int index, IntPtr value);

    [DllImport("user32.dll")]
    private static extern IntPtr MonitorFromPoint(POINT point, uint flags);

    [DllImport("shcore.dll")]
    private static extern int GetDpiForMonitor(IntPtr monitor, int type, out uint dpiX, out uint dpiY);

    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool GetCursorPos(out POINT point);

    [DllImport("user32.dll")]
    private static extern IntPtr GetForegroundWindow();

    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool SetForegroundWindow(IntPtr hwnd);

    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool IsWindow(IntPtr hwnd);

    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool IsWindowVisible(IntPtr hwnd);

    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool IsIconic(IntPtr hwnd);
}
