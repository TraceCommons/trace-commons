<#
.SYNOPSIS
Builds the MSIX package for the Windows contributor app. Does not sign it.

.DESCRIPTION
The only thing in this repository that passes TcPackaged=true. The default
build -- CI's `windows contributor app` job, and the release job's publish step
that produces the shipping zip -- is unpackaged and is not affected by anything
here.

Signing is deliberately NOT done in this script. The release workflow signs
with Azure Trusted Signing through signtool, driven by an explicit allowlist
file, and this script's job is to produce that file with exactly one entry in
it: the package it just built. Nothing globs a directory to decide what gets
signed.

Run from Windows with Visual Studio's MSBuild on PATH. A WinUI 3 project needs
tasks from Microsoft.Build.AppxPackage.dll and
Microsoft.Build.Packaging.Pri.Tasks.dll, which ship with Visual Studio and are
never in the .NET SDK, so `dotnet build` cannot drive this.

.PARAMETER Version
Three-part version, e.g. 0.2.1. Stamped into the package as <Version>.0.

.PARAMETER OutputDir
Where the .msix is written.

.PARAMETER SignListPath
Where to write the signing allowlist. One absolute path per line.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)] [string] $Version,
    [Parameter(Mandatory = $true)] [string] $OutputDir,
    [string] $SignListPath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if ($Version -notmatch '^\d+\.\d+\.\d+$') {
    throw "version '$Version' is not three-part numeric (e.g. 0.2.1)."
}

# MSIX versions are four-part and the revision must be 0 for anything the Store
# would ever accept. The build number is not smuggled in here: two packages
# that differ only by CI run number would be two versions of the app to
# Windows, which is not what a rebuild of the same release means.
$packageVersion = "$Version.0"

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..' '..')).Path
$windowsDir = Join-Path $repoRoot 'windows'
$project = Join-Path $windowsDir 'src\TraceCommons.App\TraceCommons.App.csproj'
$manifest = Join-Path $windowsDir 'packaging\Package.appxmanifest'

foreach ($required in @($project, $manifest)) {
    if (-not (Test-Path $required -PathType Leaf)) {
        throw "missing required file: $required"
    }
}

# The publisher is the one value that cannot be wrong and cannot be guessed:
# MSIX refuses to install when it does not match the signing certificate's
# subject exactly. It was read from a published signed artifact (see
# windows/packaging/README.md) and is asserted here so that an edit to the
# manifest has to be a deliberate one in two places.
$expectedPublisher = 'CN=Iqlusion Inc, O=Iqlusion Inc, L=Santa Clara, S=California, C=US'
[xml] $manifestXml = Get-Content -Path $manifest -Raw
$identity = $manifestXml.SelectSingleNode("/*[local-name()='Package']/*[local-name()='Identity']")
if (-not $identity) { throw "$manifest has no Identity element." }
$publisher = $identity.GetAttribute("Publisher")
if ($publisher -ne $expectedPublisher) {
    throw "Package.appxmanifest Publisher is '$publisher' but this script expects '$expectedPublisher'. A publisher that does not match the signing certificate's subject produces a package that installs nowhere. If the signing identity really changed, change both."
}

$packageDir = Join-Path $repoRoot 'msix-out'
if (Test-Path $packageDir) { Remove-Item -Recurse -Force $packageDir }
New-Item -ItemType Directory -Force -Path $packageDir | Out-Null

# Run from windows/ so windows/global.json applies. MSBuild otherwise selects
# the newest installed SDK regardless of what setup-dotnet put there, and the
# runner image ships a .NET whose layout Windows App SDK 1.6 cannot build
# against.
Push-Location $windowsDir
try {
    msbuild $project `
        -restore `
        -p:Configuration=Release `
        -p:Platform=x64 `
        -p:RuntimeIdentifier=win-x64 `
        -p:SelfContained=true `
        -p:TcPackaged=true `
        -p:AppxPackageDir=$packageDir\ `
        -p:UapAppxPackageBuildMode=SideloadOnly `
        -p:AppxPackageSigningEnabled=false `
        -p:GenerateAppxPackageOnBuild=true `
        -p:AppxPackageVersion=$packageVersion
    if ($LASTEXITCODE -ne 0) { throw "msbuild MSIX packaging failed with exit code $LASTEXITCODE" }
}
finally {
    Pop-Location
}

# One package, or this stops. Get-ChildItem is used to FIND the build output
# here, which is a different thing from using it to decide what gets signed:
# the count is asserted to be exactly one and the single path is written out
# explicitly. The signing step reads that file and never scans a directory.
$packages = @(Get-ChildItem -Path $packageDir -Recurse -File -Filter '*.msix')
if ($packages.Count -ne 1) {
    throw "expected exactly 1 .msix under $packageDir, found $($packages.Count): $($packages.Name -join ', ')"
}

New-Item -ItemType Directory -Force -Path $OutputDir | Out-Null
$final = Join-Path $OutputDir "trace-commons-app-windows-x86_64-$Version.msix"
Copy-Item -Path $packages[0].FullName -Destination $final -Force
Write-Host "built $final"

if ($SignListPath) {
    # Set-Content rather than Out-File: Out-File runs its input through the
    # formatter, which has a line width, and these are long absolute paths
    # whose truncation would be silent.
    Set-Content -Path $SignListPath -Encoding utf8 -Value @((Resolve-Path $final).Path)
    Write-Host "staged for signing:"
    Get-Content $SignListPath | ForEach-Object { Write-Host "  $_" }
}
