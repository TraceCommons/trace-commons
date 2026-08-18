using System;
using System.Diagnostics;
using System.Runtime.InteropServices;
using Microsoft.Win32;
using TraceCommons.Interop;

namespace TraceCommons.App;

/// <summary>
/// The notification-area presence: the mark in the tray, a tooltip that says
/// what is owed, a small menu, and the 4-hour digest balloon.
/// </summary>
/// <remarks>
/// <para>
/// <b>Why Win32 and not a package.</b> Everything below is
/// <c>Shell_NotifyIcon</c> and friends through P/Invoke against DLLs that are
/// part of Windows. No NuGet package is added for this, and none is needed:
/// the tray API is older than .NET and has not moved.
/// </para>
/// <para>
/// <b>What this may do, and what it may never do.</b> Nothing reachable from
/// the tray or from a notification approves or sends anything. The menu opens
/// the window, toggles run-at-login, and asks to quit; the balloon, when
/// clicked, opens the window. That is the same rule the Linux tray
/// (<c>gtk/src/tray.rs</c>) and the Linux notifier (<c>gtk/src/notify.rs</c>)
/// hold: a surface reachable when the contributor is not looking at the
/// window gets the smallest possible vocabulary, because a misfire there
/// ships real transcripts and is unrecoverable.
/// </para>
/// <para>
/// <b>The interruption budget.</b> <see cref="ShowDigest"/> is the only path
/// in this app that can interrupt, and it is gated by
/// <see cref="DigestCadence"/> on top of the daemon's own spacing. See that
/// class for why there are two gates.
/// </para>
/// <para>
/// <b>Its own window, not the app's.</b> A tray icon needs an HWND to send
/// its callback message to. This creates a private, never-shown popup window
/// rather than subclassing the WinUI window: WinUI owns its message handling,
/// and inserting a subclass into it to catch one custom message is a
/// borrowed-authority bug waiting to happen. A plain popup rather than a
/// message-only (HWND_MESSAGE) window because <c>TrackPopupMenu</c> needs a
/// window that can be brought to the foreground, and a message-only window
/// cannot be.
/// </para>
/// </remarks>
public sealed class TrayIcon : IDisposable
{
    /// <summary>Raised when the contributor asks for the window.</summary>
    public event Action? OpenRequested;

    /// <summary>
    /// Raised when the contributor chooses Quit from the menu.
    /// </summary>
    /// <remarks>
    /// Deliberately a request rather than an exit. On Windows this app HOSTS
    /// the daemon in-process, so quitting stops the watcher, and the shared
    /// spec requires the contributor be told that before it happens. The
    /// handler raises the window and asks; this class must not shortcut it.
    /// </remarks>
    public event Action? QuitRequested;

    private const uint CallbackMessage = WM_APP + 1;
    private const uint IconId = 1;

    private const int MenuIdOpen = 1;
    private const int MenuIdRunAtLogin = 2;
    private const int MenuIdQuit = 3;

    private readonly WndProc _wndProc;
    private readonly string _className;
    private IntPtr _hwnd;
    private IntPtr _hIcon;
    private ushort _classAtom;
    private bool _added;
    private TrayModel _model = TrayModel.Compute(0, isPaused: false, isHealthy: true);
    private bool _disposed;

    /// <summary>
    /// Creates the tray icon, or reports failure by leaving
    /// <see cref="IsPresent"/> false.
    /// </summary>
    /// <remarks>
    /// Failure is never surfaced to the contributor. Same stance as the Linux
    /// tray: the window is the primary surface and it is still there, so a
    /// shell that refuses the icon costs an indicator, not the product. There
    /// is no code path anywhere that tells someone to fix their tray.
    /// </remarks>
    public TrayIcon()
    {
        // Rooted in a field: the native window class holds a raw function
        // pointer to this delegate for as long as the window lives, and
        // native code holding a pointer does not keep a managed delegate
        // alive. The same rule the subscribe callback in TcDaemon follows,
        // and the same crash if it is dropped.
        _wndProc = OnMessage;

        // Unique per instance. A class name is process-global, and
        // RegisterClassEx fails on a duplicate -- which would silently cost
        // the tray icon if this were ever constructed twice.
        _className = "TraceCommonsTray_" + Guid.NewGuid().ToString("N");

        try
        {
            _hwnd = CreateHostWindow();
            if (_hwnd == IntPtr.Zero)
            {
                return;
            }

            _added = AddIcon();
        }
        catch (DllNotFoundException)
        {
            // A build running somewhere without the shell DLLs. Nothing to
            // say about it.
            Debug.WriteLine("tracecommons tray unavailable");
        }
        catch (EntryPointNotFoundException)
        {
            Debug.WriteLine("tracecommons tray unavailable");
        }
    }

    /// <summary>Whether the icon actually made it into the notification area.</summary>
    public bool IsPresent => _added;

    /// <summary>
    /// Updates the icon, its tooltip and its menu header from daemon state.
    /// </summary>
    /// <remarks>
    /// Takes counts and flags, not entries. Everything this surface shows is
    /// a fixed label plus a number by construction, so there is no path or
    /// identity available here to leak even by accident.
    /// </remarks>
    public void Update(int decisionsOwed, bool isPaused, bool isHealthy)
    {
        TrayModel model = TrayModel.Compute(decisionsOwed, isPaused, isHealthy);
        _model = model;

        if (!_added)
        {
            return;
        }

        IntPtr previous = _hIcon;
        _hIcon = CreateMarkIcon(model.State);

        var data = NewData();
        data.uFlags = NIF_ICON | NIF_TIP | NIF_SHOWTIP;
        data.hIcon = _hIcon;
        data.szTip = model.Tooltip;
        Shell_NotifyIcon(NIM_MODIFY, ref data);

        // Destroyed only after the shell has been handed the replacement, so
        // there is no window in which the tray is pointing at a freed icon.
        if (previous != IntPtr.Zero)
        {
            DestroyIcon(previous);
        }
    }

    /// <summary>
    /// Shows the digest, if the cadence allows it.
    /// </summary>
    /// <remarks>
    /// <para>
    /// A balloon rather than a toast with buttons. The spec's digest offers
    /// <c>[ Review ] [ Not now ]</c>; a tray balloon carries no buttons, so
    /// here Review is clicking the balloon and Not now is ignoring it, which
    /// is what "Not now" does anyway -- it "does nothing but dismiss". The
    /// property that matters survives exactly: there is no action on this
    /// notification that sends anything, and the default action opens a
    /// window. A richer toast needs an activation identity this unpackaged
    /// app does not have yet, and is worth revisiting with MSIX.
    /// </para>
    /// <para>
    /// The body is <see cref="DigestText"/>'s, which is the shared spec's
    /// wording transcribed, and carries counts and project labels only.
    /// </para>
    /// </remarks>
    /// <returns>Whether a notification was actually shown.</returns>
    public bool ShowDigest(string body)
    {
        ArgumentNullException.ThrowIfNull(body);

        if (!_added)
        {
            return false;
        }

        var data = NewData();
        data.uFlags = NIF_INFO;
        data.szInfo = Clamp(body, 255);
        data.szInfoTitle = Clamp(DigestText.Title, 63);

        // NIIF_NONE: no system icon in the balloon. NIIF_USER would show the
        // tray icon, which at balloon size is the mark stretched from 16px
        // and looks like a rendering bug.
        data.dwInfoFlags = NIIF_NONE;

        return Shell_NotifyIcon(NIM_MODIFY, ref data);
    }

    public void Dispose()
    {
        if (_disposed)
        {
            return;
        }

        _disposed = true;

        if (_added)
        {
            var data = NewData();
            Shell_NotifyIcon(NIM_DELETE, ref data);
            _added = false;
        }

        if (_hIcon != IntPtr.Zero)
        {
            DestroyIcon(_hIcon);
            _hIcon = IntPtr.Zero;
        }

        if (_hwnd != IntPtr.Zero)
        {
            DestroyWindow(_hwnd);
            _hwnd = IntPtr.Zero;
        }

        if (_classAtom != 0)
        {
            UnregisterClass(_className, GetModuleHandle(null));
            _classAtom = 0;
        }
    }

    private IntPtr CreateHostWindow()
    {
        IntPtr module = GetModuleHandle(null);

        var wc = new WNDCLASSEX
        {
            cbSize = Marshal.SizeOf<WNDCLASSEX>(),
            lpfnWndProc = Marshal.GetFunctionPointerForDelegate(_wndProc),
            hInstance = module,
            lpszClassName = _className,
        };

        _classAtom = RegisterClassEx(ref wc);
        if (_classAtom == 0)
        {
            return IntPtr.Zero;
        }

        // Not WS_VISIBLE: this window exists to receive a message and to be
        // something TrackPopupMenu can anchor to. It is never shown.
        return CreateWindowEx(
            0,
            _className,
            "Trace Commons",
            WS_POPUP,
            0, 0, 0, 0,
            IntPtr.Zero,
            IntPtr.Zero,
            module,
            IntPtr.Zero);
    }

    private bool AddIcon()
    {
        _hIcon = CreateMarkIcon(_model.State);

        var data = NewData();
        data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
        data.uCallbackMessage = CallbackMessage;
        data.hIcon = _hIcon;
        data.szTip = _model.Tooltip;

        if (!Shell_NotifyIcon(NIM_ADD, ref data))
        {
            return false;
        }

        // Version 4 changes the callback packing (the message is in the low
        // word of lParam and the cursor position arrives in wParam) and is
        // what makes the tooltip and the balloon behave like every other
        // modern tray icon. Asked for after NIM_ADD, which is the documented
        // order.
        var version = NewData();
        version.uVersionOrTimeout = NOTIFYICON_VERSION_4;
        Shell_NotifyIcon(NIM_SETVERSION, ref version);

        return true;
    }

    private NOTIFYICONDATA NewData() => new()
    {
        cbSize = Marshal.SizeOf<NOTIFYICONDATA>(),
        hWnd = _hwnd,
        uID = IconId,
        szTip = string.Empty,
        szInfo = string.Empty,
        szInfoTitle = string.Empty,
    };

    private IntPtr OnMessage(IntPtr hwnd, uint message, IntPtr wParam, IntPtr lParam)
    {
        if (message == CallbackMessage)
        {
            // Version 4 packing: the notification is the low word of lParam.
            uint notification = (uint)(lParam.ToInt64() & 0xFFFF);

            switch (notification)
            {
                case NIN_SELECT:
                case NIN_KEYSELECT:
                case NIN_BALLOONUSERCLICK:
                    // A click on the icon or on the digest balloon. The only
                    // thing either does is raise the window.
                    OpenRequested?.Invoke();
                    return IntPtr.Zero;

                case WM_CONTEXTMENU:
                    // Cursor position is in wParam under version 4, in screen
                    // coordinates already.
                    ShowMenu(
                        (short)(wParam.ToInt64() & 0xFFFF),
                        (short)(wParam.ToInt64() >> 16 & 0xFFFF));
                    return IntPtr.Zero;
            }
        }

        return DefWindowProc(hwnd, message, wParam, lParam);
    }

    /// <summary>
    /// The context menu.
    /// </summary>
    /// <remarks>
    /// <para>
    /// Deliberately short. The shared spec's tray menu also lists what is
    /// waiting per project, the week summary, pause and settings; this app
    /// has no settings surface, no week band and no pause control anywhere
    /// yet, and a menu item that opens nothing is worse than an absent one.
    /// The header line carries the state so the icon is never the only place
    /// it is said. The rest arrives with the surfaces it belongs to.
    /// </para>
    /// <para>
    /// Nothing here approves, dismisses or sends.
    /// </para>
    /// </remarks>
    private void ShowMenu(int x, int y)
    {
        IntPtr menu = CreatePopupMenu();
        if (menu == IntPtr.Zero)
        {
            return;
        }

        try
        {
            AppendMenu(menu, MF_STRING | MF_DISABLED | MF_GRAYED, IntPtr.Zero, _model.MenuHeader);
            AppendMenu(menu, MF_SEPARATOR, IntPtr.Zero, null);
            AppendMenu(menu, MF_STRING, MenuIdOpen, "Open Trace Commons");
            // Omitted entirely under MSIX, where run-at-login belongs to the
            // manifest and this toggle would write something the package
            // model does not honour. Drawn disabled would be worse: a
            // checkbox that cannot be ticked reads as a bug in the app rather
            // than as a property of the build.
            if (RunAtLogin.IsSupported)
            {
                AppendMenu(menu, MF_SEPARATOR, IntPtr.Zero, null);
                AppendMenu(
                    menu,
                    MF_STRING | (RunAtLogin.IsEnabled ? MF_CHECKED : MF_UNCHECKED),
                    MenuIdRunAtLogin,
                    "Start Trace Commons when I sign in");
            }

            AppendMenu(menu, MF_SEPARATOR, IntPtr.Zero, null);
            AppendMenu(menu, MF_STRING, MenuIdQuit, "Quit Trace Commons…");

            // Required before TrackPopupMenu, and the reason this class owns a
            // real popup window rather than a message-only one: without the
            // foreground window being ours, the menu does not dismiss when the
            // contributor clicks away from it and is left stranded on screen.
            SetForegroundWindow(_hwnd);

            int command = TrackPopupMenuEx(
                menu,
                TPM_RIGHTBUTTON | TPM_RETURNCMD | TPM_NONOTIFY,
                x,
                y,
                _hwnd,
                IntPtr.Zero);

            switch (command)
            {
                case MenuIdOpen:
                    OpenRequested?.Invoke();
                    break;

                case MenuIdRunAtLogin:
                    // Read back rather than toggled blind: RunAtLogin.Set
                    // returns what the registry says afterwards, so the next
                    // time this menu is built the checkmark is the truth.
                    RunAtLogin.Set(!RunAtLogin.IsEnabled);
                    break;

                case MenuIdQuit:
                    QuitRequested?.Invoke();
                    break;
            }
        }
        finally
        {
            DestroyMenu(menu);
        }
    }

    /// <summary>
    /// Rasterizes the mark at the current small-icon size and turns it into
    /// an HICON.
    /// </summary>
    /// <remarks>
    /// The pixels come from <see cref="MarkRaster"/>, which is in the
    /// platform-neutral assembly and is unit-tested; everything here is the
    /// GDI ceremony of getting a byte array into an icon handle.
    /// </remarks>
    private IntPtr CreateMarkIcon(TrayIconState state)
    {
        int dpi = _hwnd != IntPtr.Zero ? (int)GetDpiForWindow(_hwnd) : 96;
        int size = GetSystemMetricsForDpi(SM_CXSMICON, (uint)Math.Max(96, dpi));
        if (size <= 0)
        {
            size = 16;
        }

        bool lightTaskbar = IsTaskbarLight();

        // Single ink, following the taskbar rather than the app window.
        // Windows does not recolour a tray icon the way macOS recolours a
        // template image, so the app has to pick, and the surface it sits on
        // is the taskbar. tc_ink for that scheme, from the same palette
        // DesignSystem.xaml holds.
        uint ink = lightTaskbar ? 0xFF20241FU : 0xFFE8EAE3U;

        uint? dot = state switch
        {
            // tc_green: something is waiting for you. The spec asks for a
            // numeric badge here; a 16px tray icon cannot carry two legible
            // digits, so the count lives in the tooltip and in the menu
            // header, both of which a screen reader can also read, and the
            // icon carries only "there is something".
            TrayIconState.Attention => lightTaskbar ? 0xFF178F70U : 0xFF3FBE9AU,

            // The spec says an amber dot. This design system has no amber;
            // tc_coral is its "something went wrong" ink, and inventing a
            // token for one 3px dot would put a colour in the product that
            // no other surface uses.
            TrayIconState.Unhealthy => lightTaskbar ? 0xFFD65D4FU : 0xFFF2887AU,

            // Paused and idle carry no dot. Paused is "struck through" in the
            // spec; a strike at 16px is a smear, and the tooltip says
            // "Paused." in words.
            _ => null,
        };

        byte[] pixels = MarkRaster.Render(size, ink, dot);
        return IconFromPixels(pixels, size);
    }

    /// <summary>
    /// Whether the taskbar is drawing itself light.
    /// </summary>
    /// <remarks>
    /// <c>SystemUsesLightTheme</c> rather than <c>AppsUseLightTheme</c>: they
    /// are separate settings and the tray icon sits on the taskbar, which
    /// follows the system one. Absent or unreadable means the Windows
    /// default, which is a dark taskbar.
    /// </remarks>
    private static bool IsTaskbarLight()
    {
        try
        {
            using RegistryKey? key = Registry.CurrentUser.OpenSubKey(
                @"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");

            return key?.GetValue("SystemUsesLightTheme") is int value && value != 0;
        }
        catch (Exception e) when (e is UnauthorizedAccessException or System.Security.SecurityException)
        {
            return false;
        }
    }

    /// <summary>
    /// BGRA pixels to an HICON, via a top-down 32bpp DIB.
    /// </summary>
    /// <remarks>
    /// The mask bitmap is all zeros and is required rather than useful: a
    /// 32bpp icon is composited from its alpha channel, but
    /// <c>CreateIconIndirect</c> still demands an <c>hbmMask</c>, and a
    /// non-zero one would punch holes through the anti-aliasing.
    /// </remarks>
    private static IntPtr IconFromPixels(byte[] pixels, int size)
    {
        var header = new BITMAPINFOHEADER
        {
            biSize = Marshal.SizeOf<BITMAPINFOHEADER>(),
            biWidth = size,

            // Negative: top-down, matching the order MarkRaster writes rows
            // in. A positive height would render the mark upside down, which
            // on a symmetric-looking mark is the kind of bug that ships.
            biHeight = -size,
            biPlanes = 1,
            biBitCount = 32,
            biCompression = BI_RGB,
        };

        IntPtr colour = CreateDIBSection(
            IntPtr.Zero, ref header, DIB_RGB_COLORS, out IntPtr bits, IntPtr.Zero, 0);

        if (colour == IntPtr.Zero || bits == IntPtr.Zero)
        {
            if (colour != IntPtr.Zero)
            {
                DeleteObject(colour);
            }

            return IntPtr.Zero;
        }

        Marshal.Copy(pixels, 0, bits, pixels.Length);

        // 1bpp scanlines are padded to 4-byte boundaries.
        int maskStride = (size + 31) / 32 * 4;
        var maskBits = new byte[maskStride * size];
        IntPtr mask = CreateBitmap(size, size, 1, 1, maskBits);

        if (mask == IntPtr.Zero)
        {
            DeleteObject(colour);
            return IntPtr.Zero;
        }

        var info = new ICONINFO
        {
            fIcon = true,
            hbmMask = mask,
            hbmColor = colour,
        };

        IntPtr icon = CreateIconIndirect(ref info);

        // CreateIconIndirect copies both bitmaps, so the originals are ours
        // to release immediately whether or not it succeeded.
        DeleteObject(mask);
        DeleteObject(colour);

        return icon;
    }

    /// <summary>
    /// Trims to a fixed-size native buffer, on a character boundary.
    /// <c>szInfo</c> is 256 wide characters including the terminator and
    /// <c>szInfoTitle</c> is 64; overflowing either is a marshalling
    /// exception rather than a truncation.
    /// </summary>
    private static string Clamp(string text, int max) =>
        text.Length <= max ? text : text.Substring(0, max);

    // ---- Win32 -----------------------------------------------------------

    private const uint WM_APP = 0x8000;
    private const uint WM_CONTEXTMENU = 0x007B;
    private const uint NIN_SELECT = 0x0400;
    private const uint NIN_KEYSELECT = 0x0401;
    private const uint NIN_BALLOONUSERCLICK = 0x0405;

    private const uint WS_POPUP = 0x80000000;

    private const uint NIM_ADD = 0x00000000;
    private const uint NIM_MODIFY = 0x00000001;
    private const uint NIM_DELETE = 0x00000002;
    private const uint NIM_SETVERSION = 0x00000004;

    private const uint NIF_MESSAGE = 0x00000001;
    private const uint NIF_ICON = 0x00000002;
    private const uint NIF_TIP = 0x00000004;
    private const uint NIF_INFO = 0x00000010;
    private const uint NIF_SHOWTIP = 0x00000080;

    private const uint NIIF_NONE = 0x00000000;
    private const uint NOTIFYICON_VERSION_4 = 4;

    private const uint MF_STRING = 0x00000000;
    private const uint MF_SEPARATOR = 0x00000800;
    private const uint MF_CHECKED = 0x00000008;
    private const uint MF_UNCHECKED = 0x00000000;
    private const uint MF_DISABLED = 0x00000002;
    private const uint MF_GRAYED = 0x00000001;

    private const uint TPM_RIGHTBUTTON = 0x0002;
    private const uint TPM_RETURNCMD = 0x0100;
    private const uint TPM_NONOTIFY = 0x0080;

    private const int SM_CXSMICON = 49;
    private const uint BI_RGB = 0;
    private const uint DIB_RGB_COLORS = 0;

    private delegate IntPtr WndProc(IntPtr hwnd, uint message, IntPtr wParam, IntPtr lParam);

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    private struct NOTIFYICONDATA
    {
        public int cbSize;
        public IntPtr hWnd;
        public uint uID;
        public uint uFlags;
        public uint uCallbackMessage;
        public IntPtr hIcon;

        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 128)]
        public string szTip;

        public uint dwState;
        public uint dwStateMask;

        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 256)]
        public string szInfo;

        /// <summary>A union in the header: uTimeout on old shells, uVersion under NIM_SETVERSION.</summary>
        public uint uVersionOrTimeout;

        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 64)]
        public string szInfoTitle;

        public uint dwInfoFlags;
        public Guid guidItem;
        public IntPtr hBalloonIcon;
    }

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    private struct WNDCLASSEX
    {
        public int cbSize;
        public uint style;
        public IntPtr lpfnWndProc;
        public int cbClsExtra;
        public int cbWndExtra;
        public IntPtr hInstance;
        public IntPtr hIcon;
        public IntPtr hCursor;
        public IntPtr hbrBackground;

        [MarshalAs(UnmanagedType.LPWStr)]
        public string? lpszMenuName;

        [MarshalAs(UnmanagedType.LPWStr)]
        public string lpszClassName;

        public IntPtr hIconSm;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct BITMAPINFOHEADER
    {
        public int biSize;
        public int biWidth;
        public int biHeight;
        public ushort biPlanes;
        public ushort biBitCount;
        public uint biCompression;
        public uint biSizeImage;
        public int biXPelsPerMeter;
        public int biYPelsPerMeter;
        public uint biClrUsed;
        public uint biClrImportant;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct ICONINFO
    {
        [MarshalAs(UnmanagedType.Bool)]
        public bool fIcon;

        public int xHotspot;
        public int yHotspot;
        public IntPtr hbmMask;
        public IntPtr hbmColor;
    }

    [DllImport("shell32.dll", CharSet = CharSet.Unicode, EntryPoint = "Shell_NotifyIconW")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool Shell_NotifyIcon(uint message, ref NOTIFYICONDATA data);

    [DllImport("user32.dll", CharSet = CharSet.Unicode, EntryPoint = "RegisterClassExW")]
    private static extern ushort RegisterClassEx(ref WNDCLASSEX wc);

    [DllImport("user32.dll", CharSet = CharSet.Unicode, EntryPoint = "UnregisterClassW")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool UnregisterClass(string className, IntPtr instance);

    [DllImport("user32.dll", CharSet = CharSet.Unicode, EntryPoint = "CreateWindowExW")]
    private static extern IntPtr CreateWindowEx(
        uint exStyle,
        string className,
        string windowName,
        uint style,
        int x,
        int y,
        int width,
        int height,
        IntPtr parent,
        IntPtr menu,
        IntPtr instance,
        IntPtr param);

    [DllImport("user32.dll", CharSet = CharSet.Unicode, EntryPoint = "DefWindowProcW")]
    private static extern IntPtr DefWindowProc(IntPtr hwnd, uint message, IntPtr wParam, IntPtr lParam);

    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool DestroyWindow(IntPtr hwnd);

    [DllImport("user32.dll")]
    private static extern IntPtr CreatePopupMenu();

    [DllImport("user32.dll", CharSet = CharSet.Unicode, EntryPoint = "AppendMenuW")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool AppendMenu(IntPtr menu, uint flags, IntPtr id, string? item);

    [DllImport("user32.dll", CharSet = CharSet.Unicode, EntryPoint = "AppendMenuW")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool AppendMenu(IntPtr menu, uint flags, int id, string? item);

    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool DestroyMenu(IntPtr menu);

    [DllImport("user32.dll")]
    private static extern int TrackPopupMenuEx(
        IntPtr menu, uint flags, int x, int y, IntPtr hwnd, IntPtr parameters);

    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool SetForegroundWindow(IntPtr hwnd);

    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool DestroyIcon(IntPtr icon);

    [DllImport("user32.dll")]
    private static extern IntPtr CreateIconIndirect(ref ICONINFO info);

    [DllImport("user32.dll")]
    private static extern uint GetDpiForWindow(IntPtr hwnd);

    [DllImport("user32.dll")]
    private static extern int GetSystemMetricsForDpi(int index, uint dpi);

    [DllImport("gdi32.dll")]
    private static extern IntPtr CreateDIBSection(
        IntPtr dc,
        ref BITMAPINFOHEADER header,
        uint usage,
        out IntPtr bits,
        IntPtr section,
        uint offset);

    [DllImport("gdi32.dll")]
    private static extern IntPtr CreateBitmap(
        int width, int height, uint planes, uint bitsPerPixel, byte[] bits);

    [DllImport("gdi32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool DeleteObject(IntPtr handle);

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, EntryPoint = "GetModuleHandleW")]
    private static extern IntPtr GetModuleHandle(string? moduleName);
}
