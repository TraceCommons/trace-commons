# Verify that the daemon's Windows named pipe refuses a second local user.
#
# On Windows the pipe's DACL is the only access control on the daemon IPC
# socket. This script is the evidence that it works: it creates a second,
# non-administrator local account, has that account attempt to open the pipe,
# and requires the attempt to be refused with ERROR_ACCESS_DENIED (5).
#
# The account is deliberately NOT an administrator. An administrator can take
# ownership of any object and is expected to be able to reach the pipe, so a
# test using one would pass regardless of whether the DACL were correct --
# which is worse than no test, because it would look like evidence.
#
# Failure modes are kept distinguishable on purpose:
#   - CONNECTED from the second user  -> the DACL is wrong. Hard failure.
#   - DENIED with a code other than 5 -> refused for some unrelated reason
#                                        (e.g. the pipe was not up yet).
#                                        Also a failure: it is not evidence
#                                        the DACL did the refusing.
#   - HARNESS-FAILURE / exit 3        -> the logon or impersonation itself
#                                        failed. Not a verdict either way.
# Only "DENIED 5" from the second user plus "CONNECTED" from the owner counts
# as a pass.

$ErrorActionPreference = 'Stop'

$probe = Join-Path $PWD 'target\debug\win-pipe-acl-probe.exe'
if (-not (Test-Path $probe)) {
    Write-Error "probe not built at $probe"
    exit 1
}

$stateDir = Join-Path $env:RUNNER_TEMP 'tc-acl-state'
New-Item -ItemType Directory -Force -Path $stateDir | Out-Null

# A random password per run: this account exists for the length of one job,
# and a constant here would be a credential checked into a public repository
# even though it only ever unlocks an ephemeral runner account.
#
# Built from RandomNumberGenerator rather than
# System.Web.Security.Membership::GeneratePassword -- System.Web does not
# exist in .NET Core, so that call throws under pwsh (PowerShell 7), which is
# the shell this job runs in.
$bytes = [byte[]]::new(32)
[System.Security.Cryptography.RandomNumberGenerator]::Fill($bytes)
# Strip to alphanumerics, then append one of each required class so the
# result always satisfies the local password policy regardless of what the
# random bytes happened to produce.
$password = ([Convert]::ToBase64String($bytes) -replace '[^a-zA-Z0-9]', '') + 'aA1!'
$secure = ConvertTo-SecureString $password -AsPlainText -Force
$userName = 'tcaclprobe'

Write-Host '--- creating a second, non-administrator local account'
if (Get-LocalUser -Name $userName -ErrorAction SilentlyContinue) {
    Remove-LocalUser -Name $userName
}
New-LocalUser -Name $userName -Password $secure -AccountNeverExpires:$true `
    -UserMayNotChangePassword:$true | Out-Null
# Deliberately added to Users and nothing else. Never Administrators.
Add-LocalGroupMember -Group 'Users' -Member $userName -ErrorAction SilentlyContinue

$serveProc = $null
try {
    Write-Host '--- starting the pipe server as the current (owning) user'
    $outFile = Join-Path $env:RUNNER_TEMP 'serve-out.txt'
    $errFile = Join-Path $env:RUNNER_TEMP 'serve-err.txt'
    $serveProc = Start-Process -FilePath $probe `
        -ArgumentList 'serve', $stateDir `
        -PassThru -NoNewWindow `
        -RedirectStandardOutput $outFile -RedirectStandardError $errFile

    # Wait for READY rather than sleeping a fixed interval: a fixed sleep that
    # is too short turns into a spurious "DENIED 2" that looks like a result.
    $ready = $false
    foreach ($i in 1..60) {
        Start-Sleep -Milliseconds 500
        if ((Test-Path $outFile) -and (Get-Content $outFile -Raw) -match 'READY') {
            $ready = $true
            break
        }
        if ($serveProc.HasExited) { break }
    }
    if (-not $ready) {
        Write-Host '--- server stderr:'
        if (Test-Path $errFile) { Get-Content $errFile }
        Write-Error 'the pipe server never reported READY'
        exit 1
    }
    Write-Host '--- server ready'
    if (Test-Path $errFile) { Get-Content $errFile }

    # ORDER MATTERS. `serve` creates one pipe instance and a successful
    # connect consumes it, so the denial test runs FIRST, while the instance
    # is still free. Run the owner control first and the second user would be
    # refused with ERROR_PIPE_BUSY (231) rather than ERROR_ACCESS_DENIED --
    # still a refusal, but not one that says anything about the DACL.
    Write-Host ''
    Write-Host '--- THE TEST: a second, unprivileged user must be refused'
    $otherResult = & $probe connect-as $userName $password $stateDir
    $probeExit = $LASTEXITCODE
    Write-Host "second user: $otherResult (exit $probeExit)"

    if ($probeExit -eq 3) {
        Write-Error 'the logon/impersonation harness failed; this is not evidence either way'
        exit 1
    }
    if ($otherResult -match '^CONNECTED') {
        Write-Error 'SECURITY FAILURE: a second local user opened the daemon pipe. The DACL does not restrict it.'
        exit 1
    }
    if ($otherResult -notmatch '^DENIED 5$') {
        Write-Error "the second user was refused, but not with ERROR_ACCESS_DENIED ($otherResult). That is not evidence the DACL did the refusing."
        exit 1
    }

    Write-Host ''
    Write-Host '--- CONTROL: the owning user must be admitted'
    $ownerResult = & $probe connect $stateDir
    Write-Host "owner: $ownerResult"
    if ($ownerResult -notmatch '^CONNECTED') {
        Write-Error "the owning user was refused its own pipe ($ownerResult). A DACL that excludes everyone is also a bug."
        exit 1
    }

    Write-Host ''
    Write-Host 'PASS: the owning user is admitted and a second local user is denied by the DACL.'
}
finally {
    if ($serveProc -and -not $serveProc.HasExited) {
        Stop-Process -Id $serveProc.Id -Force -ErrorAction SilentlyContinue
    }
    Remove-LocalUser -Name $userName -ErrorAction SilentlyContinue
}
