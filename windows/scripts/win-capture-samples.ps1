# Capture the debug app on each recorded sample set, named for the parity
# gallery, and check that no sample launch changed this machine's own state.
# Runs ON the Windows test box (decision W1-D1: captures are made there and
# never uploaded from CI).
#
# Slice W1 of docs/superpowers/specs/2026-11-18-windows-mac-parity-design.md.
# The Mac's debug hook (macos/Sources/TraceCommonsApp/DebugScreenshot.swift)
# and the Linux captures write the same names, and scripts/parity/gallery.sh
# pairs them:
#
#     <set>-<tab>-<panes>-<appearance>.png
#
# The lists below are that grammar (scripts/parity/gallery.py, NAME) and the
# Mac app's sample sets (SampleDaemon.Sets). SampleDaemonTests reads this file
# and fails if either drifts.
#
# HOW A SAMPLE REACHES THE APP. win-capture.ps1 launches through a scheduled
# task in the interactive session and sets TRACE_COMMONS_SAMPLE for that
# launch only (-Environment). The app follows the Windows app theme
# (HKCU AppsUseLightTheme) at start, so the value is set before each launch
# and put back at the end, or removed if there was none.
#
# WHAT IT CAN CAPTURE TODAY. The window opens on the contribution queue in
# sample mode, which is the Mac's Traces tab with no map or inspector, so each
# launch yields one capture, <set>-traces-main-<appearance>.png. The window has
# no Home tab, map or inspector to steer to until slice W2; the other tabs and
# panes are in the lists for the names alone. Captures are of the app's window
# only, by its handle (win-capture.ps1), sized with -Width and -Height.
#
# THE STATE CHECK. Before the first launch and after every launch, the URL
# scheme key, the Run values and the app's state directory are read. Any
# change fails the run and is written to state-check.txt. The throwaway
# sample directories under %TEMP% are counted once, at the end: each launch
# removes those of stopped launches, so no more than the last two may stay. A sample set is then launched twice at once:
# two windows must result, because a sample launch never takes part in
# single-instancing (it would otherwise hand itself to a running app).
#
# The app must be a Debug build: a Release build has no sample client and
# would start a real daemon, so the script refuses one. It also refuses to run
# while any copy of the app is running, rather than stopping it: that copy may
# be someone's real app. Never sign in, enroll or share on the box.
#
# Usage (on the box):
#   powershell -ExecutionPolicy Bypass -File win-capture-samples.ps1 `
#       -Exe C:\src\trace-commons\windows\src\TraceCommons.App\bin\x64\Debug\net8.0-windows10.0.19041.0\win-x64\TraceCommons.exe
# Then copy -OutDir back and run scripts/parity/gallery.sh <mac-dir> <win-dir>.

[CmdletBinding()]
param(
    # The Debug build of the app.
    [Parameter(Mandatory = $true)]
    [string] $Exe,

    # Where the named captures land. Writable by the unelevated capture task;
    # see win-capture.ps1.
    [string] $OutDir = 'C:\tc-captures\windows',

    # A subset of the sets below, for a quicker run.
    [string[]] $Only = @(),

    [int] $SettleSeconds = 10,

    # The window's visible frame in physical pixels; 0 keeps the app's size.
    [int] $Width = 0,
    [int] $Height = 0
)

$ErrorActionPreference = 'Stop'

# The gallery's grammar and the Mac's sets. Keep in step with gallery.py and
# SampleDaemon.cs; SampleDaemonTests checks both.
$Sets = @('empty', 'normalDay', 'busyQueue', 'coreDown', 'unknownCounts', 'heldSessions', 'armedFolder')
$Tabs = @('home', 'inference', 'traces', 'insights')
$Panes = @('main', 'main+map', 'main+inspector', 'main+map+inspector')
$Appearances = @('light', 'dark')

# What a sample launch opens on, until W2 adds a way to steer the window.
$OpeningTab = 'traces'
$OpeningPanes = 'main'

function Get-CaptureName {
    param([string] $Set, [string] $Tab, [string] $Panes, [string] $Appearance)
    if ($Sets -notcontains $Set) { throw "not a sample set: $Set" }
    if ($Tabs -notcontains $Tab) { throw "not a tab: $Tab" }
    if ($script:Panes -notcontains $Panes) { throw "not a pane set: $Panes" }
    if ($Appearances -notcontains $Appearance) { throw "not an appearance: $Appearance" }
    return "$Set-$Tab-$Panes-$Appearance.png"
}

# ---------------------------------------------------------------------------
# Refuse anything but a Debug build, and refuse to touch a running app.
# ---------------------------------------------------------------------------
if (-not (Test-Path $Exe)) {
    throw "Executable not found: $Exe"
}
$Exe = (Resolve-Path $Exe).Path
$exeDir = Split-Path -Parent $Exe
$interop = Join-Path $exeDir 'TraceCommons.Interop.dll'
if ($Exe -notmatch '\\Debug\\') {
    throw "not a Debug build (no \Debug\ in the path): $Exe"
}
if (-not (Test-Path (Join-Path $exeDir 'RecordedSamples'))) {
    throw "no RecordedSamples beside ${Exe}: not a Debug build"
}
if (-not (Test-Path $interop) -or -not (Select-String -Path $interop -Pattern 'SampleDaemon' -SimpleMatch -Quiet)) {
    throw "$interop has no SampleDaemon type: not a Debug build"
}
$processName = [IO.Path]::GetFileNameWithoutExtension($Exe)
$running = @(Get-Process -Name $processName -ErrorAction SilentlyContinue)
if ($running.Count -gt 0) {
    throw "$processName is already running (pid $(($running | ForEach-Object { $_.Id }) -join ', ')); stop it first. This script never stops an app it did not start."
}

$capture = Join-Path $PSScriptRoot 'win-capture.ps1'
$selected = if ($Only.Count -gt 0) { $Only } else { $Sets }
foreach ($set in $selected) {
    if ($Sets -notcontains $set) { throw "not a sample set: $set" }
}

# ---------------------------------------------------------------------------
# This machine's own state that a sample launch must never change.
# ---------------------------------------------------------------------------
function Get-MachineState {
    $lines = New-Object System.Collections.Generic.List[string]
    $scheme = 'HKCU:\Software\Classes\tracecommons'
    if (Test-Path $scheme) {
        foreach ($key in @(Get-Item $scheme) + @(Get-ChildItem $scheme -Recurse)) {
            foreach ($value in $key.GetValueNames()) {
                $lines.Add("scheme $($key.Name) [$value] = $($key.GetValue($value))")
            }
        }
    } else {
        $lines.Add('scheme absent')
    }
    $run = Get-Item 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -ErrorAction SilentlyContinue
    if ($run) {
        foreach ($value in $run.GetValueNames()) {
            $lines.Add("run [$value] = $($run.GetValue($value))")
        }
    }
    $stateDir = Join-Path $env:LOCALAPPDATA 'trace-commons'
    if (Test-Path $stateDir) {
        foreach ($item in Get-ChildItem $stateDir -Recurse -Force) {
            $lines.Add("state $($item.FullName) $($item.Length) $($item.LastWriteTimeUtc.ToString('o'))")
        }
    } else {
        $lines.Add('state absent')
    }
    # The sample launches' throwaway folders are not compared here: a
    # force-stopped launch leaves its own, and the next launch removes it
    # (SampleDaemon.SweepThrowawayDirs). The end of the run counts them.
    return , $lines.ToArray()
}

$stateLog = Join-Path $OutDir 'state-check.txt'
$failures = New-Object System.Collections.Generic.List[string]
function Compare-MachineState {
    param([string[]] $Before, [string] $After)
    $now = Get-MachineState
    $diff = @(Compare-Object -ReferenceObject $Before -DifferenceObject $now)
    if ($diff.Count -eq 0) {
        Add-Content -Path $stateLog -Value "unchanged after $After"
        return
    }
    $script:failures.Add($After)
    Add-Content -Path $stateLog -Value "CHANGED after ${After}:"
    foreach ($d in $diff) {
        $side = if ($d.SideIndicator -eq '=>') { '+' } else { '-' }
        Add-Content -Path $stateLog -Value "  $side $($d.InputObject)"
    }
}

$themeKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize'
$previousTheme = (Get-ItemProperty -Path $themeKey -Name AppsUseLightTheme -ErrorAction SilentlyContinue).AppsUseLightTheme

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$baseline = Get-MachineState
Set-Content -Path $stateLog -Value (@("boxUtc=$((Get-Date).ToUniversalTime().ToString('o')) exe=$Exe", 'baseline:') + ($baseline | ForEach-Object { "  $_" }))

$started = New-Object System.Collections.Generic.List[int]
try {
    foreach ($set in $selected) {
        foreach ($appearance in $Appearances) {
            $light = if ($appearance -eq 'light') { 1 } else { 0 }
            Set-ItemProperty -Path $themeKey -Name AppsUseLightTheme -Value $light -Type DWord

            $name = Get-CaptureName -Set $set -Tab $OpeningTab -Panes $OpeningPanes -Appearance $appearance
            $out = Join-Path $OutDir $name
            Write-Host "capturing $name"
            $shot = & $capture -Exe $Exe -Environment "TRACE_COMMONS_SAMPLE=$set" -Out $out `
                -SettleSeconds $SettleSeconds -Width $Width -Height $Height | Select-Object -Last 1
            $started.Add($shot.ProcessId)
            Stop-Process -Id $shot.ProcessId -Force -ErrorAction SilentlyContinue
            Wait-Process -Id $shot.ProcessId -Timeout 15 -ErrorAction SilentlyContinue
            Compare-MachineState -Before $baseline -After $name
        }
    }

    # Two sample launches at once must be two windows: a launch that took
    # part in single-instancing would hand itself to the first and exit.
    $checkDir = Join-Path $OutDir 'single-instance'
    New-Item -ItemType Directory -Force -Path $checkDir | Out-Null
    $set = $selected[0]
    $first = & $capture -Exe $Exe -Environment "TRACE_COMMONS_SAMPLE=$set" -Out (Join-Path $checkDir 'first.png') `
        -SettleSeconds $SettleSeconds | Select-Object -Last 1
    $started.Add($first.ProcessId)
    $second = $null
    try {
        $second = & $capture -Exe $Exe -Environment "TRACE_COMMONS_SAMPLE=$set" -Out (Join-Path $checkDir 'second.png') `
            -SettleSeconds $SettleSeconds | Select-Object -Last 1
        $started.Add($second.ProcessId)
    } catch {
        Add-Content -Path $stateLog -Value "single-instance: the second launch made no window of its own: $($_.Exception.Message)"
    }
    $ids = @($first.ProcessId)
    if ($second) { $ids += $second.ProcessId }
    $alive = @($ids | Where-Object { Get-Process -Id $_ -ErrorAction SilentlyContinue })
    if ($alive.Count -eq 2) {
        Add-Content -Path $stateLog -Value "single-instance: two sample launches, two windows (pids $($alive -join ', '))"
    } else {
        $failures.Add('single-instance')
        Add-Content -Path $stateLog -Value "SINGLE-INSTANCE: two sample launches left $($alive.Count) running"
    }
    foreach ($id in $alive) {
        Stop-Process -Id $id -Force -ErrorAction SilentlyContinue
        Wait-Process -Id $id -Timeout 15 -ErrorAction SilentlyContinue
    }
    Compare-MachineState -Before $baseline -After 'the single-instance launches'
}
finally {
    foreach ($id in $started) {
        Stop-Process -Id $id -Force -ErrorAction SilentlyContinue
    }
    if ($null -ne $previousTheme) {
        Set-ItemProperty -Path $themeKey -Name AppsUseLightTheme -Value $previousTheme -Type DWord
    } else {
        Remove-ItemProperty -Path $themeKey -Name AppsUseLightTheme -ErrorAction SilentlyContinue
    }
}

# Every launch above was force-stopped, so its exit never removed its
# throwaway folder; each launch removes those of the launches before it.
# Only the last two (the single-instance pair) may remain.
$leftover = @(Get-ChildItem $env:TEMP -Directory -Filter 'tracecommons-sample-*' -ErrorAction SilentlyContinue |
    Where-Object { $_.Name -match '^tracecommons-sample-(\d+)$' -and -not (Get-Process -Id ([int] $matches[1]) -ErrorAction SilentlyContinue) })
Add-Content -Path $stateLog -Value "sample-temp: $($leftover.Count) folder(s) of stopped launches left"
if ($leftover.Count -gt 2) {
    $failures.Add('sample-temp')
}

Write-Host "captures in $OutDir"
Get-Content $stateLog | Where-Object { $_ -notmatch '^  ' -or $_ -match '^  [+-] ' } | Write-Host
if ($failures.Count -gt 0) {
    Write-Host "STATE CHECK FAILED: $($failures -join ', ')"
    exit 1
}
Write-Host 'state check: nothing changed'
