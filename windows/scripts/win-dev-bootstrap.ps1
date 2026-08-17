# Startup script for the Windows dev box created by provision-dev-vm.sh.
#
# Installs everything needed to build BOTH halves of the Windows app: the Rust
# cdylib and the WinUI 3 shell.
#
# GCE runs this as SYSTEM on first boot. It is not interactive, so every
# installer here runs unattended and every step logs to the serial console --
# that console is the only way to watch progress before RDP is usable.
#
# It is also idempotent. GCE re-runs the startup script on every boot, and a
# script that reinstalls Visual Studio Build Tools on each restart would make
# the box unusable for minutes at a time.

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'  # Invoke-WebRequest is far slower with a progress bar.

function Write-Log($message) {
    $stamp = (Get-Date).ToString('s')
    Write-Host "[tc-bootstrap $stamp] $message"
}

$marker = 'C:\tc-bootstrap-complete.txt'
if (Test-Path $marker) {
    Write-Log 'Bootstrap already completed on a previous boot; nothing to do.'
    exit 0
}

$downloads = 'C:\bootstrap-downloads'
New-Item -ItemType Directory -Force -Path $downloads | Out-Null

# TLS 1.2 for the download hosts. Windows Server 2022 defaults are usually
# fine, but pinning it avoids a confusing handshake failure on a fresh image.
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

function Get-Installer($url, $file) {
    $path = Join-Path $downloads $file
    if (Test-Path $path) {
        Write-Log "$file already downloaded"
        return $path
    }
    Write-Log "Downloading $file"
    Invoke-WebRequest -Uri $url -OutFile $path -UseBasicParsing
    return $path
}

# ---------------------------------------------------------------------------
# Visual Studio Build Tools, with the C++ workload.
#
# Required by RUST, not by C#: the MSVC toolchain links through link.exe, so
# the cdylib cannot be built without it. This is the slow step -- expect it to
# dominate the bootstrap time.
# ---------------------------------------------------------------------------
Write-Log 'Installing Visual Studio Build Tools (C++ workload). This is the slow one.'
$vs = Get-Installer 'https://aka.ms/vs/17/release/vs_BuildTools.exe' 'vs_BuildTools.exe'
# VCTools alone is NOT enough, and the failure is confusing when it is missing.
#
# VCTools covers Rust, which links through MSVC's link.exe. But a WinUI 3
# project also needs the Appx/MSIX MSBuild tasks -- Microsoft.Build.AppxPackage
# and Microsoft.Build.Packaging.Pri.Tasks -- which ship ONLY with Visual
# Studio's UWP tooling, never with the .NET SDK. Without them the build fails
# with MSB4062 naming a path under the dotnet SDK directory, which reads like a
# .NET problem and is not one. Disabling the offending target just surfaces the
# next task from the same missing assembly set.
$vsArgs = @(
    '--quiet', '--wait', '--norestart', '--nocache',
    '--add', 'Microsoft.VisualStudio.Workload.VCTools',
    '--add', 'Microsoft.VisualStudio.Workload.UniversalBuildTools',
    '--add', 'Microsoft.VisualStudio.Workload.ManagedDesktopBuildTools',
    '--add', 'Microsoft.VisualStudio.Component.Windows11SDK.22621',
    '--includeRecommended'
)
$proc = Start-Process -FilePath $vs -ArgumentList $vsArgs -Wait -PassThru
# 3010 is "success, reboot required" and is not a failure.
if ($proc.ExitCode -ne 0 -and $proc.ExitCode -ne 3010) {
    throw "Visual Studio Build Tools failed with exit code $($proc.ExitCode)"
}
Write-Log "Build Tools installed (exit $($proc.ExitCode))"

# ---------------------------------------------------------------------------
# .NET 8 SDK.
#
# The WinUI target framework's reference packs arrive through NuGet, so the SDK
# alone is enough to BUILD the app. Running it works because the app project
# sets WindowsAppSDKSelfContained, which carries the runtime with it rather
# than requiring a machine-wide install.
# ---------------------------------------------------------------------------
Write-Log 'Installing the .NET 8 SDK'
$dotnetScript = Get-Installer 'https://dot.net/v1/dotnet-install.ps1' 'dotnet-install.ps1'
& $dotnetScript -Channel '8.0' -InstallDir 'C:\Program Files\dotnet'
Write-Log '.NET SDK installed'

# ---------------------------------------------------------------------------
# Rust, MSVC toolchain.
# ---------------------------------------------------------------------------
#
# CARGO_HOME and RUSTUP_HOME are set machine-wide FIRST, and that is the whole
# point of this block.
#
# GCE runs startup scripts as SYSTEM, so a bare rustup-init installs into
# C:\Windows\System32\config\systemprofile\.cargo -- a directory no interactive
# login can reach. The box then looks correctly provisioned and the first thing
# anyone tries reports "cargo: command not found". Installing to a shared
# location instead makes the toolchain available to whichever account actually
# logs in.
Write-Log 'Installing Rust (stable-msvc) to a machine-wide location'
$cargoHome = 'C:\Rust\cargo'
$rustupHome = 'C:\Rust\rustup'
[Environment]::SetEnvironmentVariable('CARGO_HOME', $cargoHome, 'Machine')
[Environment]::SetEnvironmentVariable('RUSTUP_HOME', $rustupHome, 'Machine')
$env:CARGO_HOME = $cargoHome
$env:RUSTUP_HOME = $rustupHome

$rustup = Get-Installer 'https://win.rustup.rs/x86_64' 'rustup-init.exe'
& $rustup -y --default-toolchain stable-x86_64-pc-windows-msvc --profile minimal | Out-Null

# Everyone needs to read and execute it; only administrators may change it.
$rustAcl = Get-Acl 'C:\Rust'
$rustAcl.AddAccessRule((New-Object System.Security.AccessControl.FileSystemAccessRule(
    'BUILTIN\Users', 'ReadAndExecute', 'ContainerInherit,ObjectInherit', 'None', 'Allow')))
Set-Acl -Path 'C:\Rust' -AclObject $rustAcl
Write-Log "Rust installed to $cargoHome"

# ---------------------------------------------------------------------------
# Git.
# ---------------------------------------------------------------------------
Write-Log 'Installing Git'
$git = Get-Installer 'https://github.com/git-for-windows/git/releases/download/v2.47.1.windows.1/Git-2.47.1-64-bit.exe' 'git-install.exe'
Start-Process -FilePath $git -ArgumentList '/VERYSILENT', '/NORESTART', '/NOCANCEL', '/SP-' -Wait
Write-Log 'Git installed'

# ---------------------------------------------------------------------------
# PATH for every future interactive session.
#
# Set at machine scope so an RDP login inherits it. The current SYSTEM session
# does not see it, which is fine -- nothing further here needs these tools.
# ---------------------------------------------------------------------------
Write-Log 'Updating machine PATH'
$machinePath = [Environment]::GetEnvironmentVariable('Path', 'Machine')
foreach ($entry in @('C:\Program Files\dotnet', 'C:\Rust\cargo\bin', 'C:\Program Files\Git\cmd')) {
    if ($machinePath -notlike "*$entry*") {
        $machinePath = "$machinePath;$entry"
    }
}
[Environment]::SetEnvironmentVariable('Path', $machinePath, 'Machine')

# Rust installs per-user, so an RDP user other than the installing account
# needs cargo on their own PATH. Recorded rather than guessed at login time.
Set-Content -Path 'C:\tc-dev-notes.txt' -Value @'
Trace Commons Windows dev box

Build the cdylib first, then the app:

  cargo build -p trace-commons-contributor-ffi --release
  dotnet test windows\tests\TraceCommons.Interop.Tests\TraceCommons.Interop.Tests.csproj
  dotnet build windows\src\TraceCommons.App\TraceCommons.App.csproj -p:Platform=x64

Rust lives at C:\Rust\cargo (CARGO_HOME/RUSTUP_HOME are set machine-wide), so
every account gets the same toolchain. A per-user install would land in the
SYSTEM profile, because startup scripts run as SYSTEM.

The WinUI app needs Visual Studio's Appx/MSIX MSBuild tasks, which the .NET SDK
does not ship. Build it with MSBuild from Build Tools, not `dotnet build`.

Stop this VM when you are done -- it bills by the hour while running.
'@

Set-Content -Path $marker -Value (Get-Date).ToString('s')
Write-Log 'Bootstrap complete.'
