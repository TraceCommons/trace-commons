using System;
using System.Runtime.InteropServices;
using TraceCommons.Interop.Shell;

namespace TraceCommons.App;

/// <summary>
/// The credit arrival on the taskbar button (owner decision W12-D4): the
/// Mac's Dock bounce as one <c>FlashWindowEx</c>, and its Dock coin burst
/// (<c>GlassDockCoinBurst</c>) drawn frame by frame into the window's own
/// taskbar icon, then the icon put back.
/// </summary>
/// <remarks>
/// <para>
/// The surface is the spike's to settle (W12): the window's icon
/// (<c>WM_SETICON</c>, here) animates the button itself, as the Dock tile
/// does; an overlay icon (<c>ITaskbarList3.SetOverlayIcon</c>) would draw
/// only a 16-pixel badge in its corner; a thumbnail toolbar shows only on
/// hover. This class is the thin adapter: the frames are
/// <see cref="DockCoinBurst.Render"/>'s, tested off Windows.
/// </para>
/// <para>
/// Nothing here sends, approves or shows a word: a credit arrival is not a
/// notice (W12-D4's note on the copy guide's rule 9).
/// </para>
/// </remarks>
internal sealed class TaskbarCredit
{
    private readonly IntPtr _hwnd;
    private IntPtr _original;
    private bool _swapped;
    private IntPtr _frame;

    public TaskbarCredit(IntPtr hwnd)
    {
        _hwnd = hwnd;
    }

    /// <summary>
    /// The Mac's single informational bounce (<c>requestUserAttention(.informationalRequest)</c>,
    /// DockCreditAnimator.swift:39): the taskbar button flashes once. The
    /// caller asks only while the window is not the active one.
    /// </summary>
    public void FlashOnce()
    {
        if (_hwnd == IntPtr.Zero || GetForegroundWindow() == _hwnd)
        {
            return;
        }

        var info = new FLASHWINFO
        {
            cbSize = (uint)Marshal.SizeOf<FLASHWINFO>(),
            hwnd = _hwnd,
            dwFlags = FLASHW_TRAY,
            uCount = 1,
            dwTimeout = 0,
        };
        FlashWindowEx(ref info);
    }

    /// <summary>The burst <paramref name="elapsed"/> seconds in, or the window's own icon back for null.</summary>
    public void ShowFrame(double? elapsed, bool still)
    {
        if (_hwnd == IntPtr.Zero)
        {
            return;
        }

        if (elapsed is not double t)
        {
            Restore();
            return;
        }

        if (!_swapped)
        {
            _original = SendMessage(_hwnd, WM_GETICON, (IntPtr)ICON_BIG, IntPtr.Zero);
            _swapped = true;
        }

        uint dpi = GetDpiForWindow(_hwnd);
        int size = GetSystemMetricsForDpi(SM_CXICON, Math.Max(96u, dpi));
        if (size <= 0)
        {
            size = 32;
        }

        bool dark = !TrayIcon.IsTaskbarLight();
        PixelCanvas canvas = DockCoinBurst.Render(px => BaseIcon(px, dark), size, t, flash: !still);
        IntPtr icon = TrayIcon.IconFromPixels(canvas.ToBgra(), size);
        if (icon == IntPtr.Zero)
        {
            return;
        }

        SendMessage(_hwnd, WM_SETICON, (IntPtr)ICON_BIG, icon);
        if (_frame != IntPtr.Zero)
        {
            DestroyIcon(_frame);
        }

        _frame = icon;
    }

    /// <summary>
    /// The icon the burst is drawn over: the NEAR AI N in the logo kit's ink
    /// for the taskbar (G-3 (a)), inset as the Dock draws an app icon inside
    /// its tile.
    /// </summary>
    private static PixelCanvas BaseIcon(int px, bool darkTaskbar)
    {
        var canvas = new PixelCanvas(px, px);
        double inset = px * (TrayStrip.Height - TrayStrip.MarkSize) / 2 / TrayStrip.Height;
        canvas.Fill(TrayStrip.MarkShape(inset, inset, px - 2 * inset), TrayStrip.Inks.For(darkTaskbar).Mark);
        return canvas;
    }

    private void Restore()
    {
        if (!_swapped)
        {
            return;
        }

        SendMessage(_hwnd, WM_SETICON, (IntPtr)ICON_BIG, _original);
        _swapped = false;
        if (_frame != IntPtr.Zero)
        {
            DestroyIcon(_frame);
            _frame = IntPtr.Zero;
        }
    }

    private const uint WM_GETICON = 0x007F;
    private const uint WM_SETICON = 0x0080;
    private const int ICON_BIG = 1;
    private const int SM_CXICON = 11;
    private const uint FLASHW_TRAY = 0x00000002;

    [StructLayout(LayoutKind.Sequential)]
    private struct FLASHWINFO
    {
        public uint cbSize;
        public IntPtr hwnd;
        public uint dwFlags;
        public uint uCount;
        public uint dwTimeout;
    }

    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool FlashWindowEx(ref FLASHWINFO info);

    [DllImport("user32.dll")]
    private static extern IntPtr GetForegroundWindow();

    [DllImport("user32.dll", EntryPoint = "SendMessageW")]
    private static extern IntPtr SendMessage(IntPtr hwnd, uint message, IntPtr wParam, IntPtr lParam);

    [DllImport("user32.dll")]
    private static extern uint GetDpiForWindow(IntPtr hwnd);

    [DllImport("user32.dll")]
    private static extern int GetSystemMetricsForDpi(int index, uint dpi);

    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool DestroyIcon(IntPtr icon);
}
