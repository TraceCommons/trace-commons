# W25 keyboard and accessibility audit: the session-0 runner. Sets the app
# theme (and High Contrast through the driver), runs win-a11y-drive.ps1 in
# the console session through a hidden /IT task, reports whether the app's
# own state on this machine changed, and puts the theme back. The app must
# be a Debug build: only a Debug build reads TRACE_COMMONS_SAMPLE, so a
# Release build would start a real daemon.
#
# Usage (on the box):
#   powershell -ExecutionPolicy Bypass -File win-a11y-run.ps1 -Exe <Debug TraceCommons.exe> `
#       -Set normalDay -Appearance light -Tag <name> -Plan plans\main.txt
# Session-0 side: set the app theme, run drive.ps1 in the interactive session
# through a hidden /IT task, wait, report the app-state check, restore theme.
param([Parameter(Mandatory = $true)] [string] $Exe, [string] $Set = 'normalDay', [ValidateSet('light','dark','hc','hcwhite','hclight')] [string] $Appearance = 'light', [string] $Tag, [string] $Plan,
      [string] $FirstRunStep = '', [string] $Extra = '', [int] $Timeout = 900, [string] $OutDir = 'C:\tc-captures\a11y')
$ErrorActionPreference = 'Stop'
$exe = $Exe
if (Get-Process TraceCommons -EA SilentlyContinue) { throw 'a TraceCommons process is already running; refusing' }
if (-not (Test-Path $OutDir)) {
  New-Item -ItemType Directory -Force $OutDir | Out-Null
  $acl = Get-Acl $OutDir
  $acl.AddAccessRule((New-Object System.Security.AccessControl.FileSystemAccessRule('BUILTIN\Users', 'Modify', 'ContainerInherit,ObjectInherit', 'None', 'Allow')))
  Set-Acl -Path $OutDir -AclObject $acl
}
$user = $null
foreach ($line in (query session 2>$null | Select-Object -Skip 1)) { if ($line -match '^\s*>?(\S+)\s+(\S+)\s+(\d+)\s+Active' -and [int]$matches[3] -ne 0) { $user = $matches[2] } }
if (-not $user) { throw 'no active interactive session' }
function State {
  $l = New-Object System.Collections.Generic.List[string]
  foreach ($d in @("$env:LOCALAPPDATA\TraceCommons", "$env:LOCALAPPDATA\trace-commons", "$env:APPDATA\TraceCommons", "$env:APPDATA\trace-commons")) {
    if (Test-Path $d) { foreach ($i in Get-ChildItem $d -Recurse -Force) { $l.Add("$($i.FullName) $($i.Length) $($i.LastWriteTimeUtc.ToString('o'))") } } else { $l.Add("absent $d") }
  }
  $run = Get-Item 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -EA SilentlyContinue
  if ($run) { foreach ($v in $run.GetValueNames()) { $l.Add("run [$v] = $($run.GetValue($v))") } }
  $l.Add("scheme " + (Test-Path 'HKCU:\Software\Classes\tracecommons'))
  return ,$l.ToArray()
}
$stateBefore = State
$key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize'
$prev = (Get-ItemProperty $key -Name AppsUseLightTheme -EA SilentlyContinue).AppsUseLightTheme
$prevSys = (Get-ItemProperty $key -Name SystemUsesLightTheme -EA SilentlyContinue).SystemUsesLightTheme
$light = [int]($Appearance -eq 'light' -or $Appearance -eq 'hclight')
Set-ItemProperty $key -Name AppsUseLightTheme -Value $light -Type DWord
Set-ItemProperty $key -Name SystemUsesLightTheme -Value $light -Type DWord
try {
  $extraArgs = @()
  if ($Appearance -eq 'hc' -or $Appearance -eq 'hclight') { $extraArgs += '-HighContrast' }
  if ($Appearance -eq 'hcwhite') { $extraArgs += '-HighContrast'; $extraArgs += '-HcScheme white' }
  if ($FirstRunStep) { $extraArgs += "-FirstRunStep $FirstRunStep" }
  if ($Extra) { $extraArgs += "-Extra $Extra" }
  $cmd = "powershell.exe -NoProfile -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -File `"$(Join-Path $PSScriptRoot 'win-a11y-drive.ps1')`" -Exe `"$exe`" -Set $Set -OutDir $OutDir -Tag $Tag -Plan $Plan $($extraArgs -join ' ')"
  $vbs = Join-Path $OutDir 'a11y-drive.vbs'
  Set-Content -Path $vbs -Encoding ascii -Value @('Set sh = CreateObject("WScript.Shell")', ('sh.Run "' + $cmd.Replace('"', '""') + '", 0, True'))
  $logf = Join-Path $OutDir "$Tag.drive.log"
  Remove-Item -Force -EA SilentlyContinue $logf
  cmd /c "schtasks /Delete /TN tc-a11y-drive /F >nul 2>&1"
  $ErrorActionPreference = 'Continue'
  & schtasks.exe /Create /TN tc-a11y-drive /TR "wscript.exe //B //Nologo `"$vbs`"" /SC ONCE /ST 00:00 /RU $user /IT /F 2>&1 | Out-Null
  & schtasks.exe /Run /TN tc-a11y-drive 2>&1 | Out-Null
  $ErrorActionPreference = 'Stop'
  $deadline = (Get-Date).AddSeconds($Timeout)
  while ((Get-Date) -lt $deadline) {
    Start-Sleep -Seconds 3
    if ((Test-Path $logf) -and (Select-String -Path $logf -Pattern '\] done$' -Quiet -ErrorAction SilentlyContinue)) { break }
  }
  if (Test-Path $logf) { Get-Content $logf } else { 'no log' }
  $d = @(Compare-Object $stateBefore (State))
  if ($d.Count) { 'STATE CHANGED:'; $d | % { "  $($_.SideIndicator) $($_.InputObject)" } } else { 'STATE UNCHANGED' }
} finally {
  cmd /c "schtasks /Delete /TN tc-a11y-drive /F >nul 2>&1"
  if ($null -eq $prevSys) { Remove-ItemProperty $key -Name SystemUsesLightTheme -EA SilentlyContinue } else { Set-ItemProperty $key -Name SystemUsesLightTheme -Value $prevSys -Type DWord }
  if ($null -eq $prev) { Remove-ItemProperty $key -Name AppsUseLightTheme -EA SilentlyContinue } else { Set-ItemProperty $key -Name AppsUseLightTheme -Value $prev -Type DWord }
  Get-Process TraceCommons -EA SilentlyContinue | ? { $_.Path -eq $exe } | Stop-Process -Force
}
