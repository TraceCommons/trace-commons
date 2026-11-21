using System;
using System.Runtime.InteropServices;
using Microsoft.UI.Xaml;
using TraceCommons.Interop.Shell;

namespace TraceCommons.App;

/// <summary>
/// The system title bar of a window other than the main one, in the theme
/// its content is drawn in, with the app's mark in place of the generic
/// icon of an executable that has none. The theme is the root's actual one,
/// so a change of the choice and a change of the system's theme both reach it.
/// </summary>
internal static class WindowCaption
{
    // Windows App SDK 1.6 has no title bar theme of its own.
    private const int DwmaUseImmersiveDarkMode = 20;
    private const uint WmSetIcon = 0x0080;
    private const int IconSmall = 0;
    private const int SmCxSmIcon = 49;

    public static void Follow(Window window, FrameworkElement root)
    {
        IntPtr hwnd = WinRT.Interop.WindowNative.GetWindowHandle(window);
        IntPtr icon = IntPtr.Zero;

        void Apply()
        {
            bool dark = root.ActualTheme == ElementTheme.Dark;
            int value = dark ? 1 : 0;
            DwmSetWindowAttribute(hwnd, DwmaUseImmersiveDarkMode, ref value, sizeof(int));

            int size = GetSystemMetricsForDpi(SmCxSmIcon, Math.Max(96u, GetDpiForWindow(hwnd)));
            size = size > 0 ? size : 16;
            IntPtr next = TrayIcon.IconFromPixels(TaskbarCredit.BaseIcon(size, dark).ToBgra(), size);
            if (next == IntPtr.Zero)
            {
                return;
            }

            SendMessage(hwnd, WmSetIcon, IconSmall, next);
            if (icon != IntPtr.Zero)
            {
                DestroyIcon(icon);
            }

            icon = next;
        }

        void OnThemeChanged(FrameworkElement sender, object args) => Apply();
        void OnLoaded(object sender, RoutedEventArgs args) => Apply();

        root.ActualThemeChanged += OnThemeChanged;
        root.Loaded += OnLoaded;
        window.Closed += (_, _) =>
        {
            root.ActualThemeChanged -= OnThemeChanged;
            root.Loaded -= OnLoaded;
            if (icon != IntPtr.Zero)
            {
                SendMessage(hwnd, WmSetIcon, IconSmall, IntPtr.Zero);
                DestroyIcon(icon);
                icon = IntPtr.Zero;
            }
        };

        Apply();
    }

    [DllImport("dwmapi.dll")]
    private static extern int DwmSetWindowAttribute(IntPtr hwnd, int attribute, ref int value, int size);

    [DllImport("user32.dll", EntryPoint = "SendMessageW")]
    private static extern IntPtr SendMessage(IntPtr hwnd, uint message, nint wParam, IntPtr lParam);

    [DllImport("user32.dll")]
    private static extern uint GetDpiForWindow(IntPtr hwnd);

    [DllImport("user32.dll")]
    private static extern int GetSystemMetricsForDpi(int index, uint dpi);

    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool DestroyIcon(IntPtr icon);
}
