# Start every offered Windows terminal that is installed here, the way a
# managed session opens one, and check that the session folder and every
# helper argument arrive intact. The test it runs,
# managed::terminal::windows_tests::real_windows_terminals_receive_the_folder_and_every_argument,
# opens each terminal with a stub helper (compiled with rustc) that writes
# its working folder, its raw command line and its arguments to a file and
# exits. The folders hold spaces, a single quote, & ^ ( ) ! = , %NAME%,
# non-ASCII text, a leading dash, a backtick and $, and the drive root; the
# config folder holds %NAME% once. Every launch that is not refused must
# arrive whole, and the refusals must be exactly the expected ones.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File windows\scripts\managed-terminal-launch-smoke.ps1
#
# Run it inside the interactive desktop session, from the checkout to test.
# Over SSH, which lands in session 0 with no desktop, hand it to a task bound
# to the logged-on user (schtasks /Create ... /RU <user> /IT), as
# win-capture.ps1 does; a terminal started in session 0 never shows a tab.
# Run it twice: once with Windows Terminal closed, and once with a Windows
# Terminal window already open, since `wt new-tab` hands the tab to a running
# window. Each tab closes when the stub exits.
#
# It fails when no offered terminal is installed, so a pass always means at
# least one terminal was started. It prints each launch's command line.
param([string]$Log)
$ErrorActionPreference = 'Stop'
if ($env:OS -ne 'Windows_NT') { Write-Error 'managed-terminal-launch-smoke: Windows only'; exit 2 }
$session = (Get-Process -Id $PID).SessionId
if ($session -eq 0) { Write-Error 'managed-terminal-launch-smoke: run inside the interactive session, not session 0'; exit 2 }
Set-Location (git rev-parse --show-toplevel)
$cargoArgs = @('test', '-p', 'trace-commons-contributor', '--lib', '--',
  '--ignored', '--exact', '--nocapture', '--test-threads=1',
  'managed::terminal::windows_tests::real_windows_terminals_receive_the_folder_and_every_argument')
# Windows PowerShell 5.1 turns a native program's stderr into an error
# record, which 'Stop' would end the script on: cargo writes its progress
# there.
$ErrorActionPreference = 'Continue'
if ($Log) {
  & cargo @cargoArgs 2>&1 | ForEach-Object { "$_" } | Set-Content -Encoding UTF8 $Log
} else {
  & cargo @cargoArgs 2>&1 | ForEach-Object { "$_" }
}
exit $LASTEXITCODE
