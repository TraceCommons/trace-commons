# CPU and drawing profile: the session-0 runner. Runs win-perf-drive.ps1 in
# the console session through a hidden /IT task, waits, prints its log and
# measurements, and reports whether the app's own state on this machine
# changed. The app must be a Debug build: only a Debug build reads
# TRACE_COMMONS_SAMPLE, so a Release build would start a real daemon. See
# windows/docs/perf-profile.md.
#
# Usage (on the box):
#   powershell -ExecutionPolicy Bypass -File win-perf-run.ps1 -Exe <Debug TraceCommons.exe> `
#       -Tag <name> -Plan plans\map.txt [-Throttle full|2core|2core-half|1core-half] [-OutDir C:\tc-captures\perf]
param([Parameter(Mandatory = $true)] [string] $Exe, [Parameter(Mandatory = $true)] [string] $Tag,
      [Parameter(Mandatory = $true)] [string] $Plan, [string] $Throttle = 'full', [int] $Timeout = 1800, [string] $OutDir = 'C:\tc-captures\perf')
$ErrorActionPreference = 'Stop'
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
$drive = Join-Path $PSScriptRoot 'win-perf-drive.ps1'
$cmd = "powershell.exe -NoProfile -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -File `"$drive`" -Exe `"$Exe`" -OutDir `"$OutDir`" -Tag $Tag -Plan `"$Plan`" -Throttle $Throttle"
$vbs = Join-Path $OutDir 'perf-drive.vbs'
Set-Content -Path $vbs -Encoding ascii -Value @('Set sh = CreateObject("WScript.Shell")', ('sh.Run "' + $cmd.Replace('"', '""') + '", 0, True'))
$logf = Join-Path $OutDir "$Tag.drive.log"
Remove-Item -Force -EA SilentlyContinue $logf, (Join-Path $OutDir "$Tag.perf.tsv")
try {
  cmd /c "schtasks /Delete /TN tc-perf-drive /F >nul 2>&1"
  $ErrorActionPreference = 'Continue'
  & schtasks.exe /Create /TN tc-perf-drive /TR "wscript.exe //B //Nologo `"$vbs`"" /SC ONCE /ST 00:00 /RU $user /IT /F 2>&1 | Out-Null
  & schtasks.exe /Run /TN tc-perf-drive 2>&1 | Out-Null
  $ErrorActionPreference = 'Stop'
  $deadline = (Get-Date).AddSeconds($Timeout)
  # wpr needs an elevated token, which the console-session task does not
  # have: the driver asks for a trace by writing a request file, and this
  # side starts or stops it and answers.
  $req = Join-Path $OutDir "$Tag.wpr-request"
  $ack = Join-Path $OutDir "$Tag.wpr-ack"
  Remove-Item -Force -EA SilentlyContinue $req, $ack
  $tracing = $false
  while ((Get-Date) -lt $deadline) {
    Start-Sleep -Milliseconds 500
    if (Test-Path $req) {
      $r = (Get-Content $req -Raw).Trim() -split '\s+'
      Remove-Item -Force $req
      $ErrorActionPreference = 'Continue'
      if ($r[0] -eq 'start') {
        $profiles = if ($r.Count -gt 1) { $r[1..($r.Count - 1)] } else { @("CPU") }
        $wargs = @(); foreach ($p in $profiles) { $wargs += "-start"; $wargs += $p }
        $out = & wpr.exe @wargs 2>&1 | Out-String
        $tracing = $true
      } else {
        $etl = Join-Path $OutDir "$Tag-$($r[1]).etl"
        $out = & wpr.exe -stop $etl 2>&1 | Out-String
        $tracing = $false
      }
      $ErrorActionPreference = 'Stop'
      Set-Content -Path $ack -Value ("wpr {0}: exit {1} {2}" -f $r[0], $LASTEXITCODE, ($out -replace '\s+', ' ').Trim())
    }
    if ((Test-Path $logf) -and (Select-String -Path $logf -Pattern '\] done$' -Quiet -ErrorAction SilentlyContinue)) { break }
  }
  if ($tracing) { cmd /c "wpr -cancel >nul 2>&1"; 'wpr: cancelled a trace the plan left running' }
  if (Test-Path $logf) { Get-Content $logf } else { 'no log' }
  $d = @(Compare-Object $stateBefore (State))
  if ($d.Count) { 'STATE CHANGED:'; $d | % { "  $($_.SideIndicator) $($_.InputObject)" } } else { 'STATE UNCHANGED' }
} finally {
  cmd /c "schtasks /Delete /TN tc-perf-drive /F >nul 2>&1"
  Remove-Item -Force -EA SilentlyContinue $vbs
  Get-Process TraceCommons -EA SilentlyContinue | ? { $_.Path -eq $Exe } | Stop-Process -Force
}
