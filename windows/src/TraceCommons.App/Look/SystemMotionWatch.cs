using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;

namespace TraceCommons.App.Look;

/// <summary>
/// Follows the system's "Animation effects" switch live through the
/// window's own WM_SETTINGCHANGE. On the test box (Windows Server 2025,
/// 2026-11-19) turning the switch off while the window kept the foreground
/// raised no <c>UISettings.AnimationsEnabledChanged</c>: the ember went on
/// gliding until the window was activated again. The system broadcasts
/// WM_SETTINGCHANGE with SPI_SETCLIENTAREAANIMATION to every top-level
/// window, so the main window's procedure (subclassed, as
/// <c>WindowMinimumSize</c> does) asks <see cref="GlassMotionSettings"/> to
/// read the setting again.
/// </summary>
internal static class SystemMotionWatch
{
    private const uint WM_SETTINGCHANGE = 0x001A;
    private static readonly SubclassProc Proc = WndProc;
    private static readonly HashSet<IntPtr> Installed = new();

    private delegate IntPtr SubclassProc(IntPtr hwnd, uint msg, IntPtr wParam, IntPtr lParam, UIntPtr id, UIntPtr data);

    [DllImport("comctl32.dll", SetLastError = true)]
    private static extern bool SetWindowSubclass(IntPtr hwnd, SubclassProc proc, UIntPtr id, UIntPtr data);

    [DllImport("comctl32.dll")]
    private static extern IntPtr DefSubclassProc(IntPtr hwnd, uint msg, IntPtr wParam, IntPtr lParam);

    /// <summary>Watches <paramref name="hwnd"/>'s setting changes; once per window.</summary>
    public static void Install(IntPtr hwnd)
    {
        if (hwnd != IntPtr.Zero && Installed.Add(hwnd))
        {
            // Id 2: WindowMinimumSize holds id 1 on the same window.
            SetWindowSubclass(hwnd, Proc, (UIntPtr)2, UIntPtr.Zero);
        }
    }

    private static IntPtr WndProc(IntPtr hwnd, uint msg, IntPtr wParam, IntPtr lParam, UIntPtr id, UIntPtr data)
    {
        if (msg == WM_SETTINGCHANGE)
        {
            // Any setting: reading the one value again is cheap, and the
            // wParam a sender passes is not always the SPI it changed.
            GlassMotionSettings.Refresh();
        }

        return DefSubclassProc(hwnd, msg, wParam, lParam);
    }
}
