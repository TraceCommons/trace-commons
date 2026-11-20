# CPU and drawing profile: the in-session driver. Started by win-perf-run.ps1
# as a hidden /IT task in the console session; never run it from SSH
# (session 0 has no desktop). See windows/docs/perf-profile.md.
#
# Launches the Debug app on a recorded sample set inside a throttling job
# object, follows a plan file, and writes one line per 10 s measurement
# window to <OutDir>\<Tag>.perf.tsv. Nothing outside the app is captured:
# windows are found by the app's pid and captured by handle.
#
# Plan steps (one per line; '#' starts a comment; {throttle} is replaced
# by -Throttle, so one plan runs at every level):
#   launch <set> <throttle> [step=<FirstRunStep>] [extra=<SampleExtra>] [env=NAME=VALUE]
#                                  start the app (throttle: full, 2core,
#                                  2core-half, 1core-half; see $Throttles)
#   size <w> <h>                   resize the main window (top-left corner)
#   tab <AutomationId>             click a main-window element by AutomationId
#   click <Name>                   click a main-window element by Name
#   select <Name>                  select a tab or list item by Name
#   key <vk-hex> [ctrl]            press a key in the main window
#   animations on|off              the system's client-area animation setting
#   minimize | restore             the main window
#   unfocus | focus                the desktop takes the focus (the app stays on
#                                  screen) | the app's window takes it back
#   hide                           close the main window (it hides to the tray)
#   flyout                         open the tray flyout (it becomes the measured window)
#   measure <label> <n> [secs]     n windows of secs (default 10): see below
#   hover <label> <n> <Name>...    as measure, while the pointer passes in and
#                                  out of each named control every 300 ms
#   press <label> <n> <Name>[,<Name>...]
#                                  as measure, while the pointer presses the
#                                  named controls in turn, one every 600 ms
#   toggle <Name>                  toggle a switch (or click it)
#   window newest | window main    measure the app's newest window, or the main one
#   rail-open                      click the edge rail's zone; the rail becomes the measured window
#   respond <label> <Name>         click a control and time until the window's
#                                  pixels change (PrintWindow, so ~coarse)
#   snap <label>                   capture the measured window
#   wpr start | wpr stop <label>   an ETW trace (wpr's CPU and DotNET profiles),
#                                  started and stopped by the elevated runner,
#                                  written to <OutDir>\<Tag>-<label>.etl
#   sleep <ms>
#   quit                           end the app's job (and the app)
#
# A measurement line: label, window, seconds since the step, app CPU ms,
# UI-thread CPU ms, the busiest other thread's ms, DWM ms (-1 if unreadable),
# the machine's busy ms on every core, the harness's own ms, frames DWM
# composed, UI round trips (count p50 p95 max over-50ms) and the app's
# working set and private bytes in MB. No UI Automation call is made inside
# a window: those run on the app's UI thread and would be measured.
param(
    [Parameter(Mandatory = $true)] [string] $Exe,
    [Parameter(Mandatory = $true)] [string] $OutDir,
    [Parameter(Mandatory = $true)] [string] $Tag,
    [Parameter(Mandatory = $true)] [string] $Plan,
    [string] $Throttle = 'full',
    [int] $Settle = 14
)
$ErrorActionPreference = 'Stop'
$log = Join-Path $OutDir "$Tag.drive.log"
$tsv = Join-Path $OutDir "$Tag.perf.tsv"
# The app's look log (TRACE_COMMONS_LOOK_LOG): clock lines and, once a
# second, each drawn surface's frame line (LookFrames), read per window.
$lookLog = Join-Path $OutDir "$Tag.look.log"
Remove-Item -Force -EA SilentlyContinue $lookLog
function Log([string] $s) {
    $line = "[{0}] {1}`r`n" -f (Get-Date).ToString('HH:mm:ss.fff'), $s
    for ($i = 0; $i -lt 40; $i++) {
        try { [IO.File]::AppendAllText($log, $line, [Text.Encoding]::UTF8); return } catch { Start-Sleep -Milliseconds 50 }
    }
}
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -AssemblyName WindowsBase
. (Join-Path $PSScriptRoot '..\a11y\win-a11y-helpers.ps1')
Add-Type -Path (Join-Path $PSScriptRoot 'PerfProbe.cs')
[W]::Dpi()
$UIA = [System.Windows.Automation.AutomationElement]
$TS = [System.Windows.Automation.TreeScope]

# Cores 0-1 for the app at every throttle below "full"; the harness runs on
# core 3, so it never takes the app's cores. A hard cap is a share of all
# four cores of the D4s_v3: 25% is one core's worth, spread over the two.
$Throttles = @{
    'full'       = @{ Affinity = 0x0; Rate = 0 }
    '2core'      = @{ Affinity = 0x3; Rate = 0 }
    '2core-half' = @{ Affinity = 0x3; Rate = 25 }
    '1core-half' = @{ Affinity = 0x1; Rate = 12.5 }
}
[PerfProbe]::PinSelf(0x8)
$self = $PID
$appPid = 0
$main = [IntPtr]::Zero
$name = [IO.Path]::GetFileNameWithoutExtension($Exe)

function Find([string] $prop, [string] $value) {
    $p = if ($prop -eq 'aid') { $UIA::AutomationIdProperty } else { $UIA::NameProperty }
    return $UIA::FromHandle($main).FindFirst($TS::Descendants, (New-Object System.Windows.Automation.PropertyCondition($p, $value)))
}
function Centre($el) { $r = $el.Current.BoundingRectangle; return @([int]($r.X + $r.Width / 2), [int]($r.Y + $r.Height / 2)) }
function Park() { $f = [W]::Frame($main); [W]::Move($f.Right + 40, $f.Bottom - 6) }
function ClickEl($el) {
    $c = Centre $el
    [W]::Move($c[0], $c[1]); Start-Sleep -Milliseconds 150; [W]::Click(); Start-Sleep -Milliseconds 200
    Park
}
function MainWindow() {
    $best = [IntPtr]::Zero; $area = -1
    foreach ($h in [W]::TopLevel($appPid, $true)) {
        $r = [W]::Frame($h); $a = ($r.Right - $r.Left) * ($r.Bottom - $r.Top)
        if ($a -gt $area) { $area = $a; $best = $h }
    }
    return $best
}

function Snapshot() {
    return @{
        T = [Diagnostics.Stopwatch]::GetTimestamp()
        App = [PerfProbe]::ProcessMs($appPid)
        Threads = [PerfProbe]::ThreadMs($appPid)
        Dwm = [PerfProbe]::NamedMs('dwm')
        Machine = [PerfProbe]::MachineBusyMs()
        Self = [PerfProbe]::ProcessMs($self)
        Frames = [PerfProbe]::DwmFrames()
        Utc = [DateTime]::UtcNow
    }
}
function Row([string] $label, [int] $win, [double] $since, $a, $b, [string] $ping) {
    $ui = [PerfProbe]::UiThread($main)
    $uiMs = 0; $other = 0; $otherId = 0
    $groups = @{}
    foreach ($k in $b.Threads.Keys) {
        $d = $b.Threads[$k] - $(if ($a.Threads.ContainsKey($k)) { $a.Threads[$k] } else { 0 })
        if ($k -eq $ui) { $uiMs = $d; continue }
        if ($d -gt $other) { $other = $d; $otherId = $k }
        if ($d -gt 0) { $g = [PerfProbe]::ThreadModule($appPid, $k); $groups[$g] = $(if ($groups.ContainsKey($g)) { $groups[$g] } else { 0 }) + $d }
    }
    $byModule = ($groups.GetEnumerator() | Sort-Object Value -Descending | Select-Object -First 5 | ForEach-Object { '{0}={1:0}' -f $_.Key, $_.Value }) -join ','
    if (-not $byModule) { $byModule = '-' }
    $secs = ($b.T - $a.T) / [Diagnostics.Stopwatch]::Frequency
    $dwm = if ($a.Dwm -ge 0 -and $b.Dwm -ge 0) { [int]($b.Dwm - $a.Dwm) } else { -1 }
    $dwmFrames = if ($a.Frames -ge 0 -and $b.Frames -ge 0) { $b.Frames - $a.Frames } else { -1 }
    $mem = [PerfProbe]::Memory($appPid)
    # Each surface's frames in the window, from the app's look log (LookFrames).
    $fr = @{}
    if (Test-Path $lookLog) {
        foreach ($l in [IO.File]::ReadAllLines($lookLog)) {
            if ($l -notmatch '^(\S+) frames (\S+) n=(\d+) over=([\d.]+)s fps=[\d.]+ draw_mean=([\d.]+)ms draw_max=([\d.]+)ms gap_max=([\d.]+)ms') { continue }
            $t = [DateTime]::Parse($matches[1]).ToUniversalTime()
            if ($t -le $a.Utc -or $t -gt $b.Utc) { continue }
            $k = $matches[2]
            if (-not $fr.ContainsKey($k)) { $fr[$k] = @{ N = 0; Over = 0.0; Draw = 0.0; Gap = 0.0 } }
            $e = $fr[$k]; $n = [int]$matches[3]
            $e.N += $n; $e.Over += [double]$matches[4]; $e.Draw += $n * [double]$matches[5]
            $e.Gap = [Math]::Max($e.Gap, [double]$matches[7])
        }
    }
    $frames = ($fr.GetEnumerator() | Sort-Object Name | ForEach-Object { '{0}:fps={1:0.0},draw={2:0.0}ms,gap={3:0}ms' -f $_.Name, ($_.Value.N / [Math]::Max(0.001, $_.Value.Over)), ($_.Value.Draw / [Math]::Max(1, $_.Value.N)), $_.Value.Gap }) -join ' '
    if (-not $frames) { $frames = '-' }
    $line = "{0}`t{1}`t{2:0.0}`t{3:0}`t{4:0}`t{5:0}`t{6}`t{7:0}`t{8:0}`t{9}`t{10}`t{11}`t{12}`t{13:0.0}`t{14}`t{15}" -f $Tag, $label, $win, ($b.App - $a.App), $uiMs, $other, $dwm, ($b.Machine - $a.Machine), ($b.Self - $a.Self), $dwmFrames, ($ping -replace ' ', "`t"), ($mem -replace ' ', "`t"), $since, $secs, $byModule, $frames
    [IO.File]::AppendAllText($tsv, $line + "`r`n")
    Log ("  {0} w{1} app={2:0} ui={3:0} other={4:0}(tid {5}) dwm={6} machine={7:0} self={8:0} ping[n p50 p95 max >50]={10} mem[ws priv]={11} threads={12} frames={13}" -f $label, $win, ($b.App - $a.App), $uiMs, $other, $otherId, $dwm, ($b.Machine - $a.Machine), ($b.Self - $a.Self), $dwmFrames, $ping, $mem, $byModule, $frames)
}

# Runs $n windows of $secs, calling $activity (if any) in a loop meanwhile.
function MeasureWindows([string] $label, [int] $n, [double] $secs, [scriptblock] $activity) {
    [PerfProbe]::StartPing($main, 50)
    $start = [Diagnostics.Stopwatch]::StartNew()
    try {
        for ($w = 1; $w -le $n; $w++) {
            $since = $start.Elapsed.TotalSeconds
            [void][PerfProbe]::TakePing()
            $a = Snapshot
            $until = $start.Elapsed.TotalSeconds + $secs
            while ($start.Elapsed.TotalSeconds -lt $until) {
                if ($activity) { & $activity } else { Start-Sleep -Milliseconds 100 }
            }
            $b = Snapshot
            Row $label $w $since $a $b ([PerfProbe]::TakePing())
        }
    } finally { [PerfProbe]::StopPing() }
}

$animBefore = [W]::Anim()
Log "dwm timing: $([PerfProbe]::DwmDebug())"
Log "== $Tag exe=$Exe display=$([W]::GetSystemMetrics(0))x$([W]::GetSystemMetrics(1)) cores=$([Environment]::ProcessorCount) animations=$animBefore session=$((Get-Process -Id $PID).SessionId)"
"tag`tlabel`twindow`tapp_ms`tui_ms`tother_ms`tdwm_ms`tmachine_ms`tharness_ms`tdwm_frames`tping_n`tping_p50`tping_p95`tping_max`tping_over50`tws_mb`tprivate_mb`tsince_s`tsecs`tother_threads_by_start_module`tframes_by_surface" | Set-Content -Path $tsv -Encoding ascii
try {
    foreach ($raw in Get-Content $Plan) {
        $line = ($raw -replace '#.*$', '').Trim().Replace('{throttle}', $Throttle)
        if (-not $line) { continue }
        $a = $line -split '\s+'
        Log "> $line"
        try {
            switch ($a[0]) {
                'launch' {
                    if (Get-Process -Name $name -EA SilentlyContinue) { throw "$name already running" }
                    $t = $Throttles[$a[2]]
                    if (-not $t) { throw "unknown throttle $($a[2])" }
                    $env:TRACE_COMMONS_SAMPLE = $a[1]
                    $env:TRACE_COMMONS_LOOK_LOG = $lookLog
                    $set = @('TRACE_COMMONS_SAMPLE', 'TRACE_COMMONS_LOOK_LOG')
                    foreach ($opt in $a[3..($a.Count)]) {
                        if ($opt -like 'step=*') { $env:TRACE_COMMONS_FIRST_RUN_STEP = $opt.Substring(5); $set += 'TRACE_COMMONS_FIRST_RUN_STEP' }
                        if ($opt -like 'extra=*') { $env:TRACE_COMMONS_SAMPLE_EXTRA = $opt.Substring(6); $set += 'TRACE_COMMONS_SAMPLE_EXTRA' }
                        if ($opt -like 'env=*=*') { $kv = $opt.Substring(4) -split '=', 2; [Environment]::SetEnvironmentVariable($kv[0], $kv[1]); $set += $kv[0] }
                    }
                    $appPid = [PerfProbe]::StartThrottled($Exe, [uint64]$t.Affinity, [double]$t.Rate)
                    foreach ($v in $set) { [Environment]::SetEnvironmentVariable($v, $null) }
                    $sw = [Diagnostics.Stopwatch]::StartNew()
                    $main = [IntPtr]::Zero
                    # A launch's first seconds: CPU from start to the first window and to the settle.
                    while ($sw.Elapsed.TotalSeconds -lt 60 -and $main -eq [IntPtr]::Zero) { Start-Sleep -Milliseconds 200; $main = MainWindow }
                    $shownAt = $sw.Elapsed.TotalSeconds
                    Start-Sleep -Seconds $Settle
                    $main = MainWindow
                    [W]::Fg($main); Park
                    Log ("  pid={0} throttle={1} (affinity 0x{2:X}, cap {3}%) first window {4:0.0} s; main=0x{5:X} {6} cpu after settle={7:0} ms" -f $appPid, $a[2], $t.Affinity, $t.Rate, $shownAt, $main.ToInt64(), [W]::R($main), [PerfProbe]::ProcessMs($appPid))
                }
                'size' { [PerfProbe]::SetWindowPos($main, [IntPtr]::Zero, 0, 0, [int]$a[1], [int]$a[2], 0x0004) | Out-Null; Start-Sleep -Milliseconds 1500; Log "  frame $([W]::R($main))" }
                'tab' { $el = Find 'aid' $a[1]; if ($el) { ClickEl $el; Start-Sleep -Milliseconds 1500 } else { Log '  not found' } }
                'click' { $el = Find 'name' ($a[1..($a.Count - 1)] -join ' '); if ($el) { ClickEl $el; Start-Sleep -Milliseconds 1500 } else { Log '  not found' } }
                'select' {
                    $el = Find 'name' ($a[1..($a.Count - 1)] -join ' ')
                    if ($el) { $o = $null; if ($el.TryGetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern, [ref]$o)) { $o.Select() } else { ClickEl $el }; Start-Sleep -Milliseconds 1500 } else { Log '  not found' }
                }
                'key' {
                    [W]::Fg($main); Start-Sleep -Milliseconds 300
                    if ($a.Count -gt 2 -and $a[2] -eq 'ctrl') { [W]::keybd_event(0x11, 0, 0, [UIntPtr]::Zero) }
                    [W]::Key([Convert]::ToByte($a[1], 16), $false)
                    if ($a.Count -gt 2 -and $a[2] -eq 'ctrl') { [W]::keybd_event(0x11, 0, 2, [UIntPtr]::Zero) }
                    Start-Sleep -Milliseconds 1200
                }
                'animations' { [W]::SetAnim($a[1] -eq 'on'); Start-Sleep -Milliseconds 1500; Log "  animations now $([W]::Anim())" }
                'unfocus' {
                    # Another window takes the focus (the desktop); the app stays on screen.
                    $desk = [W]::FindWindow('Progman', $null)
                    [W]::Fg($desk); Start-Sleep -Milliseconds 1500
                    Log "  foreground now 0x$([W]::GetForegroundWindow().ToString('X')) $([W]::Cls([W]::GetForegroundWindow())); app visible=$([W]::IsWindowVisible($main))"
                }
                'focus' { [W]::Fg($main); Start-Sleep -Milliseconds 1500; Log "  foreground is the app: $([W]::GetForegroundWindow() -eq $main)" }
                'minimize' { [W]::ShowWindow($main, 6) | Out-Null; Start-Sleep -Milliseconds 1500; Log "  iconic=$([W]::IsIconic($main))" }
                'restore' { [W]::ShowWindow($main, 9) | Out-Null; [W]::Fg($main); Start-Sleep -Milliseconds 1500; Log "  iconic=$([W]::IsIconic($main)) visible=$([W]::IsWindowVisible($main))" }
                'hide' { [W]::PostMessage($main, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null; Start-Sleep -Milliseconds 2000; Log "  visible=$([W]::IsWindowVisible($main)) alive=$([bool](Get-Process -Id $appPid -EA SilentlyContinue))" }
                'flyout' {
                    $tray = [W]::TrayHost($appPid)
                    $before = [W]::TopLevel($appPid, $true)
                    $taskbar = [W]::FindWindow('Shell_TrayWnd', $null); $tb = [W]::Frame($taskbar)
                    [W]::Move($tb.Right - 120, [int](($tb.Top + $tb.Bottom) / 2))
                    [W]::PostMessage($tray, 0x8001, [IntPtr]::Zero, [IntPtr](0x10000 -bor 0x400)) | Out-Null
                    Start-Sleep -Milliseconds 1500
                    foreach ($h in [W]::TopLevel($appPid, $true)) { if (-not ($before -contains $h)) { $main = $h } }
                    Log "  measured window now 0x$($main.ToString('X')) $([W]::Cls($main)) $([W]::R($main))"
                }
                'measure' { MeasureWindows $a[1] ([int]$a[2]) $(if ($a.Count -gt 3) { [double]$a[3] } else { 10 }) $null }
                'hover' {
                    $pts = @(); foreach ($n in (($a[3..($a.Count - 1)] -join ' ') -split ',')) { $el = Find 'name' $n.Trim(); if ($el) { $pts += , (Centre $el) } else { Log "  not found: $n" } }
                    $f = [W]::Frame($main); $out = @(($f.Right + 40), ($f.Bottom - 6)); $script:hi = 0
                    MeasureWindows $a[1] ([int]$a[2]) 10 {
                        $p = $pts[$script:hi % $pts.Count]; $script:hi++
                        [W]::Move($p[0], $p[1]); Start-Sleep -Milliseconds 300
                        [W]::Move($out[0], $out[1]); Start-Sleep -Milliseconds 300
                    }
                }
                'press' {
                    $pts = @(); foreach ($n in (($a[3..($a.Count - 1)] -join ' ') -split ',')) { $el = Find 'name' $n.Trim(); if ($el) { $pts += , (Centre $el) } else { Log "  not found: $n" } }
                    if ($pts.Count -eq 0) { break }
                    $script:pi = 0
                    MeasureWindows $a[1] ([int]$a[2]) 10 {
                        $p = $pts[$script:pi % $pts.Count]; $script:pi++
                        [W]::Move($p[0], $p[1]); Start-Sleep -Milliseconds 200
                        [W]::mouse_event(2, 0, 0, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 200
                        [W]::mouse_event(4, 0, 0, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 200
                    }
                    Park
                }
                'window' {
                    # newest: the app's newest visible top-level window becomes the measured one; main: back.
                    if ($a[1] -eq 'main') { $main = $script:mainWindow } else {
                        $script:mainWindow = if ($script:mainWindow) { $script:mainWindow } else { $main }
                        foreach ($h in [W]::TopLevel($appPid, $true)) { if ($h -ne $script:mainWindow) { $main = $h } }
                    }
                    Log "  measured window 0x$($main.ToString('X')) $([W]::Cls($main)) $([W]::R($main))"
                }
                'toggle' {
                    $el = Find 'name' ($a[1..($a.Count - 1)] -join ' ')
                    if ($el) { $o = $null; if ($el.TryGetCurrentPattern([System.Windows.Automation.TogglePattern]::Pattern, [ref]$o)) { $o.Toggle() } else { ClickEl $el }; Start-Sleep -Milliseconds 1500 } else { Log '  not found' }
                }
                'rail-open' {
                    # The edge rail: a click on its zone opens it; it becomes the measured window.
                    $rail = [IntPtr]::Zero
                    foreach ($h in [W]::TopLevel($appPid, $true)) { $r = [W]::Frame($h); if (($r.Right - $r.Left) -lt 80 -and $h -ne $main) { $rail = $h } }
                    if ($rail -eq [IntPtr]::Zero) { Log '  no rail window'; break }
                    $rc = [W]::Frame($rail)
                    [W]::Move($rc.Right - 2, [int](($rc.Top + $rc.Bottom) / 2)); Start-Sleep -Milliseconds 150; [W]::Click(); Start-Sleep -Milliseconds 1500
                    $script:mainWindow = if ($script:mainWindow) { $script:mainWindow } else { $main }
                    $main = $rail
                    Log "  rail 0x$($rail.ToString('X')) now $([W]::R($rail))"
                }
                'respond' {
                    $el = Find 'name' ($a[2..($a.Count - 1)] -join ' ')
                    if (-not $el) { Log '  not found'; break }
                    $c = Centre $el
                    [W]::Move($c[0], $c[1]); Start-Sleep -Milliseconds 400
                    $base = [W]::Grab($main)
                    $sw = [Diagnostics.Stopwatch]::StartNew(); [W]::Click()
                    $changed = -1
                    while ($sw.ElapsedMilliseconds -lt 5000) {
                        $now = [W]::Grab($main); $d = [W]::Diff($base, $now); $now.Dispose()
                        if ($d -gt 200) { $changed = $sw.ElapsedMilliseconds; break }
                    }
                    $base.Dispose(); Park
                    Log "  respond $($a[1]): first change after $changed ms"
                    [IO.File]::AppendAllText($tsv, ("{0}`t{1}`trespond`t{2}`r`n" -f $Tag, $a[1], $changed))
                }
                'snap' { Log ("  " + [W]::Capture($main, (Join-Path $OutDir "$Tag-$($a[1]).png"))) }
                'wpr' {
                    # start | stop <label>: the runner (elevated, session 0) runs wpr.exe.
                    $ack = Join-Path $OutDir "$Tag.wpr-ack"
                    Remove-Item -Force -EA SilentlyContinue $ack
                    Set-Content -Path (Join-Path $OutDir "$Tag.wpr-request") -Value ($a[1..($a.Count - 1)] -join ' ')
                    $sw = [Diagnostics.Stopwatch]::StartNew()
                    while (-not (Test-Path $ack) -and $sw.Elapsed.TotalSeconds -lt 180) { Start-Sleep -Milliseconds 200 }
                    Log "  $(if (Test-Path $ack) { (Get-Content $ack -Raw).Trim() } else { 'no answer from the runner' }) after $([int]$sw.Elapsed.TotalSeconds) s"
                }
                'sleep' { Start-Sleep -Milliseconds ([int]$a[1]) }
                'quit' { [PerfProbe]::EndJob(); Start-Sleep -Seconds 2; Log "  app alive after job end: $([bool](Get-Process -Id $appPid -EA SilentlyContinue))" }
                default { Log "  unknown step $($a[0])" }
            }
        } catch { Log "  STEP FAILED: $_" }
    }
} finally {
    [PerfProbe]::StopPing()
    [PerfProbe]::EndJob()
    Get-Process -Name $name -EA SilentlyContinue | Where-Object { $_.Path -eq $Exe } | Stop-Process -Force
    [W]::SetAnim($animBefore)
    Log "animations restored to $([W]::Anim())"
    Log 'done'
}
