# Launch a GUI app in the interactive console session and capture ITS WINDOW,
# found by its window handle, never the screen.
#
# WHY THIS EXISTS. A WinUI app cannot start in session 0, which is where an
# SSH session lands. Anything launched from there dies inside
# Microsoft.UI.Xaml.dll with a stowed exception (0xc000027b) before a single
# line of managed application code runs -- no window station, no desktop, no
# XAML. That failure looks exactly like an application bug and is not one.
#
# So this script does not run the app itself. It hands the launch to a
# scheduled task registered against the logged-on interactive user, which
# Windows starts INSIDE that user's session, and then captures from a second
# task in the same session.
#
# WHAT IS CAPTURED. Only the launched process's own top-level window: found
# by enumerating windows and checking each one's owning process id against the
# process this script launched (or -ProcessId), painted with PrintWindow
# (PW_RENDERFULLCONTENT) and cropped to the window's visible frame
# (DWMWA_EXTENDED_FRAME_BOUNDS; the rest of GetWindowRect is an invisible
# resize border that PrintWindow paints black). Nothing else on the screen --
# another app, a notification, anyone's desktop -- can appear in the image.
# Each capture writes <png>.log with the box's UTC time, the window handle,
# the owning pid, the frame, the DPI, and which window was in the foreground.
#
# Both tasks start through wscript.exe, which has no window: a console window
# from the task would take the foreground and the app would be photographed
# in its inactive state (spike S7, 2026-11-18). The log says whether the app
# was in the foreground; an inactive capture is reported, never hidden.
#
# The console session on the test boxes has a display with nobody connected
# over RDP (auto-logon). A DISCONNECTED RDP session generally does not.

[CmdletBinding(DefaultParameterSetName = 'Launch')]
param(
    # The executable to launch.
    [Parameter(Mandatory = $true, ParameterSetName = 'Launch')]
    [string] $Exe,

    # Capture an app that is already running, by process id, instead.
    [Parameter(Mandatory = $true, ParameterSetName = 'Running')]
    [int] $ProcessId,

    # Where the PNG lands. Not the drive root: the tasks run with /IT, as the
    # interactive user WITHOUT elevation, and cannot write to C:\.
    [string] $Out = 'C:\tc-captures\tc-capture.png',

    # How long to let the app settle before capturing. A WinUI cold start does
    # real work -- runtime bootstrap, XAML load, first layout -- and capturing
    # too early photographs an empty window.
    [int] $SettleSeconds = 8,

    # Variables set for the launched app only, as NAME=VALUE. Names are
    # letters, digits and underscores; values letters, digits and . _ + -.
    [Parameter(ParameterSetName = 'Launch')]
    [string[]] $Environment = @(),

    # The window's visible frame, in physical pixels, before the capture.
    # 0 leaves the app's own size.
    [int] $Width = 0,
    [int] $Height = 0,

    # Stop the launched process (by its id, never by name) after capturing.
    [switch] $Stop
)

$ErrorActionPreference = 'Stop'

# schtasks writes to stderr for ordinary, expected conditions -- deleting a
# task that was never created being the obvious one. Under
# ErrorActionPreference = 'Stop' PowerShell promotes any native stderr write to
# a terminating error, so these helpers scope the relaxed preference to the
# native calls.
function Remove-TaskQuietly {
    param([string] $Name)
    $previous = $ErrorActionPreference
    $ErrorActionPreference = 'SilentlyContinue'
    & schtasks.exe /Delete /TN $Name /F *>$null
    $ErrorActionPreference = $previous
}

function Invoke-Schtasks {
    param([string[]] $Arguments)
    $previous = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    $output = & schtasks.exe @Arguments 2>&1
    $code = $LASTEXITCODE
    $ErrorActionPreference = $previous
    if ($code -ne 0) {
        throw "schtasks $($Arguments -join ' ') failed ($code): $output"
    }
}

function Get-InteractiveUser {
    # The logged-on user of the ACTIVE session. `query session` marks only
    # the current session (session 0 here) with '>', so the state column is
    # what to trust: Active means a user is present.
    $lines = query session 2>$null | Select-Object -Skip 1
    foreach ($line in $lines) {
        if ($line -match '^\s*>?(\S+)\s+(\S+)\s+(\d+)\s+(\w+)') {
            $user = $matches[2]
            $id = [int] $matches[3]
            $state = $matches[4]
            if ($state -eq 'Active' -and $user -ne '' -and $id -ne 0) {
                return [pscustomobject]@{ User = $user; Id = $id }
            }
        }
    }
    return $null
}

# Runs one command line in the interactive session through wscript.exe, so no
# console window opens there. $EnvLines set variables for that command only.
function Start-InSession {
    param([string] $Name, [string] $CommandLine, [string] $User, [string[]] $EnvLines, [int] $Show, [bool] $Wait)
    $vbs = Join-Path $script:workDir "$Name.vbs"
    $quoted = $CommandLine.Replace('"', '""')
    $waitWord = if ($Wait) { 'True' } else { 'False' }
    $lines = @('Set sh = CreateObject("WScript.Shell")') + $EnvLines + @("sh.Run ""$quoted"", $Show, $waitWord")
    Set-Content -Path $vbs -Encoding ascii -Value $lines
    Remove-TaskQuietly $Name
    Invoke-Schtasks @('/Create', '/TN', $Name, '/TR', "wscript.exe //B //Nologo `"$vbs`"",
        '/SC', 'ONCE', '/ST', '00:00', '/RU', $User, '/IT', '/F')
    Invoke-Schtasks @('/Run', '/TN', $Name)
}

$session = Get-InteractiveUser
if (-not $session) {
    throw 'No active interactive session. Auto-logon has not completed, or nobody is logged on.'
}
Write-Host "interactive session: $($session.User) (id $($session.Id))"

# The working directory, writable by the unelevated tasks.
$outDir = Split-Path -Parent $Out
$script:workDir = $outDir
if (-not (Test-Path $outDir)) {
    New-Item -ItemType Directory -Force -Path $outDir | Out-Null
    $acl = Get-Acl $outDir
    $acl.AddAccessRule((New-Object System.Security.AccessControl.FileSystemAccessRule(
        'BUILTIN\Users', 'Modify', 'ContainerInherit,ObjectInherit', 'None', 'Allow')))
    Set-Acl -Path $outDir -AclObject $acl
}

# ---------------------------------------------------------------------------
# Launch, and find the process this launch made.
# ---------------------------------------------------------------------------
$launched = $false
if ($PSCmdlet.ParameterSetName -eq 'Launch') {
    if (-not (Test-Path $Exe)) {
        throw "Executable not found: $Exe"
    }
    $Exe = (Resolve-Path $Exe).Path
    $name = [IO.Path]::GetFileNameWithoutExtension($Exe)

    $envLines = @('Set env = sh.Environment("Process")')
    foreach ($pair in $Environment) {
        if ($pair -notmatch '^([A-Za-z_][A-Za-z0-9_]*)=([A-Za-z0-9_.+-]*)$') {
            throw "not NAME=VALUE with plain characters: $pair"
        }
        $envLines += "env(""$($matches[1])"") = ""$($matches[2])"""
    }

    $before = @(Get-Process -Name $name -ErrorAction SilentlyContinue | ForEach-Object { $_.Id })
    Start-InSession -Name 'tc-launch-app' -CommandLine "`"$Exe`"" -User $session.User -EnvLines $envLines -Show 1 -Wait $false
    Write-Host "launched $Exe in session $($session.Id); settling ${SettleSeconds}s"
    Start-Sleep -Seconds $SettleSeconds
    Remove-TaskQuietly 'tc-launch-app'

    $new = @(Get-Process -Name $name -ErrorAction SilentlyContinue |
        Where-Object { $before -notcontains $_.Id -and $_.Path -eq $Exe })
    if ($new.Count -ne 1) {
        throw "expected one new $name process from $Exe, found $($new.Count)"
    }
    $ProcessId = $new[0].Id
    $launched = $true
}

$target = Get-Process -Id $ProcessId -ErrorAction SilentlyContinue
if (-not $target) {
    throw "process $ProcessId is not running"
}
if ($target.SessionId -ne $session.Id) {
    throw "process $ProcessId is in session $($target.SessionId), not the interactive session $($session.Id)"
}

# ---------------------------------------------------------------------------
# Capture, from a hidden task in the same session.
# ---------------------------------------------------------------------------
$captureScript = @'
param([int] $ProcessId, [string] $Out, [int] $Width, [int] $Height)
$ErrorActionPreference = 'Stop'
try {
Add-Type -ReferencedAssemblies System.Drawing -TypeDefinition @"
using System;
using System.Collections.Generic;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;
using System.Text;

public static class TcWindowCapture {
    delegate bool EnumProc(IntPtr hwnd, IntPtr param);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc f, IntPtr param);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr hwnd);
    [DllImport("user32.dll")] static extern bool IsIconic(IntPtr hwnd);
    [DllImport("user32.dll")] static extern IntPtr GetWindow(IntPtr hwnd, uint cmd);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr hwnd, out RECT rect);
    [DllImport("user32.dll")] static extern bool PrintWindow(IntPtr hwnd, IntPtr hdc, uint flags);
    [DllImport("user32.dll")] static extern IntPtr SetProcessDpiAwarenessContext(IntPtr value);
    [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] static extern bool SetForegroundWindow(IntPtr hwnd);
    [DllImport("user32.dll")] static extern bool ShowWindow(IntPtr hwnd, int cmd);
    [DllImport("user32.dll")] static extern bool SetWindowPos(IntPtr hwnd, IntPtr after, int x, int y, int cx, int cy, uint flags);
    [DllImport("user32.dll")] static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
    [DllImport("user32.dll")] static extern uint GetDpiForWindow(IntPtr hwnd);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassName(IntPtr hwnd, StringBuilder name, int count);
    [DllImport("dwmapi.dll")] static extern int DwmGetWindowAttribute(IntPtr hwnd, int attribute, out RECT value, int size);

    public struct RECT { public int Left, Top, Right, Bottom; }

    const int DWMWA_EXTENDED_FRAME_BOUNDS = 9;
    const uint PW_RENDERFULLCONTENT = 2;
    const uint GW_OWNER = 4;
    const uint SWP_NOMOVE = 0x2, SWP_NOZORDER = 0x4, SWP_NOACTIVATE = 0x10;

    static RECT Frame(IntPtr hwnd) {
        RECT frame;
        if (DwmGetWindowAttribute(hwnd, DWMWA_EXTENDED_FRAME_BOUNDS, out frame, Marshal.SizeOf(typeof(RECT))) != 0)
            throw new Exception("DwmGetWindowAttribute failed");
        return frame;
    }

    public static string Capture(int pid, string path, int width, int height) {
        SetProcessDpiAwarenessContext(new IntPtr(-4));   // per-monitor v2: physical pixels
        var found = new List<IntPtr>();
        EnumWindows((h, p) => {
            uint owner;
            GetWindowThreadProcessId(h, out owner);
            if (owner == (uint)pid && IsWindowVisible(h) && GetWindow(h, GW_OWNER) == IntPtr.Zero) found.Add(h);
            return true;
        }, IntPtr.Zero);
        if (found.Count == 0) throw new Exception("no visible top-level window for pid " + pid);

        // The largest of the process's top-level windows is its main window.
        IntPtr hwnd = IntPtr.Zero;
        long best = -1;
        foreach (IntPtr h in found) {
            RECT r; GetWindowRect(h, out r);
            long area = (long)(r.Right - r.Left) * (r.Bottom - r.Top);
            if (area > best) { best = area; hwnd = h; }
        }

        if (IsIconic(hwnd)) ShowWindow(hwnd, 9);   // SW_RESTORE
        if (width > 0 && height > 0) {
            RECT outer; GetWindowRect(hwnd, out outer);
            RECT frame = Frame(hwnd);
            int dx = (outer.Right - outer.Left) - (frame.Right - frame.Left);
            int dy = (outer.Bottom - outer.Top) - (frame.Bottom - frame.Top);
            SetWindowPos(hwnd, IntPtr.Zero, 0, 0, width + dx, height + dy, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
            System.Threading.Thread.Sleep(1500);
        }

        // Ask for the foreground. Windows grants it to a process that has just
        // had input; a synthetic ALT tap supplies that.
        if (GetForegroundWindow() != hwnd) {
            keybd_event(0x12, 0, 0, UIntPtr.Zero);
            SetForegroundWindow(hwnd);
            keybd_event(0x12, 0, 2, UIntPtr.Zero);
            System.Threading.Thread.Sleep(800);
        }

        // The owner is checked again at the moment of capture.
        uint check;
        GetWindowThreadProcessId(hwnd, out check);
        if (check != (uint)pid) throw new Exception("window owner changed");

        RECT win; GetWindowRect(hwnd, out win);
        RECT vis = Frame(hwnd);
        int w = win.Right - win.Left, h2 = win.Bottom - win.Top;
        using (var full = new Bitmap(w, h2, PixelFormat.Format32bppArgb)) {
            using (var g = Graphics.FromImage(full)) {
                IntPtr dc = g.GetHdc();
                bool ok = PrintWindow(hwnd, dc, PW_RENDERFULLCONTENT);
                g.ReleaseHdc(dc);
                if (!ok) throw new Exception("PrintWindow failed");
            }
            var crop = new Rectangle(vis.Left - win.Left, vis.Top - win.Top, vis.Right - vis.Left, vis.Bottom - vis.Top);
            using (var visible = full.Clone(crop, PixelFormat.Format32bppArgb)) visible.Save(path, ImageFormat.Png);
        }

        IntPtr fg = GetForegroundWindow();
        uint fgPid; GetWindowThreadProcessId(fg, out fgPid);
        var cls = new StringBuilder(256); GetClassName(fg, cls, 256);
        return "hwnd=0x" + hwnd.ToString("X") + " pid=" + check + " windows=" + found.Count
            + " frame=" + vis.Left + "," + vis.Top + " " + (vis.Right - vis.Left) + "x" + (vis.Bottom - vis.Top)
            + " dpi=" + GetDpiForWindow(hwnd)
            + " foreground=" + (fg == hwnd) + " foregroundPid=" + fgPid + " foregroundClass=" + cls;
    }
}
"@
$info = [TcWindowCapture]::Capture($ProcessId, $Out, $Width, $Height)
Set-Content -Path "$Out.log" -Value ("boxUtc=" + (Get-Date).ToUniversalTime().ToString('o') + " " + $info)
} catch {
    Set-Content -Path "$Out.error.txt" -Value ($_ | Out-String)
}
'@

$captureFile = Join-Path $outDir 'tc-capture-window.ps1'
Set-Content -Path $captureFile -Value $captureScript -Encoding ascii
foreach ($stale in @($Out, "$Out.log", "$Out.error.txt")) {
    if (Test-Path $stale) { Remove-Item $stale -Force }
}

# SW_HIDE (0): the capturer has no window to take the foreground.
Start-InSession -Name 'tc-capture-window' -User $session.User -EnvLines @() -Show 0 -Wait $true -CommandLine (
    "powershell.exe -NoProfile -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -File `"$captureFile`"" +
    " -ProcessId $ProcessId -Out `"$Out`" -Width $Width -Height $Height")

# Poll for the result rather than sleeping a fixed interval.
for ($i = 0; $i -lt 60; $i++) {
    if ((Test-Path "$Out.log") -or (Test-Path "$Out.error.txt")) {
        Start-Sleep -Milliseconds 500
        break
    }
    Start-Sleep -Seconds 1
}
Remove-TaskQuietly 'tc-capture-window'
Remove-Item -Path $captureFile, (Join-Path $outDir 'tc-capture-window.vbs'), (Join-Path $outDir 'tc-launch-app.vbs') -Force -ErrorAction SilentlyContinue

if (Test-Path "$Out.error.txt") {
    throw "Capture failed: $(Get-Content "$Out.error.txt" -Raw)"
}
if (-not (Test-Path $Out)) {
    throw 'Capture produced no file and recorded no error.'
}
$log = (Get-Content "$Out.log" -Raw).Trim()
Write-Host "captured: $Out ($((Get-Item $Out).Length) bytes)"
Write-Host $log

if ($Stop -and $launched) {
    Stop-Process -Id $ProcessId -Force -ErrorAction SilentlyContinue
    Write-Host "stopped pid $ProcessId"
}

[pscustomobject]@{ ProcessId = $ProcessId; Png = $Out; Log = $log }
