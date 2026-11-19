# Run the Win2D render tests in the test box's interactive session.
#
# Win2D cannot create a device in session 0 (spike S1), and an SSH command or
# an S4U/SYSTEM task runs there. This runs TraceCommons.RenderTests.exe through
# an /IT scheduled task as the logged-on user, with TC_RENDER_TESTS_REQUIRED=1
# so a session that cannot render fails instead of skipping, and prints the
# harness's own log. Run it on the box, from the repository's windows/
# directory, after building the project (see .github/workflows/clients.yml).

param(
    [string] $Exe = 'tests\TraceCommons.RenderTests\bin\x64\Release\net8.0-windows10.0.19041.0\win-x64\TraceCommons.RenderTests.exe',
    [string] $OutDir = 'C:\tc-render-out',
    [int] $TimeoutSeconds = 180
)

$ErrorActionPreference = 'Stop'

function Invoke-Schtasks {
    param([string[]] $Arguments)
    $previous = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    $output = & schtasks.exe @Arguments 2>&1
    $code = $LASTEXITCODE
    $ErrorActionPreference = $previous
    if ($code -ne 0) { throw "schtasks $($Arguments -join ' ') failed ($code): $output" }
}

function Remove-TaskQuietly {
    param([string] $Name)
    $previous = $ErrorActionPreference
    $ErrorActionPreference = 'SilentlyContinue'
    & schtasks.exe /Delete /TN $Name /F *>$null
    $ErrorActionPreference = $previous
}

$user = $null
foreach ($line in (query session 2>$null | Select-Object -Skip 1)) {
    if ($line -match '^\s*>?(\S+)\s+(\S+)\s+(\d+)\s+Active' -and [int] $matches[3] -ne 0) { $user = $matches[2] }
}
if (-not $user) { throw 'No active interactive session: auto-logon has not completed, or nobody is logged on.' }

$Exe = (Resolve-Path $Exe).Path
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$acl = Get-Acl $OutDir
$acl.AddAccessRule((New-Object System.Security.AccessControl.FileSystemAccessRule(
    'BUILTIN\Users', 'Modify', 'ContainerInherit,ObjectInherit', 'None', 'Allow')))
Set-Acl -Path $OutDir -AclObject $acl

$log = Join-Path $OutDir 'render-tests.log'
$code = Join-Path $OutDir 'render-tests.exit'
Remove-Item -Force -ErrorAction SilentlyContinue $log, $code
$cmd = Join-Path $OutDir 'render-tests.cmd'
Set-Content -Path $cmd -Encoding ascii -Value @(
    '@echo off',
    'set TC_RENDER_TESTS_REQUIRED=1',
    "`"$Exe`" `"$OutDir`" > `"$log`" 2>&1",
    "echo %ERRORLEVEL% > `"$code`""
)
$vbs = Join-Path $OutDir 'render-tests.vbs'
Set-Content -Path $vbs -Encoding ascii -Value @(
    'Set sh = CreateObject("WScript.Shell")',
    "sh.Run ""cmd.exe /c """"$cmd"""""", 0, True"
)

Remove-TaskQuietly 'tc-render-tests'
Invoke-Schtasks @('/Create', '/TN', 'tc-render-tests', '/TR', "wscript.exe //B //Nologo `"$vbs`"",
    '/SC', 'ONCE', '/ST', '00:00', '/RU', $user, '/IT', '/F')
Invoke-Schtasks @('/Run', '/TN', 'tc-render-tests')
for ($i = 0; $i -lt $TimeoutSeconds -and -not (Test-Path $code); $i++) { Start-Sleep -Seconds 1 }
Remove-TaskQuietly 'tc-render-tests'
if (-not (Test-Path $code)) { throw "the render tests did not finish within ${TimeoutSeconds}s" }

Get-Content $log
$exit = [int] (Get-Content $code -Raw).Trim()
Write-Host "render tests exit code: $exit (box utc $([DateTime]::UtcNow.ToString('o')))"
exit $exit
