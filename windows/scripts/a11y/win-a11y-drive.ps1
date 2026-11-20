# W25 keyboard and accessibility audit: the in-session driver. Started by
# win-a11y-run.ps1 as an /IT task in the console session; never run it from
# SSH (session 0 has no desktop). See windows/docs/dev-vm.md, "Keyboard and
# accessibility audit".
#
# W25 keyboard and accessibility audit driver. Runs INSIDE the interactive
# console session (an /IT task started by run.ps1). Launches the Debug app on
# one recorded sample set, then follows a plan file of steps. Every window is
# found by its HWND and owning pid; nothing outside the app is captured.
#
# Plan steps (one per line, '#' comments; also front, toggle, invoke,
# clicklast, rail-open, size, expect, exists, focusname, selected, hc,
# theme, scroll):
#   tab <AutomationId>               click a main-window element by AutomationId
#   click <win> <name>               click an element of window <win> by Name (first match)
#   clickaid <win> <aid>             click by AutomationId in window <win>
#   select <win> <name>              select a TabItem/ListItem by Name
#   key <win> <vk-hex> [ctrl|shift|ctrlshift]
#   newwin <label>                   the newest visible top-level window becomes <label>
#   flyout                           open the tray flyout as window 'flyout'
#   audit <win> <label> [maxTabs]    UIA dump, Tab walk, capture, contrast sweep
#   snap <win> <label>
#   state <win> <label>              log visibility, foreground and focused element
#   sleep <ms>
param(
    [string] $Exe,
    [string] $Set = 'normalDay',
    [string] $OutDir,
    [string] $Tag,
    [string] $Plan,
    [string] $FirstRunStep = '',
    [string] $Extra = '',
    [int] $Settle = 16,
    [switch] $HighContrast,
    [string] $HcScheme = ''
)
$ErrorActionPreference = 'Stop'
$log = Join-Path $OutDir "$Tag.drive.log"
# The runner reads this log while the driver writes it: retry a busy file.
function Log([string] $s) {
    $line = "[{0}] {1}`r`n" -f (Get-Date).ToString('HH:mm:ss.fff'), $s
    for ($i = 0; $i -lt 40; $i++) {
        try { [IO.File]::AppendAllText($log, $line, [Text.Encoding]::UTF8); return } catch { Start-Sleep -Milliseconds 50 }
    }
}
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -AssemblyName WindowsBase
. (Join-Path $PSScriptRoot 'win-a11y-helpers.ps1')
[W]::Dpi()
$UIA = [System.Windows.Automation.AutomationElement]
$TS = [System.Windows.Automation.TreeScope]
$walker = [System.Windows.Automation.TreeWalker]::ControlViewWalker
$wins = @{}
$appPid = 0

function Rid($e) { try { return ($e.GetRuntimeId() -join '.') } catch { return 'stale' } }
function Clean([string] $s) { if ($null -eq $s) { return '' }; return ($s -replace '[\r\n\t|]', ' ') }
function Desc($e) {
    try {
        $c = $e.Current; $r = $c.BoundingRectangle
        return ('{0} | {1} | class={2} | aid={3} | name="{4}" | help="{5}" | pid={6} | rect={7:0},{8:0},{9:0}x{10:0} | offscreen={11}' -f (Rid $e), $c.ControlType.ProgrammaticName.Replace('ControlType.', ''), (Clean $c.ClassName), (Clean $c.AutomationId), (Clean $c.Name), (Clean $c.HelpText), $c.ProcessId, $r.X, $r.Y, $r.Width, $r.Height, $c.IsOffscreen)
    } catch { return "ERR $_" }
}
function Focused() { try { return $UIA::FocusedElement } catch { return $null } }
function Find($win, $prop, $value) {
    $root = $UIA::FromHandle($wins[$win])
    return $root.FindFirst($TS::Descendants, (New-Object System.Windows.Automation.PropertyCondition($prop, $value)))
}
function ClickEl($el) {
    $r = $el.Current.BoundingRectangle
    [W]::Move([int]($r.X + $r.Width / 2), [int]($r.Y + $r.Height / 2)); Start-Sleep -Milliseconds 150; [W]::Click(); Start-Sleep -Milliseconds 200
    # Park the pointer off the window so hover never colours a capture.
    $f = [W]::Frame($wins['main']); [W]::Move($f.Right + 40, $f.Bottom - 6)
}

function Dump([string] $win, [string] $label) {
    $path = Join-Path $OutDir "$Tag-$label.uia.txt"
    $job = Start-Job -ArgumentList $wins[$win].ToInt64(), $path -ScriptBlock {
        param([long] $h, [string] $path)
        Add-Type -AssemblyName UIAutomationClient; Add-Type -AssemblyName UIAutomationTypes
        $UIA = [System.Windows.Automation.AutomationElement]
        $walker = [System.Windows.Automation.TreeWalker]::ControlViewWalker
        $lines = New-Object System.Collections.Generic.List[string]
        function Clean([string] $s) { if ($null -eq $s) { return '' }; return ($s -replace '[\r\n\t|]', ' ') }
        function Walk($e, $d) {
            if ($d -gt 45) { return }
            $c = $e.Current; $r = $c.BoundingRectangle
            $lines.Add(('{0}{1} | rid={2} | name="{3}" | help="{4}" | aid={5} | class={6} | focusable={7} | enabled={8} | offscreen={9} | rect={10:0},{11:0},{12:0}x{13:0}' -f ('  ' * $d), $c.ControlType.ProgrammaticName.Replace('ControlType.', ''), ($e.GetRuntimeId() -join '.'), (Clean $c.Name), (Clean $c.HelpText), (Clean $c.AutomationId), (Clean $c.ClassName), $c.IsKeyboardFocusable, $c.IsEnabled, $c.IsOffscreen, $r.X, $r.Y, $r.Width, $r.Height))
            $ch = $walker.GetFirstChild($e)
            while ($ch -ne $null) { Walk $ch ($d + 1); $ch = $walker.GetNextSibling($ch) }
        }
        Walk ($UIA::FromHandle([IntPtr]$h)) 0
        $lines | Set-Content -Path $path -Encoding UTF8
    }
    if (-not (Wait-Job $job -Timeout 60)) { Log "UIA dump $label timed out"; Stop-Job $job }
    Remove-Job $job -Force
}

function Walk([string] $win, [string] $label, [int] $max) {
    $h = $wins[$win]
    [W]::Fg($h); Start-Sleep -Milliseconds 700
    $out = New-Object System.Collections.Generic.List[string]
    $winRid = Rid ($UIA::FromHandle($h))
    $out.Add("window-rid: $winRid")
    $f0 = Focused
    $out.Add("start: " + (Desc $f0))
    $seen = @{}; $first = $null; $prevRid = $null; $repeat = 0
    $prevBmp = [W]::Grab($h)
    for ($i = 1; $i -le $max; $i++) {
        [W]::Key(0x09, $false); Start-Sleep -Milliseconds 420
        $f = Focused
        $rid = if ($f) { Rid $f } else { 'none' }
        # What the press changed on screen: 0 means the focus moved somewhere
        # nothing draws it (an unseen stop).
        $fh = if ($f) { try { [IntPtr]$f.Current.NativeWindowHandle } catch { [IntPtr]::Zero } } else { [IntPtr]::Zero }
        $bmp = [W]::Grab($h)
        $changed = [W]::Diff($prevBmp, $bmp)
        $prevBmp.Dispose(); $prevBmp = $bmp
        $out.Add(("{0,3}: {1} | changed={2}" -f $i, (Desc $f), $changed))
        if ($rid -eq $prevRid) { $repeat++ } else { $repeat = 0 }
        if ($repeat -ge 2) { $out.Add("TRAP: focus did not move for 3 Tabs"); break }
        if ($null -eq $first) { $first = $rid } elseif ($rid -eq $first) { $out.Add("CYCLE after $($i - 1) stops"); break }
        $prevRid = $rid
    }
    $prevBmp.Dispose()
    [W]::Key(0x09, $true); Start-Sleep -Milliseconds 260
    $out.Add("shift-tab: " + (Desc (Focused)))
    $out | Set-Content -Path (Join-Path $OutDir "$Tag-$label.tab.txt") -Encoding UTF8
}

function Contrast([string] $win, [string] $label) {
    $h = $wins[$win]
    $fr = [W]::Frame($h)
    $bmp = [W]::Grab($h)
    $png = Join-Path $OutDir "$Tag-$label.png"
    $bmp.Save($png, [System.Drawing.Imaging.ImageFormat]::Png)
    Log ("captured $label hwnd=0x{0:X} frame={1} dpi={2} fg={3}" -f $h.ToInt64(), [W]::R($h), [W]::GetDpiForWindow($h), ([W]::GetForegroundWindow() -eq $h))
    $root = $UIA::FromHandle($h)
    $cond = New-Object System.Windows.Automation.OrCondition(
        (New-Object System.Windows.Automation.PropertyCondition($UIA::ControlTypeProperty, [System.Windows.Automation.ControlType]::Text)),
        (New-Object System.Windows.Automation.PropertyCondition($UIA::ControlTypeProperty, [System.Windows.Automation.ControlType]::Hyperlink)))
    $lines = New-Object System.Collections.Generic.List[string]
    # Controls whose label has no Text element of its own (a Button with a
    # string, a tab, a list item): measured inside a 4 px inset, so the
    # control's border is not taken for its ink. Disabled ones are skipped:
    # inactive controls carry no contrast requirement.
    $controls = New-Object System.Windows.Automation.OrCondition(
        (New-Object System.Windows.Automation.PropertyCondition($UIA::ControlTypeProperty, [System.Windows.Automation.ControlType]::Button)),
        (New-Object System.Windows.Automation.PropertyCondition($UIA::ControlTypeProperty, [System.Windows.Automation.ControlType]::TabItem)),
        (New-Object System.Windows.Automation.PropertyCondition($UIA::ControlTypeProperty, [System.Windows.Automation.ControlType]::ListItem)),
        (New-Object System.Windows.Automation.PropertyCondition($UIA::ControlTypeProperty, [System.Windows.Automation.ControlType]::CheckBox)),
        (New-Object System.Windows.Automation.PropertyCondition($UIA::ControlTypeProperty, [System.Windows.Automation.ControlType]::RadioButton)),
        (New-Object System.Windows.Automation.PropertyCondition($UIA::ControlTypeProperty, [System.Windows.Automation.ControlType]::ComboBox)))
    $items = New-Object System.Collections.Generic.List[object]
    foreach ($e in $root.FindAll($TS::Descendants, $cond)) { $items.Add(@($e, 0)) }
    foreach ($e in $root.FindAll($TS::Descendants, $controls)) {
        if (-not $e.Current.IsEnabled -or -not $e.Current.Name) { continue }
        if ($e.FindFirst($TS::Descendants, $cond)) { continue }
        $items.Add(@($e, 4))
    }
    foreach ($pair in $items) {
        $e = $pair[0]; $inset = $pair[1]
        $c = $e.Current
        if ($c.IsOffscreen) { continue }
        # Inside a disabled control (a disabled field's header): inactive, no
        # contrast requirement. Covered by something drawn over it (a notice
        # over a row): what is measured is not its ink. Both are logged.
        $p = $walker.GetParent($e); $inactive = $false
        for ($k = 0; $k -lt 4 -and $p; $k++) { if (-not $p.Current.IsEnabled) { $inactive = $true; break }; $p = $walker.GetParent($p) }
        if ($inactive) { $lines.Add(('{0} | skip=inactive | name="{1}"' -f (Rid $e), (Clean $c.Name))); continue }
        $rc = $c.BoundingRectangle
        try {
            $hit = $UIA::FromPoint((New-Object System.Windows.Point(($rc.X + $rc.Width / 2), ($rc.Y + $rc.Height / 2))))
            $mine = (Rid $hit) -eq (Rid $e)
            $q = $walker.GetParent($e)
            for ($k = 0; -not $mine -and $k -lt 6 -and $q; $k++) { if ((Rid $q) -eq (Rid $hit)) { $mine = $true }; $q = $walker.GetParent($q) }
            $q = $walker.GetParent($hit)
            for ($k = 0; -not $mine -and $k -lt 6 -and $q; $k++) { if ((Rid $q) -eq (Rid $e)) { $mine = $true }; $q = $walker.GetParent($q) }
            if (-not $mine) { $lines.Add(('{0} | skip=covered by {1} "{2}" | name="{3}"' -f (Rid $e), (Rid $hit), (Clean $hit.Current.Name), (Clean $c.Name))); continue }
        } catch { }
        $r = $c.BoundingRectangle
        if ($r.Width -lt (4 + 2 * $inset) -or $r.Height -lt (6 + 2 * $inset)) { continue }
        $x = [int]($r.X - $fr.Left); $y = [int]($r.Y - $fr.Top)
        if ($x -lt 0 -or $y -lt 0 -or ($x + $r.Width) -gt ($fr.Right - $fr.Left) -or ($y + $r.Height) -gt ($fr.Bottom - $fr.Top)) { continue }
        $m = [Cx]::Measure($bmp, $x + $inset, $y + $inset, [int]$r.Width - 2 * $inset, [int]$r.Height - 2 * $inset)
        $lines.Add(('{0} | {1} | name="{2}" | rect={3},{4},{5:0}x{6:0} | {7}' -f (Rid $e), $c.ControlType.ProgrammaticName.Replace('ControlType.', ''), (Clean $c.Name), ($x + $inset), ($y + $inset), ($r.Width - 2 * $inset), ($r.Height - 2 * $inset), $m))
    }
    $bmp.Dispose()
    $lines | Set-Content -Path (Join-Path $OutDir "$Tag-$label.contrast.txt") -Encoding UTF8
}

function ScrollSweep([string] $win, [string] $label) {
    $root = $UIA::FromHandle($wins[$win])
    $cond = New-Object System.Windows.Automation.PropertyCondition($UIA::IsScrollPatternAvailableProperty, $true)
    $k = 0
    foreach ($sv in $root.FindAll($TS::Descendants, $cond)) {
        $sp = $null
        if (-not $sv.TryGetCurrentPattern([System.Windows.Automation.ScrollPattern]::Pattern, [ref]$sp)) { continue }
        if (-not $sp.Current.VerticallyScrollable -or $sv.Current.IsOffscreen) { continue }
        $view = [Math]::Max(10, $sp.Current.VerticalViewSize)
        $steps = [int][Math]::Ceiling(100 / $view)
        Log ("  scroller {0} view={1:0}% steps={2}" -f (Rid $sv), $view, $steps)
        for ($i = 1; $i -le $steps; $i++) {
            $pct = [Math]::Min(100, $i * $view)
            try { $sp.SetScrollPercent(-1, $pct) } catch { Log "  scroll failed: $_" }
            Start-Sleep -Milliseconds 700
            Contrast $win "$label-sc$k-$i"
        }
        try { $sp.SetScrollPercent(-1, 0) } catch { }
        Start-Sleep -Milliseconds 400
        $k++
    }
}

function Audit([string] $win, [string] $label, [int] $max) {
    if (-not $wins.ContainsKey($win) -or $wins[$win] -eq [IntPtr]::Zero) { Log "audit ${label}: no window '$win'"; return }
    [W]::Fg($wins[$win]); Start-Sleep -Milliseconds 600
    Contrast $win $label
    Dump $win $label
    Walk $win $label $max
    Log "audited $label"
}

function NewestWindow() {
    $all = [W]::TopLevel($appPid, $true)
    foreach ($h in $all) { if (-not ($wins.Values -contains $h)) { return $h } }
    return [IntPtr]::Zero
}


Add-Type -TypeDefinition @"
using System; using System.Runtime.InteropServices;
public static class Hc {
    [StructLayout(LayoutKind.Sequential)] public struct HIGHCONTRAST { public uint cbSize; public uint dwFlags; public IntPtr lpszDefaultScheme; }
    [DllImport("user32.dll", SetLastError = true)] static extern bool SystemParametersInfo(uint a, uint b, ref HIGHCONTRAST c, uint d);
    public static bool On() { var h = new HIGHCONTRAST(); h.cbSize = (uint)Marshal.SizeOf(h); SystemParametersInfo(0x42, h.cbSize, ref h, 0); return (h.dwFlags & 1) != 0; }
    public static bool Set(bool on, string scheme) { var h = new HIGHCONTRAST(); h.cbSize = (uint)Marshal.SizeOf(h); SystemParametersInfo(0x42, h.cbSize, ref h, 0); h.dwFlags = on ? (h.dwFlags | 1u) : (h.dwFlags & ~1u); h.lpszDefaultScheme = string.IsNullOrEmpty(scheme) ? IntPtr.Zero : Marshal.StringToHGlobalUni(scheme); return SystemParametersInfo(0x43, h.cbSize, ref h, 3); }
    public static string Scheme() { var h = new HIGHCONTRAST(); h.cbSize = (uint)Marshal.SizeOf(h); SystemParametersInfo(0x42, h.cbSize, ref h, 0); return h.lpszDefaultScheme == IntPtr.Zero ? "" : Marshal.PtrToStringUni(h.lpszDefaultScheme); }
}
"@
Add-Type -TypeDefinition @"
using System; using System.Runtime.InteropServices;
public static class Sz {
    [DllImport("user32.dll")] static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint f);
    public static bool Size(IntPtr h, int w, int hh) { return SetWindowPos(h, IntPtr.Zero, 0, 0, w, hh, 0x0016); }
}
"@
Add-Type -TypeDefinition @"
using System; using System.Runtime.InteropServices;
public static class Bc {
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern IntPtr SendMessageTimeout(IntPtr h, uint m, UIntPtr w, string l, uint f, uint t, out UIntPtr r);
    public static bool Send() { UIntPtr r; return SendMessageTimeout(new IntPtr(0xFFFF), 0x1A, UIntPtr.Zero, "ImmersiveColorSet", 2, 3000, out r) != IntPtr.Zero; }
}
"@
$hcBefore = [Hc]::On()
function SchemeReg() { (Get-ItemProperty 'HKCU:\Control Panel\Accessibility\HighContrast' -EA SilentlyContinue).'High Contrast Scheme' }
$schemeBefore = SchemeReg
$want = switch ($HcScheme) { 'white' { 'High Contrast White' } 'black' { 'High Contrast Black' } default { '' } }
if ($HighContrast) { $ok = [Hc]::Set($true, $want); Start-Sleep -Seconds 8; Log "high contrast set=$ok now=$([Hc]::On()) scheme='$(SchemeReg)' before=$hcBefore/'$schemeBefore'" }
$env:TRACE_COMMONS_SAMPLE = $Set
if ($FirstRunStep) { $env:TRACE_COMMONS_FIRST_RUN_STEP = $FirstRunStep }
if ($Extra) { $env:TRACE_COMMONS_SAMPLE_EXTRA = $Extra }
Log "start tag=$Tag set=$Set step=$FirstRunStep spiAnim=$([W]::Anim())"
try {
    $p = Start-Process -FilePath $Exe -PassThru
    $appPid = $p.Id
    Log "launched pid=$appPid"
    Start-Sleep -Seconds $Settle
    $main = [IntPtr]::Zero; $best = -1
    foreach ($h in [W]::TopLevel($appPid, $true)) {
        $r = [W]::Frame($h); $a = ($r.Right - $r.Left) * ($r.Bottom - $r.Top)
        Log ("top 0x{0:X} cls={1} frame={2} title={3}" -f $h.ToInt64(), [W]::Cls($h), [W]::R($h), [W]::Txt($h))
        if ($a -gt $best -and [W]::Cls($h) -eq 'WinUIDesktopWin32WindowClass') { $best = $a; $main = $h }
    }
    if ($main -eq [IntPtr]::Zero) { throw 'NO MAIN WINDOW' }
    $wins['main'] = $main
    [W]::Fg($main); Start-Sleep -Milliseconds 800
    foreach ($raw in (Get-Content $Plan)) {
        $line = $raw.Trim()
        if (-not $line -or $line.StartsWith('#')) { continue }
        $a = $line -split '\s+', 4
        Log "step: $line"
        try {
            switch ($a[0]) {
                'tab' { $el = Find 'main' $UIA::AutomationIdProperty $a[1]; if ($el) { ClickEl $el; Start-Sleep -Milliseconds 1500 } else { Log "  not found" } }
                'click' { $el = Find $a[1] $UIA::NameProperty (($a[2..($a.Count - 1)]) -join ' '); if ($el) { ClickEl $el; Start-Sleep -Milliseconds 1500 } else { Log "  not found" } }
                'rail-open' {
                    $rc = [W]::Frame($wins['rail'])
                    [W]::Fg($wins['main']); Start-Sleep -Milliseconds 400
                    [W]::Move($rc.Right - 2, [int](($rc.Top + $rc.Bottom) / 2)); Start-Sleep -Milliseconds 150
                    [W]::Click(); Start-Sleep -Milliseconds 1500
                    Log ("  rail frame after click: {0}" -f [W]::R($wins['rail']))
                }
                'clicklast' {
                    $all = $UIA::FromHandle($wins[$a[1]]).FindAll($TS::Descendants, (New-Object System.Windows.Automation.PropertyCondition($UIA::NameProperty, (($a[2..($a.Count - 1)]) -join ' '))))
                    if ($all.Count -gt 0) { ClickEl $all[$all.Count - 1]; Start-Sleep -Milliseconds 1500 } else { Log "  not found" }
                }
                'toggle' {
                    $el = Find $a[1] $UIA::NameProperty (($a[2..($a.Count - 1)]) -join ' ')
                    if ($el) { try { $el.GetCurrentPattern([System.Windows.Automation.ScrollItemPattern]::Pattern).ScrollIntoView() } catch { }; $el.GetCurrentPattern([System.Windows.Automation.TogglePattern]::Pattern).Toggle(); Start-Sleep -Milliseconds 1500; Log ("  toggled to " + $el.GetCurrentPattern([System.Windows.Automation.TogglePattern]::Pattern).Current.ToggleState) } else { Log "  not found" }
                }
                'invoke' {
                    $el = Find $a[1] $UIA::NameProperty (($a[2..($a.Count - 1)]) -join ' ')
                    if ($el) { $el.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke(); Start-Sleep -Milliseconds 2000 } else { Log "  not found" }
                }
                'clickaid' { $el = Find $a[1] $UIA::AutomationIdProperty $a[2]; if ($el) { ClickEl $el; Start-Sleep -Milliseconds 1500 } else { Log "  not found" } }
                'select' {
                    $el = Find $a[1] $UIA::NameProperty (($a[2..($a.Count - 1)]) -join ' ')
                    if ($el) { $o = $null; if ($el.TryGetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern, [ref]$o)) { $o.Select() } else { ClickEl $el }; Start-Sleep -Milliseconds 1500 } else { Log "  not found" }
                }
                'key' {
                    [W]::Fg($wins[$a[1]]); Start-Sleep -Milliseconds 300
                    $vk = [Convert]::ToByte($a[2], 16)
                    $mod = if ($a.Count -gt 3) { $a[3] } else { '' }
                    if ($mod -like 'ctrl*') { [W]::keybd_event(0x11, 0, 0, [UIntPtr]::Zero) }
                    [W]::Key($vk, ($mod -like '*shift'))
                    if ($mod -like 'ctrl*') { [W]::keybd_event(0x11, 0, 2, [UIntPtr]::Zero) }
                    Start-Sleep -Milliseconds 1200
                }
                'newwin' { $h = NewestWindow; $wins[$a[1]] = $h; Log ("  {0} = 0x{1:X} cls={2} frame={3} title={4}" -f $a[1], $h.ToInt64(), [W]::Cls($h), [W]::R($h), [W]::Txt($h)) }
                'flyout' {
                    $tray = [W]::TrayHost($appPid)
                    $taskbar = [W]::FindWindow('Shell_TrayWnd', $null)
                    $tb = [W]::Frame($taskbar)
                    [W]::Move($tb.Right - 120, [int](($tb.Top + $tb.Bottom) / 2))
                    [W]::PostMessage($tray, 0x8001, [IntPtr]::Zero, [IntPtr](0x10000 -bor 0x400)) | Out-Null
                    Start-Sleep -Milliseconds 1500
                    $h = NewestWindow
                    $wins['flyout'] = $h
                    Log ("  flyout = 0x{0:X} frame={1}" -f $h.ToInt64(), [W]::R($h))
                }
                'scroll' { ScrollSweep $a[1] $a[2] }
                'audit' { $max = if ($a.Count -gt 3) { [int]$a[3] } else { 120 }; Audit $a[1] $a[2] $max }
                'snap' { $h = $wins[$a[1]]; $p2 = Join-Path $OutDir "$Tag-$($a[2]).png"; Log ("  " + [W]::Capture($h, $p2)) }
                'state' {
                    $h = $wins[$a[1]]
                    Log ("  state {0}: visible={1} fg={2} fgcls={3} focused={4}" -f $a[2], [W]::IsWindowVisible($h), ([W]::GetForegroundWindow() -eq $h), [W]::Cls([W]::GetForegroundWindow()), (Desc (Focused)))
                }
                'sleep' { Start-Sleep -Milliseconds ([int]$a[1]) }
                'size' {
                    [Sz]::Size($wins[$a[1]], [int]$a[2], [int]$a[3]) | Out-Null; Start-Sleep -Milliseconds 1500
                    Log ("  {0} frame now {1}" -f $a[1], [W]::R($wins[$a[1]]))
                }
                'expect' {
                    # expect <win> shown|hidden, or expect <win> has|lacks <AutomationId>
                    # A window that never opened passes nothing: no handle fails.
                    $h = if ($wins.ContainsKey($a[1])) { $wins[$a[1]] } else { [IntPtr]::Zero }
                    $ok = switch ($a[2]) {
                        'shown' { $h -ne [IntPtr]::Zero -and [W]::IsWindowVisible($h) }
                        'hidden' { $h -ne [IntPtr]::Zero -and -not [W]::IsWindowVisible($h) }
                        'has' { $null -ne (Find $a[1] $UIA::AutomationIdProperty $a[3]) }
                        'lacks' { $null -eq (Find $a[1] $UIA::AutomationIdProperty $a[3]) }
                        default { $false }
                    }
                    Log ("  EXPECT {0}: {1}" -f $(if ($ok) { 'PASS' } else { 'FAIL' }), ($a[1..($a.Count - 1)] -join ' '))
                }
                'front' { [W]::Fg($wins[$a[1]]); Start-Sleep -Milliseconds 600 }
                'exists' {
                    $el = Find $a[1] $UIA::AutomationIdProperty $a[2]
                    Log ("  exists {0}: {1}" -f $a[2], $(if ($el) { "yes offscreen=$($el.Current.IsOffscreen)" } else { 'no' }))
                }
                'focusname' {
                    $el = Find $a[1] $UIA::NameProperty (($a[2..($a.Count - 1)]) -join ' ')
                    if ($el) { try { $el.SetFocus(); Log ("  focused " + (Desc (Focused))) } catch { Log "  SetFocus failed: $_" } } else { Log "  not found" }
                    Start-Sleep -Milliseconds 600
                }
                'selected' {
                    $el = Find $a[1] $UIA::NameProperty (($a[2..($a.Count - 1)]) -join ' ')
                    $o = $null
                    if ($el -and $el.TryGetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern, [ref]$o)) { Log ("  selected '{0}': {1}" -f $el.Current.Name, $o.Current.IsSelected) } else { Log "  no selection item" }
                }
                'hc' {
                    $ok = [Hc]::Set(($a[1] -eq 'on'), ''); Start-Sleep -Seconds 8
                    Log "  high contrast now=$([Hc]::On()) set=$ok"
                }
                'theme' {
                    $k = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize'
                    $v = [int]($a[1] -eq 'light')
                    Set-ItemProperty $k -Name AppsUseLightTheme -Value $v -Type DWord
                    Set-ItemProperty $k -Name SystemUsesLightTheme -Value $v -Type DWord
                    [Bc]::Send() | Out-Null
                    Start-Sleep -Seconds 4
                    Log "  theme now $($a[1])"
                }
                default { Log "  unknown step" }
            }
        } catch { Log "  step failed: $_" }
    }
} catch { Log "FAILED: $_" }
finally {
    if ($appPid) { Get-Process -Id $appPid -ErrorAction SilentlyContinue | Stop-Process -Force }
    if (-not $hcBefore -and [Hc]::On() -and -not $HighContrast) { [Hc]::Set($false, '') | Out-Null; Start-Sleep -Seconds 4; Log "high contrast left on by a step: restored off now=$([Hc]::On())" }
    if ($HighContrast -and -not $hcBefore) {
        if ($want -and $schemeBefore -and $want -ne $schemeBefore) { [Hc]::Set($true, $schemeBefore) | Out-Null; Start-Sleep -Seconds 4 }
        $ok = [Hc]::Set($false, ''); Start-Sleep -Seconds 4; Log "high contrast restored off=$ok now=$([Hc]::On()) scheme='$(SchemeReg)'"
    }
    Log 'done'
}
