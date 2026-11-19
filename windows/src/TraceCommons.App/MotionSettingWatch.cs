using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using TraceCommons.Interop.Shell;

namespace TraceCommons.App;

/// <summary>
/// Raises <see cref="Changed"/> when the system's motion setting may have
/// changed, from the window's own WM_SETTINGCHANGE (comctl32
/// SetWindowSubclass), so a clock stopped by motion going off starts again
/// when it comes back on (<see cref="MotionSettingMessage"/>). Runs on the
/// window's thread.
/// </summary>
internal static class MotionSettingWatch
{
    private const nuint SubclassId = 0x5733;
    private static readonly SubclassProc Proc = WndProc;
    private static readonly HashSet<IntPtr> Installed = new();

    private delegate IntPtr SubclassProc(IntPtr hwnd, uint msg, IntPtr wParam, IntPtr lParam, UIntPtr id, UIntPtr data);

    /// <summary>The motion setting may have changed; read it again.</summary>
    public static event Action? Changed;

    [DllImport("comctl32.dll", SetLastError = true)]
    private static extern bool SetWindowSubclass(IntPtr hwnd, SubclassProc proc, UIntPtr id, UIntPtr data);

    [DllImport("comctl32.dll")]
    private static extern IntPtr DefSubclassProc(IntPtr hwnd, uint msg, IntPtr wParam, IntPtr lParam);

    public static void Install(IntPtr hwnd)
    {
        if (hwnd != IntPtr.Zero && Installed.Add(hwnd))
        {
            SetWindowSubclass(hwnd, Proc, (UIntPtr)SubclassId, UIntPtr.Zero);
        }
    }

    private static IntPtr WndProc(IntPtr hwnd, uint msg, IntPtr wParam, IntPtr lParam, UIntPtr id, UIntPtr data)
    {
        if (MotionSettingMessage.MayChangeMotion(msg, (ulong)(nuint)wParam))
        {
            Changed?.Invoke();
        }

        return DefSubclassProc(hwnd, msg, wParam, lParam);
    }
}
