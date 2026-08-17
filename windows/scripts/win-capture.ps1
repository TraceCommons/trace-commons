# Launch a GUI app in the interactive console session and screenshot it.
#
# WHY THIS EXISTS. A WinUI app cannot start in session 0, which is where an
# SSH session lands. Anything launched from there dies inside
# Microsoft.UI.Xaml.dll with a stowed exception (0xc000027b) before a single
# line of managed application code runs -- no window station, no desktop, no
# XAML. That failure looks exactly like an application bug and is not one.
#
# So this script does not run the app itself. It hands the work to a scheduled
# task registered against the logged-on interactive user, which Windows starts
# INSIDE that user's session, and then captures the screen from a second task
# running in the same session. Screen capture from session 0 would return a
# black frame even if the app were running.
#
# The console session on a GCE Windows VM has a virtual display, so this works
# with nobody connected over RDP. A DISCONNECTED RDP session generally does
# not -- it has no rendering surface, and captures come back black. That is
# the reason auto-logon at the console is set up rather than relying on someone
# leaving an RDP window open.

[CmdletBinding()]
param(
    # The executable to launch. Omit to capture whatever is already on screen.
    [string] $Exe,

    # Where the PNG lands. Read back over SSH as base64.
    #
    # NOT the drive root. The launch and capture tasks run with /IT, which
    # means they run as the interactive user WITHOUT elevation, and an
    # unelevated process cannot write to C:\. The task still reports exit code
    # 0, so the only symptom is a missing file -- which reads as "capture
    # failed" rather than "permission denied".
    [string] $Out = 'C:\tc-captures\tc-capture.png',

    # How long to let the app settle before capturing. A WinUI cold start does
    # real work -- runtime bootstrap, XAML load, first layout -- and capturing
    # too early photographs an empty window, which reads as a rendering bug.
    [int] $SettleSeconds = 8,

    # Process name to raise to the foreground before capturing. Defaults to
    # the launched executable's name when -Exe is given.
    [string] $ProcessName = '',

    # Kill the app after capturing. Left running otherwise, so a follow-up
    # capture can photograph a later state without paying the start cost again.
    [switch] $Stop
)

$ErrorActionPreference = 'Stop'

# schtasks writes to stderr for ordinary, expected conditions -- deleting a
# task that was never created being the obvious one. Under
# ErrorActionPreference = 'Stop' PowerShell promotes any native stderr write to
# a terminating error, so a first run would die on its own cleanup step. These
# helpers scope the relaxed preference to the native calls rather than
# loosening it for the whole script.
function Remove-TaskQuietly {
    param([string] $Name)
    $previous = $ErrorActionPreference
    $ErrorActionPreference = 'SilentlyContinue'
    & schtasks.exe /Delete /TN $Name /F *>$null
    $ErrorActionPreference = $previous
}

function Invoke-Schtasks {
    param([string[]] $Arguments)
    $previous = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    $output = & schtasks.exe @Arguments 2>&1
    $code = $LASTEXITCODE
    $ErrorActionPreference = $previous
    if ($code -ne 0) {
        throw "schtasks $($Arguments -join ' ') failed ($code): $output"
    }
}

function Get-InteractiveUser {
    # The logged-on user of the ACTIVE session. `query session` marks it with
    # a leading '>' only for the current session, which is session 0 here, so
    # the state column is what to trust: Active means a user is present.
    $lines = query session 2>$null | Select-Object -Skip 1
    foreach ($line in $lines) {
        # SESSIONNAME USERNAME ID STATE TYPE DEVICE, space-padded.
        if ($line -match '^\s*>?(\S+)\s+(\S+)\s+(\d+)\s+(\w+)') {
            $user = $matches[2]
            $id = [int] $matches[3]
            $state = $matches[4]
            if ($state -eq 'Active' -and $user -ne '' -and $id -ne 0) {
                return [pscustomobject]@{ User = $user; Id = $id }
            }
        }
    }
    return $null
}

if ($ProcessName -eq '' -and $Exe) {
    $ProcessName = [IO.Path]::GetFileNameWithoutExtension($Exe)
}

$session = Get-InteractiveUser
if (-not $session) {
    throw 'No active interactive session. Auto-logon has not completed, or nobody is logged on.'
}

Write-Host "interactive session: $($session.User) (id $($session.Id))"

# ---------------------------------------------------------------------------
# Launch, via a scheduled task bound to the interactive user.
#
# /IT means "run only when the user is logged on", which is what places the
# process in their session rather than in session 0. Without it the task runs
# in session 0 and the app dies exactly as it does over SSH.
# ---------------------------------------------------------------------------
if ($Exe) {
    if (-not (Test-Path $Exe)) {
        throw "Executable not found: $Exe"
    }

    $taskName = 'tc-launch-app'
    Remove-TaskQuietly $taskName
    Invoke-Schtasks @('/Create', '/TN', $taskName, '/TR', "`"$Exe`"", '/SC', 'ONCE', '/ST', '00:00', '/RU', $session.User, '/IT', '/F')
    Invoke-Schtasks @('/Run', '/TN', $taskName)
    Write-Host "launched $Exe in session $($session.Id); settling ${SettleSeconds}s"
    Start-Sleep -Seconds $SettleSeconds
}

# ---------------------------------------------------------------------------
# Capture, also via a task in that session.
# ---------------------------------------------------------------------------
# The capture directory, writable by the unelevated task.
$outDir = Split-Path -Parent $Out
if (-not (Test-Path $outDir)) {
    New-Item -ItemType Directory -Force -Path $outDir | Out-Null
    $acl = Get-Acl $outDir
    $acl.AddAccessRule((New-Object System.Security.AccessControl.FileSystemAccessRule(
        'BUILTIN\Users', 'Modify', 'ContainerInherit,ObjectInherit', 'None', 'Allow')))
    Set-Acl -Path $outDir -AclObject $acl
}

# The inner script records its own failure. Without this a capture that threw
# still exits 0, and the only evidence is an absent file.
$captureScript = @'
$ErrorActionPreference = 'Stop'
try {
# Bring the target window to the front before photographing the screen.
# Without this the capture shows whatever happens to be on top -- usually the
# shell that launched the task -- and an app that is running perfectly well
# looks like an app that never opened.
if ('__PROC__' -ne '') {
    Add-Type -Namespace TcWin -Name Native -MemberDefinition @"
[DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
[DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
"@
    $target = Get-Process -Name '__PROC__' -ErrorAction SilentlyContinue |
        Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
    if ($target) {
        [TcWin.Native]::ShowWindow($target.MainWindowHandle, 3) | Out-Null   # SW_MAXIMIZE
        [TcWin.Native]::SetForegroundWindow($target.MainWindowHandle) | Out-Null
        Start-Sleep -Milliseconds 1200
    }
}
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms
$bounds = [System.Windows.Forms.SystemInformation]::VirtualScreen
$bmp = New-Object System.Drawing.Bitmap($bounds.Width, $bounds.Height)
$gfx = [System.Drawing.Graphics]::FromImage($bmp)
$gfx.CopyFromScreen($bounds.Location, [System.Drawing.Point]::Empty, $bounds.Size)
$bmp.Save('__OUT__', [System.Drawing.Imaging.ImageFormat]::Png)
$gfx.Dispose()
$bmp.Dispose()
} catch {
    Set-Content -Path '__OUT__.error.txt' -Value ($_ | Out-String)
}
'@ -replace '__OUT__', $Out -replace '__PROC__', $ProcessName

$captureFile = 'C:\tc-capture-inner.ps1'
Set-Content -Path $captureFile -Value $captureScript -Encoding ascii

if (Test-Path $Out) {
    Remove-Item $Out -Force
}

$capTask = 'tc-capture-screen'
Remove-TaskQuietly $capTask
Invoke-Schtasks @('/Create', '/TN', $capTask, '/TR',
    "powershell.exe -NoProfile -ExecutionPolicy Bypass -File $captureFile",
    '/SC', 'ONCE', '/ST', '00:00', '/RU', $session.User, '/IT', '/F')
Invoke-Schtasks @('/Run', '/TN', $capTask)

# Poll for the file rather than sleeping a fixed interval: the task is started
# asynchronously and a fixed sleep is either a guess that fails or wasted time.
for ($i = 0; $i -lt 30; $i++) {
    if (Test-Path $Out) {
        Start-Sleep -Milliseconds 500   # let the write complete
        break
    }
    Start-Sleep -Seconds 1
}

if (-not (Test-Path $Out)) {
    $errFile = "$Out.error.txt"
    if (Test-Path $errFile) {
        throw "Capture task failed: $(Get-Content $errFile -Raw)"
    }
    throw 'Capture task produced no file and recorded no error.'
}

Write-Host "captured: $Out ($((Get-Item $Out).Length) bytes)"

if ($Stop -and $Exe) {
    $name = [IO.Path]::GetFileNameWithoutExtension($Exe)
    Get-Process -Name $name -ErrorAction SilentlyContinue | Stop-Process -Force
    Write-Host "stopped $name"
}
