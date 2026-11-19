using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using TraceCommons.Interop.Shell;
using Windows.UI.ViewManagement;

namespace TraceCommons.App;

/// <summary>
/// The two system settings Home, the tray and the taskbar draw from, read
/// once and again only when the system says a setting changed: the
/// "Animation effects" switch and whether the taskbar draws itself light.
/// The system broadcasts WM_SETTINGCHANGE (and WM_THEMECHANGED) to every
/// top-level window, so the main window's procedure, subclassed as
/// <see cref="WindowMinimumSize"/> does, refreshes both and raises
/// <see cref="Changed"/>. Nothing polls (checklist P1, O3): the review found
/// Home's glyph clock probing every 0.5 s with animations off, and the tray
/// and taskbar re-reading the registry on every frame of a credit arrival.
/// </summary>
/// <remarks>
/// The W5 look track brings its own watch on the same message
/// (<c>Look/SystemMotionWatch</c>); once it is on main, the animation half of
/// this one moves onto it.
/// </remarks>
internal static class SystemSettingWatch
{
    private const uint WM_SETTINGCHANGE = 0x001A;
    private const uint WM_THEMECHANGED = 0x031A;
    private static readonly UISettings Ui = new();
    private static readonly CachedReading<bool> Animations = new(() => Ui.AnimationsEnabled);
    private static readonly CachedReading<bool> Taskbar = new(TrayIcon.ReadTaskbarLight);
    private static readonly SubclassProc Proc = WndProc;
    private static readonly HashSet<IntPtr> Installed = new();

    private delegate IntPtr SubclassProc(IntPtr hwnd, uint msg, IntPtr wParam, IntPtr lParam, UIntPtr id, UIntPtr data);

    [DllImport("comctl32.dll", SetLastError = true)]
    private static extern bool SetWindowSubclass(IntPtr hwnd, SubclassProc proc, UIntPtr id, UIntPtr data);

    [DllImport("comctl32.dll")]
    private static extern IntPtr DefSubclassProc(IntPtr hwnd, uint msg, IntPtr wParam, IntPtr lParam);

    /// <summary>The system lets things move.</summary>
    public static bool AnimationsEnabled => Animations.Value;

    /// <summary>The taskbar draws itself light.</summary>
    public static bool TaskbarLight => Taskbar.Value;

    /// <summary>Raised on the UI thread when either setting changed.</summary>
    public static event Action? Changed;

    /// <summary>Watches <paramref name="hwnd"/>'s setting changes; once per window.</summary>
    public static void Install(IntPtr hwnd)
    {
        if (hwnd != IntPtr.Zero && Installed.Add(hwnd))
        {
            // Id 3: WindowMinimumSize holds id 1, the W5 look track's watch id 2.
            SetWindowSubclass(hwnd, Proc, (UIntPtr)3, UIntPtr.Zero);
        }
    }

    /// <summary>Reads both settings again, as the window is activated; raises <see cref="Changed"/> if either moved.</summary>
    public static void Refresh()
    {
        bool animations = Animations.Refresh();
        bool taskbar = Taskbar.Refresh();
        if (animations || taskbar)
        {
            Changed?.Invoke();
        }
    }

    private static IntPtr WndProc(IntPtr hwnd, uint msg, IntPtr wParam, IntPtr lParam, UIntPtr id, UIntPtr data)
    {
        if (msg is WM_SETTINGCHANGE or WM_THEMECHANGED)
        {
            // Any setting: the wParam a sender passes is not always the SPI
            // it changed, and reading two values again is cheap next to a
            // message the system sends only on a change.
            Refresh();
        }

        return DefSubclassProc(hwnd, msg, wParam, lParam);
    }
}
