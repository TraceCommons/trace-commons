#!/usr/bin/env bash
#
# Run a PowerShell command on the Azure Windows test box over SSH and print
# its output here. The Azure counterpart of win-exec.sh.
#
# The box's NSG admits tcp:22 from one address only (see
# provision-azure-vm.sh), its sshd accepts keys only, and its host key is
# pinned in a dedicated known_hosts file at create time -- fetched through the
# Azure control plane, not trusted on first use -- so StrictHostKeyChecking is
# on here, unlike win-exec.sh, whose IAP tunnel is its own authentication.
#
# The command travels as -EncodedCommand (UTF-16LE base64), so nothing in it
# needs escaping for the remote shell. Standard input is passed through, which
# is how a secret reaches the box without appearing in a command line or a
# script: pipe it in and read it with [Console]::In.ReadLine().
#
# Long work -- a toolchain install, a cold cargo build -- must not ride on one
# SSH connection: a process started from an SSH session dies with the session.
# --job runs the command as a detached scheduled task that writes a log and an
# exit-code file; --wait polls for them.
#
# Usage:
#   ./win-exec-az.sh 'cargo --version'
#   ./win-exec-az.sh --job ffi 'cd C:\src\trace-commons; cargo build -p trace-commons-contributor-ffi'
#   ./win-exec-az.sh --job bootstrap --system '& C:\tc\win-dev-bootstrap.ps1'
#   ./win-exec-az.sh --wait ffi 540     # wait up to 540 s on the box; exit = job's exit code
#
# --wait exits 0 or the job's own exit code once the job is done, 2 if there
# is no such job, and 75 if it is still running when the wait runs out.
#
set -euo pipefail

RG="${TC_AZ_RG:-tc-win-azure}"
NAME="${TC_AZ_VM:-tc-win-az}"
KEY="${TC_AZ_KEY:-$HOME/.ssh/tc-win-az}"
KNOWN_HOSTS="${TC_AZ_KNOWN_HOSTS:-$HOME/.ssh/${NAME}.known_hosts}"
USER_NAME="${TC_AZ_USER:-tcdev}"

usage() {
  sed -n '/^# Usage:/,/^# --wait exits/p' "$0" | sed 's/^# \{0,1\}//' >&2
  exit 1
}

[ $# -ge 1 ] || usage

# Resolved from the API on every run: no address is written down anywhere.
IP="${TC_AZ_IP:-$(az network public-ip show -g "$RG" -n "${NAME}-ip" --query ipAddress -o tsv --only-show-errors)}"
if [ -z "$IP" ]; then
  echo "could not resolve the public IP of $NAME in $RG" >&2
  exit 1
fi
if [ ! -f "$KNOWN_HOSTS" ]; then
  echo "no pinned host key at $KNOWN_HOSTS; run provision-azure-vm.sh create" >&2
  exit 1
fi

# The remote shell is PowerShell (set by win-enable-ssh.ps1).
run_ps() {
  # Errors are folded into stdout as plain text. Left on stderr, a PowerShell
  # started with -EncodedCommand serialises them as CLIXML.
  local script="\$ProgressPreference = 'SilentlyContinue'
& {
$1
} 2>&1 | ForEach-Object { \"\$_\" }
if (\$LASTEXITCODE) { exit \$LASTEXITCODE }"
  local enc
  enc="$(printf '%s' "$script" | iconv -f UTF-8 -t UTF-16LE | base64 | tr -d '\n')"
  ssh -i "$KEY" \
    -o IdentitiesOnly=yes \
    -o HostKeyAlias="$NAME" \
    -o UserKnownHostsFile="$KNOWN_HOSTS" \
    -o StrictHostKeyChecking=yes \
    -o ConnectTimeout=15 \
    -o ServerAliveInterval=30 \
    -o LogLevel=ERROR \
    "${USER_NAME}@${IP}" \
    "powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -OutputFormat Text -EncodedCommand $enc; exit \$LASTEXITCODE"
}

valid_job_name() {
  [[ "$1" =~ ^[A-Za-z0-9_-]+$ ]] || { echo "job name must be [A-Za-z0-9_-]+" >&2; exit 1; }
}

case "$1" in
  --job)
    shift
    [ $# -ge 2 ] || usage
    JOB="$1"; shift
    valid_job_name "$JOB"
    PRINCIPAL='New-ScheduledTaskPrincipal -UserId ([Security.Principal.WindowsIdentity]::GetCurrent().Name) -LogonType S4U -RunLevel Highest'
    if [ "$1" = "--system" ]; then
      PRINCIPAL='New-ScheduledTaskPrincipal -UserId SYSTEM -LogonType ServiceAccount -RunLevel Highest'
      shift
    fi
    [ $# -ge 1 ] || usage
    CMD_B64="$(printf '%s\nexit $LASTEXITCODE\n' "$*" | base64 | tr -d '\n')"
    # S4U runs as this account without storing its password; --system runs as
    # SYSTEM, which the toolchain bootstrap's machine-wide installs expect.
    run_ps "
\$ErrorActionPreference = 'Stop'
\$dir = 'C:\tc\jobs'
New-Item -ItemType Directory -Force \$dir | Out-Null
\$name = '$JOB'
if ((Get-ScheduledTask -TaskName \"tc-job-\$name\" -ErrorAction SilentlyContinue).State -eq 'Running') {
    throw \"job \$name is already running\"
}
Remove-Item -Force -ErrorAction SilentlyContinue \"\$dir\\\$name.log\", \"\$dir\\\$name.exitcode\"
[IO.File]::WriteAllText(\"\$dir\\\$name.cmd.ps1\", [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('$CMD_B64')))
\$runner = @'
\$ProgressPreference = 'SilentlyContinue'
\$log = 'C:\tc\jobs\__NAME__.log'
\$code = 1
# Shared for reading: Add-Content would hold the log exclusively for the whole
# run, and --wait could not read it until the job had finished.
\$fs = [IO.File]::Open(\$log, 'Append', 'Write', 'ReadWrite')
\$w = New-Object IO.StreamWriter(\$fs, (New-Object Text.UTF8Encoding(\$false)))
\$w.AutoFlush = \$true
try {
    \$w.WriteLine(\"[tc-job __NAME__ \$((Get-Date).ToString('s'))] started as \$env:USERNAME\")
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File 'C:\tc\jobs\__NAME__.cmd.ps1' *>&1 |
        ForEach-Object { \$w.WriteLine(\"\$_\") }
    \$code = \$LASTEXITCODE
} catch {
    \$w.WriteLine(\"JOB RUNNER FAILED: \$_\")
}
\$w.WriteLine(\"[tc-job __NAME__ \$((Get-Date).ToString('s'))] exit \$code\")
\$w.Close()
Set-Content -Path 'C:\tc\jobs\__NAME__.exitcode' -Value \$code
'@
Set-Content -Encoding ascii -Path \"\$dir\\\$name.run.ps1\" -Value (\$runner -replace '__NAME__', \$name)
\$action = New-ScheduledTaskAction -Execute powershell.exe -Argument \"-NoProfile -ExecutionPolicy Bypass -File \$dir\\\$name.run.ps1\"
\$principal = $PRINCIPAL
\$settings = New-ScheduledTaskSettingsSet -ExecutionTimeLimit (New-TimeSpan -Hours 4) -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries
Register-ScheduledTask -TaskName \"tc-job-\$name\" -Action \$action -Principal \$principal -Settings \$settings -Force -ErrorAction Stop | Out-Null
Start-ScheduledTask -TaskName \"tc-job-\$name\" -ErrorAction Stop
\"job \$name started; poll with: win-exec-az.sh --wait \$name\"
"
    ;;
  --wait)
    shift
    [ $# -ge 1 ] || usage
    JOB="$1"
    valid_job_name "$JOB"
    WAIT="${2:-540}"
    [[ "$WAIT" =~ ^[0-9]+$ ]] || usage
    # The wait happens on the box, so this side holds one short connection.
    run_ps "
\$name = '$JOB'
\$exit = \"C:\tc\jobs\\\$name.exitcode\"
\$log = \"C:\tc\jobs\\\$name.log\"
if (-not (Test-Path \$exit) -and -not (Get-ScheduledTask -TaskName \"tc-job-\$name\" -ErrorAction SilentlyContinue)) {
    \"== no job named \$name\"
    exit 2
}
\$deadline = (Get-Date).AddSeconds($WAIT)
while (-not (Test-Path \$exit) -and (Get-Date) -lt \$deadline) { Start-Sleep -Seconds 10 }
if (Test-Path \$log) {
    # The runner holds the log open for writing; read it with a sharing mode
    # that allows that, which Get-Content does not.
    \$fs = [IO.File]::Open(\$log, 'Open', 'Read', 'ReadWrite')
    \$sr = New-Object IO.StreamReader(\$fs)
    \$text = \$sr.ReadToEnd()
    \$sr.Close()
    \$text -split \"\`r?\`n\" | Select-Object -Last 40
}
if (Test-Path \$exit) {
    \$code = [int]((Get-Content \$exit -Raw).Trim())
    \"== job \$name finished, exit code \$code\"
    exit \$code
}
\"== job \$name still running (task state: \$((Get-ScheduledTask -TaskName \"tc-job-\$name\").State))\"
exit 75
"
    ;;
  -*)
    usage
    ;;
  *)
    run_ps "$*"
    ;;
esac
