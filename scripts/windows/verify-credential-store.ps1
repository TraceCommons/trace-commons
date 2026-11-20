# Verify that the built Windows app can reach the Windows Credential Manager.
#
# The Windows counterpart of the launch check in
# scripts/ci/verify-macos-entitlements.sh (spec W25-D2 (a)). It launches the
# app with TRACE_COMMONS_CREDENTIAL_STORE_CHECK_OUT naming a report file. The
# app answers before it opens a window, starts the daemon or takes its
# single-instance key (CredentialStoreCheck in
# windows/src/TraceCommons.Interop/CredentialStoreCheck.cs, called first in
# Program.Main), through tc_credential_store_self_check, the FFI call the Mac
# app makes. It writes one line, `reachable` or `unreachable code=N`, and
# exits.
#
# The self-check reads a credential reference that was never stored and
# writes nothing, so there is nothing to clean up. This script still compares
# the Credential Manager's list of target names before and after and fails if
# it changed; it prints only the counts, never a target name or a secret.
#
# No console session is needed: the app is started hidden and never reaches
# its window, and the Credential Manager is per user, not per desktop.
#
# Failure modes are kept distinguishable on purpose:
#   - still running after the timeout -> the app did not answer the check
#                                        at all (it went on to start).
#   - exited without a report         -> it died before answering, e.g. a
#                                        missing runtime file or the FFI DLL.
#   - `unreachable code=N`            -> it answered, and the store refused.

param(
    [Parameter(Mandatory = $true)] [string] $App,
    [int] $TimeoutSeconds = 30
)

$ErrorActionPreference = 'Stop'

if (-not (Test-Path $App -PathType Leaf)) {
    Write-Host "FAIL: $App is missing"
    exit 1
}
$App = (Resolve-Path $App).Path

function Get-CredentialTargets {
    # cmdkey prints one "Target: ..." line per stored credential.
    @(cmdkey /list | Where-Object { $_ -match '^\s*Target:' } | Sort-Object)
}

$before = Get-CredentialTargets

$out = Join-Path ([System.IO.Path]::GetTempPath()) ("tc-credential-store-check-" + [guid]::NewGuid().ToString('N') + ".txt")
$env:TRACE_COMMONS_CREDENTIAL_STORE_CHECK_OUT = $out
try {
    $process = Start-Process -FilePath $App -WindowStyle Hidden -PassThru
} finally {
    Remove-Item Env:\TRACE_COMMONS_CREDENTIAL_STORE_CHECK_OUT
}

if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
    Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
    Remove-Item $out -ErrorAction SilentlyContinue
    Write-Host "FAIL: the app was still running after ${TimeoutSeconds}s and did not answer the check"
    exit 1
}
$exitCode = $process.ExitCode

if (-not (Test-Path $out -PathType Leaf)) {
    Write-Host "FAIL: the app exited (code $exitCode) without writing a report"
    exit 1
}
$report = (Get-Content -Path $out -Raw).Trim()
Remove-Item $out -ErrorAction SilentlyContinue
Write-Host "report: $report (app exit code $exitCode)"

$after = Get-CredentialTargets
if (($before -join "`n") -ne ($after -join "`n")) {
    Write-Host "FAIL: the Credential Manager changed during the check ($($before.Count) entries before, $($after.Count) after)"
    exit 1
}
Write-Host "Credential Manager unchanged ($($after.Count) entries)"

if ($exitCode -ne 0) {
    Write-Host "FAIL: the app exited with code $exitCode"
    exit 1
}
if (-not $report.StartsWith('reachable')) {
    Write-Host "FAIL: $report"
    exit 1
}
Write-Host "PASS: the app reached the Windows Credential Manager"
