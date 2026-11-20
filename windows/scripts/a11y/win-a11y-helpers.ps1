# Win32 and drawing helpers for win-a11y-drive.ps1: windows by process and
# handle, PrintWindow captures cropped to the visible frame, keys and pointer,
# and the contrast measure (WCAG relative luminance; the ground is the
# rectangle's commonest colour, the ink the 0.2%-rank most contrasting pixel).
Add-Type -ReferencedAssemblies System.Drawing -TypeDefinition @"
using System;
using System.Collections.Generic;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;
using System.Text;
public static class W {
    public delegate bool EnumProc(IntPtr h, IntPtr p);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc f, IntPtr p);
    [DllImport("user32.dll")] public static extern bool EnumChildWindows(IntPtr parent, EnumProc f, IntPtr p);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
    [DllImport("user32.dll")] public static extern IntPtr GetWindow(IntPtr h, uint cmd);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint f);
    [DllImport("user32.dll")] public static extern IntPtr SetProcessDpiAwarenessContext(IntPtr v);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int c);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint f, UIntPtr e);
    [DllImport("user32.dll")] public static extern void mouse_event(uint f, int dx, int dy, uint d, UIntPtr e);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll", EntryPoint = "GetWindowLongPtrW")] public static extern IntPtr GetWindowLongPtr(IntPtr h, int i);
    [DllImport("user32.dll")] public static extern bool SystemParametersInfo(uint a, uint b, IntPtr c, uint d);
    [DllImport("user32.dll")] public static extern bool SystemParametersInfo(uint a, uint b, ref int c, uint d);
    [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h, int a, out RECT v, int s);
    [DllImport("user32.dll")] public static extern IntPtr FindWindow(string c, string n);
    [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
    public static IntPtr BigIcon(IntPtr h) { return SendMessage(h, 0x7F, new IntPtr(1), IntPtr.Zero); }
    public static IntPtr Pack(int x, int y) { return new IntPtr((long)(((uint)(ushort)(short)y << 16) | (ushort)(short)x)); }
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    public static void Dpi() { SetProcessDpiAwarenessContext(new IntPtr(-4)); }
    public static string Cls(IntPtr h) { var s = new StringBuilder(256); GetClassName(h, s, 256); return s.ToString(); }
    public static string Txt(IntPtr h) { var s = new StringBuilder(256); GetWindowText(h, s, 256); return s.ToString(); }
    public static long ExStyle(IntPtr h) { return GetWindowLongPtr(h, -20).ToInt64(); }
    public static long Style(IntPtr h) { return GetWindowLongPtr(h, -16).ToInt64(); }
    public static List<IntPtr> TopLevel(int pid, bool visibleOnly) {
        var found = new List<IntPtr>();
        EnumWindows((h, p) => { uint o; GetWindowThreadProcessId(h, out o);
            if (o == (uint)pid && (!visibleOnly || IsWindowVisible(h)) && GetWindow(h, 4) == IntPtr.Zero) found.Add(h); return true; }, IntPtr.Zero);
        return found;
    }
    public static RECT Frame(IntPtr h) { RECT f; if (DwmGetWindowAttribute(h, 9, out f, Marshal.SizeOf(typeof(RECT))) != 0) GetWindowRect(h, out f); return f; }
    public static string R(IntPtr h) { RECT f = Frame(h); return f.Left + "," + f.Top + " " + (f.Right - f.Left) + "x" + (f.Bottom - f.Top); }
    public static Bitmap Grab(IntPtr h) {
        RECT win; GetWindowRect(h, out win); RECT vis = Frame(h);
        int w = win.Right - win.Left, hh = win.Bottom - win.Top;
        var full = new Bitmap(Math.Max(w,1), Math.Max(hh,1), PixelFormat.Format32bppArgb);
        using (var g = Graphics.FromImage(full)) { IntPtr dc = g.GetHdc(); PrintWindow(h, dc, 2); g.ReleaseHdc(dc); }
        var crop = new Rectangle(vis.Left - win.Left, vis.Top - win.Top, vis.Right - vis.Left, vis.Bottom - vis.Top);
        crop.Intersect(new Rectangle(0, 0, w, hh));
        if (crop.Width <= 0 || crop.Height <= 0) return full;
        var c = full.Clone(crop, PixelFormat.Format32bppArgb); full.Dispose(); return c;
    }
    public static string Capture(IntPtr h, string path) {
        using (var b = Grab(h)) b.Save(path, ImageFormat.Png);
        return "hwnd=0x" + h.ToString("X") + " frame=" + R(h) + " dpi=" + GetDpiForWindow(h) + " fg=" + (GetForegroundWindow() == h);
    }
    // Mean luminance of a rectangle inside the window (window-relative physical px).
    public static double Lum(IntPtr h, int x, int y, int w, int hh) {
        using (var b = Grab(h)) {
            double sum = 0; int n = 0;
            for (int j = Math.Max(0,y); j < Math.Min(b.Height, y + hh); j += 2)
                for (int i = Math.Max(0,x); i < Math.Min(b.Width, x + w); i += 2) { var c = b.GetPixel(i, j); sum += 0.2126*c.R + 0.7152*c.G + 0.0722*c.B; n++; }
            return n == 0 ? -1 : sum / n;
        }
    }
    public static void Fg(IntPtr h) { if (GetForegroundWindow() == h) return; keybd_event(0x12, 0, 0, UIntPtr.Zero); SetForegroundWindow(h); keybd_event(0x12, 0, 2, UIntPtr.Zero); }
    public static void Key(byte vk, bool shift) {
        if (shift) keybd_event(0x10, 0, 0, UIntPtr.Zero);
        keybd_event(vk, 0, 0, UIntPtr.Zero); keybd_event(vk, 0, 2, UIntPtr.Zero);
        if (shift) keybd_event(0x10, 0, 2, UIntPtr.Zero);
    }
    [DllImport("user32.dll")] public static extern int GetSystemMetrics(int i);
    // A real pointer move (SetCursorPos alone does not reach WinUI's pointer events).
    public static void Move(int x, int y) {
        int w = GetSystemMetrics(0), h = GetSystemMetrics(1);
        SetCursorPos(x, y);
        mouse_event(0x8001, (int)Math.Round(x * 65535.0 / (w - 1)), (int)Math.Round(y * 65535.0 / (h - 1)), 0, UIntPtr.Zero);
    }
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
    [StructLayout(LayoutKind.Sequential)] public struct CURSORINFO { public int cbSize; public int flags; public IntPtr hCursor; public POINT pt; }
    [DllImport("user32.dll")] public static extern bool GetCursorInfo(ref CURSORINFO ci);
    [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(POINT p);
    public static string CursorReport() {
        var ci = new CURSORINFO(); ci.cbSize = Marshal.SizeOf(typeof(CURSORINFO)); GetCursorInfo(ref ci);
        IntPtr w = WindowFromPoint(ci.pt); uint pid; GetWindowThreadProcessId(w, out pid);
        return "cursor=" + ci.pt.X + "," + ci.pt.Y + " flags=0x" + ci.flags.ToString("X") + " (1 showing, 2 suppressed) under=" + Cls(w) + " pid=" + pid;
    }
    public static void Click() { mouse_event(2, 0, 0, 0, UIntPtr.Zero); mouse_event(4, 0, 0, 0, UIntPtr.Zero); }
    public static bool Anim() { int v = 0; SystemParametersInfo(0x1042, 0, ref v, 0); return v != 0; }
    public static void SetAnim(bool on) { SystemParametersInfo(0x1043, 0, new IntPtr(on ? 1 : 0), 3); }
    // Tight-loop grabs for ms milliseconds; returns "t:bitmap" pairs analysed later.
    public static List<KeyValuePair<long, Bitmap>> Burst(IntPtr h, int ms) {
        var list = new List<KeyValuePair<long, Bitmap>>();
        var sw = System.Diagnostics.Stopwatch.StartNew();
        while (sw.ElapsedMilliseconds < ms) list.Add(new KeyValuePair<long, Bitmap>(sw.ElapsedMilliseconds, Grab(h)));
        return list;
    }
    public static double LumOf(Bitmap b, int x, int y, int w, int hh) {
        double sum = 0; int n = 0;
        for (int j = Math.Max(0,y); j < Math.Min(b.Height, y + hh); j += 2)
            for (int i = Math.Max(0,x); i < Math.Min(b.Width, x + w); i += 2) { var c = b.GetPixel(i, j); sum += 0.2126*c.R + 0.7152*c.G + 0.0722*c.B; n++; }
        return n == 0 ? -1 : sum / n;
    }
    // Leftmost column on row y whose colour differs from column 1 (the window ground) by more than tol.
    public static int Edge(Bitmap b, int y, int tol) {
        if (y < 0 || y >= b.Height) return -1;
        var g = b.GetPixel(1, y);
        for (int i = 2; i < b.Width; i++) { var c = b.GetPixel(i, y); if (Math.Abs(c.R-g.R)+Math.Abs(c.G-g.G)+Math.Abs(c.B-g.B) > tol) return i; }
        return -1;
    }
    public static int Diff(Bitmap a, Bitmap b) {
        if (a.Width != b.Width || a.Height != b.Height) return -1;
        int n = 0;
        for (int j = 0; j < a.Height; j += 2) for (int i = 0; i < a.Width; i += 2) { if (a.GetPixel(i,j).ToArgb() != b.GetPixel(i,j).ToArgb()) n++; }
        return n;
    }
    public static IntPtr TrayHost(int pid) {
        IntPtr hit = IntPtr.Zero;
        foreach (var h in TopLevel(pid, false)) if (Cls(h).StartsWith("TraceCommonsTray_")) hit = h;
        return hit;
    }
}
"@

Add-Type -ReferencedAssemblies System.Drawing -TypeDefinition @"
using System;
using System.Collections.Generic;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;
public static class Cx {
    static double Lin(int c) { double s = c / 255.0; return s <= 0.04045 ? s / 12.92 : Math.Pow((s + 0.055) / 1.055, 2.4); }
    public static double Lum(int rgb) { return 0.2126 * Lin((rgb >> 16) & 255) + 0.7152 * Lin((rgb >> 8) & 255) + 0.0722 * Lin(rgb & 255); }
    public static double Ratio(int a, int b) { double x = Lum(a), y = Lum(b); return (Math.Max(x, y) + 0.05) / (Math.Min(x, y) + 0.05); }
    // The ground is the most common colour in the rectangle; the ink is the
    // pixel with the highest contrast against it (anti-aliased edges are
    // lighter than the ink, so the extreme pixel is the drawn ink colour).
    public static string Measure(Bitmap b, int x, int y, int w, int h) {
        var r = Rectangle.Intersect(new Rectangle(x, y, w, h), new Rectangle(0, 0, b.Width, b.Height));
        if (r.Width < 2 || r.Height < 2) return "skip=outside";
        var data = b.LockBits(r, ImageLockMode.ReadOnly, PixelFormat.Format32bppArgb);
        int[] px = new int[r.Width * r.Height];
        for (int j = 0; j < r.Height; j++) Marshal.Copy(IntPtr.Add(data.Scan0, j * data.Stride), px, j * r.Width, r.Width);
        b.UnlockBits(data);
        var count = new Dictionary<int, int>();
        foreach (int p in px) { int q = p & 0xFFFFFF; int n; count.TryGetValue(q, out n); count[q] = n + 1; }
        int bg = 0, best = -1;
        foreach (var kv in count) if (kv.Value > best) { best = kv.Value; bg = kv.Key; }
        // Every pixel's contrast against the ground, most contrasting first.
        // ratio: the k-th of them, k = 0.2% of the rectangle (at least 3, at
        // most a fifth of the non-ground pixels), so a stray pixel from a
        // neighbour cannot pass failing text, while anti-aliased edges (each
        // a different shade) do not hide the glyph cores.
        var cs = new List<KeyValuePair<double, int>>();
        foreach (int p in px) { int q = p & 0xFFFFFF; if (q != bg) cs.Add(new KeyValuePair<double, int>(Ratio(q, bg), q)); }
        if (cs.Count == 0) return string.Format("bg=#{0:X6} ink=#{0:X6} ratio=1.00 max=#{0:X6}/1.00 bgShare=1.00 colours=1", bg);
        cs.Sort((a, c) => c.Key.CompareTo(a.Key));
        int k = Math.Max(3, Math.Min((int)(px.Length * 0.002), cs.Count / 5));
        k = Math.Min(k, cs.Count);
        var pick = cs[k - 1];
        return string.Format("bg=#{0:X6} ink=#{1:X6} ratio={2:0.00} max=#{3:X6}/{4:0.00} bgShare={5:0.00} colours={6}", bg, pick.Value, pick.Key, cs[0].Value, cs[0].Key, best / (double)px.Length, count.Count);
    }
}
"@
