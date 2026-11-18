# Capture the debug app on each recorded sample set, named for the parity
# gallery. Runs ON the Windows test box (decision W1-D1: captures are made
# there and never uploaded from CI).
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
# task in the interactive session, which does not inherit this shell's
# environment. So each launch goes through a small .cmd written here that sets
# TRACE_COMMONS_SAMPLE and the app theme (HKCU AppsUseLightTheme, which the
# app follows at start) and then starts the exe. The theme value in force
# before the run is put back at the end.
#
# WHAT IT CAN CAPTURE TODAY. The window opens on the contribution queue in
# sample mode, which is the Mac's Traces tab with no map or inspector, so each
# launch yields one capture, <set>-traces-main-<appearance>.png. The window has
# no Home tab, map or inspector to steer to until slice W2; the other tabs and
# panes are in the lists for the names alone. win-capture.ps1 maximises the
# window and grabs the whole screen, so captures are screen-sized.
#
# The app must be a Debug build: a Release build has no sample client and
# would start a real daemon. Never sign in, enroll or share on the box.
#
# Usage (on the box):
#   powershell -ExecutionPolicy Bypass -File win-capture-samples.ps1 `
#       -Exe C:\tc\windows\src\TraceCommons.App\bin\x64\Debug\...\TraceCommons.exe
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

    [int] $SettleSeconds = 10
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

if (-not (Test-Path $Exe)) {
    throw "Executable not found: $Exe"
}
$Exe = (Resolve-Path $Exe).Path
$processName = [IO.Path]::GetFileNameWithoutExtension($Exe)
$capture = Join-Path $PSScriptRoot 'win-capture.ps1'

$selected = if ($Only.Count -gt 0) { $Only } else { $Sets }
foreach ($set in $selected) {
    if ($Sets -notcontains $set) { throw "not a sample set: $set" }
}

$themeKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize'
$previousTheme = (Get-ItemProperty -Path $themeKey -Name AppsUseLightTheme -ErrorAction SilentlyContinue).AppsUseLightTheme

New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$launcher = Join-Path $OutDir 'tc-sample-launch.cmd'

try {
    foreach ($set in $selected) {
        foreach ($appearance in $Appearances) {
            $light = if ($appearance -eq 'light') { 1 } else { 0 }
            Set-Content -Path $launcher -Encoding ascii -Value @(
                '@echo off',
                "reg add HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize /v AppsUseLightTheme /t REG_DWORD /d $light /f >nul",
                "set `"TRACE_COMMONS_SAMPLE=$set`"",
                "start `"`" `"$Exe`""
            )

            $name = Get-CaptureName -Set $set -Tab $OpeningTab -Panes $OpeningPanes -Appearance $appearance
            $out = Join-Path $OutDir $name
            Write-Host "capturing $name"
            & $capture -Exe $launcher -ProcessName $processName -Out $out -SettleSeconds $SettleSeconds

            # win-capture.ps1 -Stop would stop a process named after the
            # launcher, not the app, so the app is stopped here.
            Get-Process -Name $processName -ErrorAction SilentlyContinue | Stop-Process -Force
            Start-Sleep -Seconds 2
        }
    }
}
finally {
    Remove-Item -Path $launcher -Force -ErrorAction SilentlyContinue
    if ($null -ne $previousTheme) {
        Set-ItemProperty -Path $themeKey -Name AppsUseLightTheme -Value $previousTheme
    }
}

Write-Host "captures in $OutDir"
