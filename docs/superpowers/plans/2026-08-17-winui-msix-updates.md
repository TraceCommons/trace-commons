# WinUI MSIX Packaging and App Installer Updates Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the WinUI 3 contributor app as an Azure-Trusted-Signing-signed MSIX with a `.appinstaller` feed, so Windows itself owns replacing the bytes, and give the app an in-window update indicator plus an apply-now action that drains in-flight uploads first.

**Architecture:** The app converts from unpackaged (`WindowsPackageType=None`) to **single-project MSIX** — package identity is not optional, because every App Installer API refuses without it. The release pipeline builds the app with MSBuild, signs the `.msix` through the *same* Azure Trusted Signing OIDC + dlib + signtool mechanism the CLI already uses, installs and smoke-verifies it on the runner, generates a `.appinstaller` feed, and publishes both to the existing public GCS bucket. Inside the app, `Package.CheckUpdateAvailabilityAsync` drives an `InfoBar`; the apply-now action calls the daemon's `quiesce` IPC method over the in-process C ABI before handing off to `PackageManager.RequestAddPackageByAppInstallerFileAsync`, which shuts the app down and restarts it updated.

**Tech Stack:** WinUI 3 / Windows App SDK 1.6, .NET 8 (`net8.0-windows10.0.19041.0`), MSBuild from Visual Studio Build Tools, MSIX single-project packaging, PowerShell 5.1 (`System.Drawing`, `Appx` module), `signtool` + `Azure.CodeSigning.Dlib`, GitHub Actions, `gcloud storage`, xunit.

## No new dependencies are required

**No new NuGet package, and no new Cargo crate, is needed by this plan.** Every API used here is already reachable:

- `Windows.ApplicationModel.Package`, `Windows.ApplicationModel.PackageUpdateAvailabilityResult`, `Windows.Management.Deployment.PackageManager`, `AddPackageByAppInstallerOptions`, `DeploymentResult` are Windows SDK projections that come with the existing `net8.0-windows10.0.19041.0` target framework via CsWinRT. No `Microsoft.Windows.SDK.Contracts`, no `Microsoft.Windows.CsWinRT` package reference.
- MSIX packaging targets come from `Microsoft.Windows.SDK.BuildTools 10.0.22621.3233` and `Microsoft.WindowsAppSDK 1.6.240923002`, both already referenced.
- Signing reuses `Microsoft.Trusted.Signing.Client 1.0.95`, already SHA-pinned and fetched at CI time by `.github/workflows/release-apps.yml`.

If an implementer finds themselves reaching for a package, stop and raise it with the user before adding it — the repo's dependency policy requires a written workup and explicit approval.

## Global Constraints

- Reuse the existing Azure Trusted Signing setup: repository variables `AZURE_SIGNING_CLIENT_ID`, `AZURE_SIGNING_TENANT_ID`, `AZURE_SIGNING_SUBSCRIPTION_ID`, `AZURE_SIGNING_ENDPOINT`, `AZURE_SIGNING_ACCOUNT`, `AZURE_SIGNING_PROFILE`. Do not invent a second signing mechanism.
- **Any job that signs MUST declare `environment: release`.** The federated credential's subject is `repo:...:environment:release`; without the environment declaration the OIDC token does not match and `azure/login` fails at auth.
- RFC3161 timestamping is mandatory: `/tr http://timestamp.acs.microsoft.com /td SHA256`. Trusted Signing certificates carry roughly three-day validity and the countersignature is the only reason a signature outlives them.
- Fail closed. Never publish an unsigned or untimestamped package. Every signing step is immediately followed by `signtool verify /pa /v` on the same file, and a verification failure throws before anything is uploaded.
- No new NuGet packages. See the section above.
- `TraceCommons.Interop` targets plain `net8.0` and MUST stay that way. Its 23 tests run on macOS and Linux against the same Rust crate's `.dylib`/`.so`. Nothing added to that project may reference WinUI, WinRT, or `Windows.*`. Windows-only code belongs in `TraceCommons.App`.
- The WinUI app builds with **MSBuild from Visual Studio Build Tools, not `dotnet build`**. A WinUI 3 project invokes MSBuild tasks from `Microsoft.Build.AppxPackage.dll` and `Microsoft.Build.Packaging.Pri.Tasks.dll`, which ship with Visual Studio tooling and are never part of the .NET SDK; `dotnet build` fails with MSB4062 naming a path under the dotnet SDK directory. See `windows/README.md` and `.github/workflows/ci.yml:472-483`.
- `windows/global.json` pins the .NET SDK to `8.0.100` with `rollForward: latestMinor`. Run every dotnet/msbuild invocation from `windows/` so that pin applies.
- `TreatWarningsAsErrors` is `true` in every C# project here. A nullable warning is a build failure.
- Hash-only, label-only logging and UI strings. Never put a URL, token, file path, or trace body into a log line, an exception message shown to a contributor, or a status string.
- Never allow a downgrade. Do not add `s4:ForceUpdateFromAnyVersion` to the `.appinstaller`; without it Windows will only move the package to a strictly higher version, which is exactly the spec's downgrade protection.
- No emojis anywhere — code, commits, PR bodies, UI strings. Commit subjects are short and imperative with no `feat:`/`fix:` prefix.
- MSIX version quads are `Major.Minor.Build.0`, derived from the three-part release version. The revision field stays `0`.
- This subsystem **does not consume `updates/latest.json` or `updates/appcast.xml`** from `docs/superpowers/plans/2026-08-17-update-manifest-publishing.md`. The `.appinstaller` feed is its own, independent discovery channel, read by the Windows deployment service rather than by our code. The only thing the two share is the bucket they are published to.

### Cross-plan dependency

Task 5 consumes the `quiesce` IPC method defined by **Task 9 of `docs/superpowers/plans/2026-08-17-cli-self-update.md`**: method name `"quiesce"`, params `{"timeout_secs": u64?}`, success result `{"quiesced": true, "waited_ms": u64}`, refusals `busy` / `quiesce-timeout` and `bad_params` / `quiesce-requires-async`.

That method is reachable from this app: `tc_call` routes through `ipc::handle_local` -> `block_on_ipc` -> `handle_request_async` (`crates/trace-commons-contributor-ffi/src/lib.rs:785`, `crates/trace-commons-contributor/src/daemon/ipc.rs:1646-1685`), so the async-only handler answers normally over the C ABI. The `quiesce-requires-async` refusal belongs to the synchronous socket path, which this app never takes.

If that task has not landed, the daemon answers `unknown_method` and Task 5's code maps it to `TcQuiesceOutcome.Unsupported`, which refuses the apply-now action rather than updating over a live upload. That is the fail-closed behavior; this plan is implementable and testable either way.

---

### Task 1: Package identity — convert the app to single-project MSIX

**Files:**
- Modify: `windows/src/TraceCommons.App/TraceCommons.App.csproj`
- Create: `windows/src/TraceCommons.App/Package.appxmanifest`
- Create: `windows/scripts/make-app-icons.ps1`
- Create: `windows/src/TraceCommons.App/Images/Square44x44Logo.png` (generated)
- Create: `windows/src/TraceCommons.App/Images/Square150x150Logo.png` (generated)
- Create: `windows/src/TraceCommons.App/Images/StoreLogo.png` (generated)
- Modify: `windows/README.md`

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces:
  - Package identity `Name="ai.tracecommons.Contributor"`, `Application Id="TraceCommons"`. Later tasks and the `.appinstaller` `MainPackage/@Name` must use these exact strings.
  - Build output path `windows/dist/msix/` (`AppxPackageDir`).
  - MSBuild properties later tasks pass on the command line: `GenerateAppxPackageOnBuild`, `AppxPackageSigningEnabled`, `AppxBundle`, `UapAppxPackageBuildMode`.

**Why single-project MSIX rather than a Windows Application Packaging Project.** A `.wapproj` is a second project with its own platform configuration matrix, its own manifest, and its own copy of the version number, and `windows/TraceCommons.sln` would grow a fourth entry whose configuration mapping has to be maintained by hand alongside the three that exist. Its one advantage — combining several executables into one package — does not apply: this app ships exactly one executable and one native DLL, and single-project MSIX's documented limitation is on *executables*, not on content. Single-project MSIX keeps the manifest, the version, and the payload in the one project file that already knows where the Rust cdylib is.

**Why the restricted capabilities.** Two of them, and both are load-bearing:

- `rescap:packageManagement` — `PackageManager.RequestAddPackageByAppInstallerFileAsync` lists it under App capabilities. Without it the apply-now action in Task 7 throws access-denied.
- `rescap:unvirtualizedResources` plus `desktop6:FileSystemWriteVirtualization` = `disabled` — on Windows 10 1903 and later, files a packaged desktop app creates under `%LOCALAPPDATA%` are redirected to a per-user, per-package private location. `DaemonHost.DefaultConfigDir()` resolves `%LOCALAPPDATA%\trace-commons`, which is the contributor CLI daemon's state directory. Under redirection the packaged app would silently start a *second*, independent queue that shares nothing with the CLI — a correctness break that looks exactly like working software. Disabling write virtualization keeps one state directory, and Task 8 proves it on a real installed package rather than assuming it.

Both are restricted capabilities. That forecloses Microsoft Store distribution without approval, which costs nothing here: distribution is the `.appinstaller` feed, not the Store.

- [ ] **Step 1: Generate the visual assets**

Create `windows/scripts/make-app-icons.ps1`:

```powershell
#Requires -Version 5.1
<#
.SYNOPSIS
  Draws the Trace Commons brand mark into the MSIX visual assets.

.DESCRIPTION
  A direct transcription of `.brand-mark` from the community site, the same
  one macos/Sources/TraceCommonsApp/Views/DesignSystem.swift and
  crates/trace-commons-contributor-gtk/src/ui/style.css already carry:

    background-color: #ffffff
    linear-gradient(135deg, #178f70 0 38%, transparent 38% 100%)
    linear-gradient(45deg, transparent 0 45%, #315fba 45% 100%)
    1px border #d9dfdc

  The CSS layer order puts the green wedge on top, so it is painted last.
  Light-mode colours only: Windows composes tile and taskbar icons over its
  own backplate and never asks the asset for a dark variant.

  Regenerate with:
    powershell -ExecutionPolicy Bypass -File windows/scripts/make-app-icons.ps1
#>
[CmdletBinding()]
param(
    [string]$OutputDirectory = (Join-Path $PSScriptRoot '..\src\TraceCommons.App\Images')
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$surface = [System.Drawing.Color]::FromArgb(255, 0xFF, 0xFF, 0xFF)
$green   = [System.Drawing.Color]::FromArgb(255, 0x17, 0x8F, 0x70)
$blue    = [System.Drawing.Color]::FromArgb(255, 0x31, 0x5F, 0xBA)
$line    = [System.Drawing.Color]::FromArgb(255, 0xD9, 0xDF, 0xDC)

function Write-BrandMark {
    param(
        [Parameter(Mandatory = $true)][int]$Size,
        [Parameter(Mandatory = $true)][string]$Path
    )

    $bmp = New-Object System.Drawing.Bitmap(
        $Size, $Size, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    try {
        $g = [System.Drawing.Graphics]::FromImage($bmp)
        try {
            $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
            $g.Clear($surface)

            $s = [float]$Size

            # 45deg gradient, blue from 45% to 100%: everything on the
            # top-right side of the line y = x + 0.1s.
            $bluePoly = @(
                (New-Object System.Drawing.PointF(0.0,        0.0)),
                (New-Object System.Drawing.PointF($s,         0.0)),
                (New-Object System.Drawing.PointF($s,         $s)),
                (New-Object System.Drawing.PointF($s * 0.9,   $s)),
                (New-Object System.Drawing.PointF(0.0,        $s * 0.1))
            )
            $blueBrush = New-Object System.Drawing.SolidBrush($blue)
            try { $g.FillPolygon($blueBrush, [System.Drawing.PointF[]]$bluePoly) }
            finally { $blueBrush.Dispose() }

            # 135deg gradient, green from 0 to 38%: the top-left triangle cut
            # at 38% of the diagonal, so legs of 0.76s. Painted last because
            # it is the first CSS background layer, and the first layer is on
            # top.
            $greenPoly = @(
                (New-Object System.Drawing.PointF(0.0,       0.0)),
                (New-Object System.Drawing.PointF($s * 0.76, 0.0)),
                (New-Object System.Drawing.PointF(0.0,       $s * 0.76))
            )
            $greenBrush = New-Object System.Drawing.SolidBrush($green)
            try { $g.FillPolygon($greenBrush, [System.Drawing.PointF[]]$greenPoly) }
            finally { $greenBrush.Dispose() }

            $pen = New-Object System.Drawing.Pen($line, 1.0)
            try { $g.DrawRectangle($pen, 0, 0, $Size - 1, $Size - 1) }
            finally { $pen.Dispose() }
        }
        finally { $g.Dispose() }

        $bmp.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png)
    }
    finally { $bmp.Dispose() }
}

$dir = (New-Item -ItemType Directory -Force -Path $OutputDirectory).FullName

# Exactly the three the manifest references. Extra scale variants are not
# authored: MakePri accepts a single unscaled asset and Windows downsamples,
# and every extra file is another thing that can drift from the brand.
Write-BrandMark -Size 44  -Path (Join-Path $dir 'Square44x44Logo.png')
Write-BrandMark -Size 150 -Path (Join-Path $dir 'Square150x150Logo.png')
Write-BrandMark -Size 50  -Path (Join-Path $dir 'StoreLogo.png')

Write-Host "Wrote 3 assets to $dir"
```

- [ ] **Step 2: Run the generator and confirm the assets exist at the right sizes**

Run (from the repository root, on Windows):

```powershell
powershell -ExecutionPolicy Bypass -File windows\scripts\make-app-icons.ps1
Add-Type -AssemblyName System.Drawing
Get-ChildItem windows\src\TraceCommons.App\Images\*.png | ForEach-Object {
  $i = [System.Drawing.Image]::FromFile($_.FullName)
  "{0} {1}x{2}" -f $_.Name, $i.Width, $i.Height
  $i.Dispose()
}
```

Expected output, exactly these three lines in some order:

```
Square150x150Logo.png 150x150
Square44x44Logo.png 44x44
StoreLogo.png 50x50
```

- [ ] **Step 3: Write the package manifest**

Create `windows/src/TraceCommons.App/Package.appxmanifest`:

```xml
<?xml version="1.0" encoding="utf-8"?>
<!--
  Package identity for the Windows contributor app.

  Identity/@Name and Identity/@Publisher are the app's permanent identity on
  every machine that installs it. Changing either one produces a DIFFERENT
  app that installs alongside the old one rather than updating it, so neither
  is a value to adjust casually.

  Identity/@Version and Identity/@Publisher are rewritten at release time by
  windows/scripts/stamp-package-identity.ps1: the version from the release
  tag, and the publisher from the subject of the certificate Azure Trusted
  Signing actually issued. The values checked in here are what a local
  unsigned developer build uses.
-->
<Package
  xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10"
  xmlns:uap="http://schemas.microsoft.com/appx/manifest/uap/windows10"
  xmlns:rescap="http://schemas.microsoft.com/appx/manifest/foundation/windows10/restrictedcapabilities"
  xmlns:desktop6="http://schemas.microsoft.com/appx/manifest/desktop/windows10/6"
  IgnorableNamespaces="uap rescap desktop6">

  <Identity
    Name="ai.tracecommons.Contributor"
    Publisher="CN=TraceCommons Development, O=TraceCommons, C=US"
    Version="0.0.1.0"
    ProcessorArchitecture="x64" />

  <Properties>
    <DisplayName>Trace Commons</DisplayName>
    <PublisherDisplayName>Iqlusion Inc</PublisherDisplayName>
    <Logo>Images\StoreLogo.png</Logo>

    <!--
      Write virtualization OFF, deliberately.

      On Windows 10 1903 and later, files a packaged desktop app creates
      under %LOCALAPPDATA% are redirected to a per-user, per-package private
      location. DaemonHost.DefaultConfigDir() resolves
      %LOCALAPPDATA%\trace-commons -- the contributor CLI daemon's state
      directory -- and under redirection this app would quietly maintain a
      SECOND queue that the CLI cannot see and that the daemon lock cannot
      arbitrate. One state directory per machine is the design; this element
      is what keeps it true. It requires the unvirtualizedResources
      restricted capability declared below.
    -->
    <desktop6:FileSystemWriteVirtualization>disabled</desktop6:FileSystemWriteVirtualization>
    <desktop6:RegistryWriteVirtualization>disabled</desktop6:RegistryWriteVirtualization>
  </Properties>

  <!--
    19041 (Windows 10, version 2004), not 17763. Three things need it:
    desktop6 write virtualization control, the app's own
    net8.0-windows10.0.19041.0 target framework, and Package
    .CheckUpdateAvailabilityAsync's 1809 floor comfortably cleared.
  -->
  <Dependencies>
    <TargetDeviceFamily Name="Windows.Desktop"
                        MinVersion="10.0.19041.0"
                        MaxVersionTested="10.0.22621.0" />
  </Dependencies>

  <Resources>
    <Resource Language="en-US" />
  </Resources>

  <Applications>
    <Application Id="TraceCommons"
                 Executable="TraceCommons.exe"
                 EntryPoint="Windows.FullTrustApplication">
      <uap:VisualElements
        DisplayName="Trace Commons"
        Description="Review and approve the sessions that leave your machine."
        BackgroundColor="transparent"
        Square150x150Logo="Images\Square150x150Logo.png"
        Square44x44Logo="Images\Square44x44Logo.png" />
    </Application>
  </Applications>

  <Capabilities>
    <!-- A full-trust desktop app. Required for anything that is not UWP. -->
    <rescap:Capability Name="runFullTrust" />

    <!--
      PackageManager.RequestAddPackageByAppInstallerFileAsync lists
      packageManagement under App capabilities. Without it the in-app
      "Update and restart" action throws access-denied, and the app can only
      wait for the scheduled OnLaunch check.
    -->
    <rescap:Capability Name="packageManagement" />

    <!-- Required by desktop6:FileSystemWriteVirtualization above. -->
    <rescap:Capability Name="unvirtualizedResources" />
  </Capabilities>
</Package>
```

- [ ] **Step 4: Convert the project file**

In `windows/src/TraceCommons.App/TraceCommons.App.csproj`, replace this block:

```xml
    <TargetPlatformMinVersion>10.0.17763.0</TargetPlatformMinVersion>
```

with:

```xml
    <TargetPlatformMinVersion>10.0.19041.0</TargetPlatformMinVersion>
```

Then replace this block:

```xml
    <!--
      Unpackaged (WindowsPackageType=None) rather than MSIX for this slice.

      MSIX is the right end state for shipping — it is how the macOS app's
      bundle and the Linux shell's desktop entry are matched on Windows — but
      it needs signing identity and a packaging project, neither of which
      exists yet. Unpackaged builds and runs from CI with no certificate, so
      the interop path can be proven on Windows before packaging work starts.
    -->
    <WindowsPackageType>None</WindowsPackageType>
    <WindowsAppSDKSelfContained>true</WindowsAppSDKSelfContained>
    <SelfContained>true</SelfContained>
```

with:

```xml
    <!--
      MSIX, via single-project packaging rather than a separate .wapproj.

      Package identity is not a preference here: every App Installer API the
      update flow uses -- Package.CheckUpdateAvailabilityAsync and
      PackageManager.RequestAddPackageByAppInstallerFileAsync -- refuses
      without it, so an unpackaged build has no update path at all.

      Single-project rather than a Windows Application Packaging Project
      because its one real limitation is a single executable per package,
      and this app ships exactly one. A .wapproj would add a fourth project
      to the solution carrying a duplicate of the version, the manifest and
      the platform matrix.
    -->
    <WindowsPackageType>MSIX</WindowsPackageType>
    <EnableMsixTooling>true</EnableMsixTooling>

    <!--
      Self-contained stays on under MSIX. It keeps the Windows App Runtime
      out of the package's Dependencies, which keeps it out of the
      .appinstaller feed: a framework reference there is one more artifact
      the deployment service must reach, on a host we do not control.
    -->
    <WindowsAppSDKSelfContained>true</WindowsAppSDKSelfContained>
    <SelfContained>true</SelfContained>

    <!--
      Packaging is OFF by default and turned on from the command line with
      -p:GenerateAppxPackageOnBuild=true. A plain `msbuild` stays a fast
      compile check; only the release job and the CI packaging step pay for
      MakeAppx and MakePri.

      Signing is OFF here on purpose. The package is signed afterwards by
      signtool against Azure Trusted Signing, which holds no local key --
      AppxPackageSigningEnabled would look for a .pfx that must never exist
      in this repository.
    -->
    <GenerateAppxPackageOnBuild>false</GenerateAppxPackageOnBuild>
    <AppxPackageSigningEnabled>false</AppxPackageSigningEnabled>
    <AppxBundle>Never</AppxBundle>
    <UapAppxPackageBuildMode>SideloadOnly</UapAppxPackageBuildMode>
    <AppxPackageDir>$(MSBuildThisFileDirectory)..\..\dist\msix\</AppxPackageDir>
    <AppxSymbolPackageEnabled>false</AppxSymbolPackageEnabled>
```

Then add this `ItemGroup` immediately after the existing `ItemGroup` that holds the two `PackageReference` elements:

```xml
  <ItemGroup>
    <AppxManifest Include="Package.appxmanifest">
      <SubType>Designer</SubType>
    </AppxManifest>
    <Content Include="Images\**\*.png" />
  </ItemGroup>
```

- [ ] **Step 5: Build the package and confirm it is produced with the right identity**

Run (from `windows/`, on Windows, with the cdylib already built):

```powershell
cargo build -p trace-commons-contributor-ffi --release
msbuild src\TraceCommons.App\TraceCommons.App.csproj -restore `
  -p:Configuration=Release -p:Platform=x64 -p:GenerateAppxPackageOnBuild=true
$msix = @(Get-ChildItem -Recurse -Filter *.msix -Path dist\msix)
$msix.Count
$msix[0].Name
```

Expected: the build succeeds, `$msix.Count` prints `1`, and the name is `ai.tracecommons.Contributor_0.0.1.0_x64.msix`.

- [ ] **Step 6: Confirm the manifest inside the package carries the identity later tasks depend on**

Run (from `windows/`):

```powershell
$sdk = (Get-ChildItem "C:\Program Files (x86)\Windows Kits\10\bin\*\x64\makeappx.exe" |
        Sort-Object { [version]$_.Directory.Parent.Name } -Descending |
        Select-Object -First 1 -ExpandProperty FullName)
$msix = (Get-ChildItem -Recurse -Filter *.msix -Path dist\msix | Select-Object -First 1).FullName
& $sdk unpack /p $msix /d "$env:TEMP\tc-msix-check" /o
[xml]$m = Get-Content "$env:TEMP\tc-msix-check\AppxManifest.xml"
$m.Package.Identity.Name
$m.Package.Identity.ProcessorArchitecture
$m.Package.Applications.Application.Id
```

Expected output:

```
ai.tracecommons.Contributor
x64
TraceCommons
```

- [ ] **Step 7: Correct the README**

Two edits to `windows/README.md`.

First, under "What is not here yet", delete these two lines entirely — the tray
bullet above them already covers what is genuinely still missing:

    - MSIX packaging and signing. The app builds unpackaged so that CI can verify
      it without a certificate.

Second, insert a new section immediately before the `## What is not here yet`
heading. Its text, verbatim (the indented block is a fenced `powershell` code
block in the README, written here indented so it does not terminate this plan's
own fence):

    ## Packaging

    The app ships as an MSIX built by single-project packaging, signed through the
    same Azure Trusted Signing account the contributor CLI uses, and distributed
    through a `.appinstaller` feed that Windows polls on its own schedule.

    ```powershell
    # From windows/. Packaging is off by default; a plain msbuild is a compile
    # check. dist/msix/ is the output directory.
    msbuild src\TraceCommons.App\TraceCommons.App.csproj -restore `
      -p:Configuration=Release -p:Platform=x64 -p:GenerateAppxPackageOnBuild=true
    ```

    The package is deliberately unsigned at build time. `AppxPackageSigningEnabled`
    is `false` because Trusted Signing holds no local key and the signature is
    applied afterwards by `signtool` against a short-lived certificate issued to
    the release job's OIDC token. There is no `.pfx` in this repository and there
    must never be one.

    Package identity is `ai.tracecommons.Contributor`, application id
    `TraceCommons`. Both are permanent: changing either produces a different app
    that installs alongside the old one instead of updating it.

- [ ] **Step 8: Commit**

```bash
git add windows/src/TraceCommons.App/TraceCommons.App.csproj \
        windows/src/TraceCommons.App/Package.appxmanifest \
        windows/src/TraceCommons.App/Images \
        windows/scripts/make-app-icons.ps1 \
        windows/README.md
git commit -m "Give the Windows app package identity via single-project MSIX"
```

---

### Task 2: Stamp the release version and the real certificate subject into the manifest

**Files:**
- Create: `windows/scripts/stamp-package-identity.ps1`

**Interfaces:**
- Consumes: `windows/src/TraceCommons.App/Package.appxmanifest` from Task 1.
- Produces: `windows/scripts/stamp-package-identity.ps1`, invoked as
  `stamp-package-identity.ps1 -ManifestPath <path> -Version <X.Y.Z> -Publisher <subject>`,
  writing the manifest in place and emitting the resulting quad version on stdout.

**Why the publisher is discovered rather than configured.** `Identity/@Publisher` must match the signing certificate's subject *exactly* or `signtool` refuses the package. Hard-coding a subject means a silent drift the day the Trusted Signing profile's identity validation is re-issued, and a new repository variable means one more thing an operator must keep correct. Instead the release job signs a throwaway copy of the app's own executable, reads back the subject Windows itself reports for that signature, and stamps that. The value cannot be wrong, because it is the value the signature will carry.

- [ ] **Step 1: Write the script**

Create `windows/scripts/stamp-package-identity.ps1`:

```powershell
#Requires -Version 5.1
<#
.SYNOPSIS
  Rewrites Identity/@Version and Identity/@Publisher in a Package.appxmanifest.

.DESCRIPTION
  Called by the release job between building the app and packaging it.

  The version arrives as the three-part release version (matching the
  app-vX.Y.Z tag validation in release-apps.yml) and is written as the quad
  MSIX requires, with the revision field held at 0. The publisher arrives as
  the subject of the certificate Azure Trusted Signing actually issued, read
  back from a signature rather than configured, so it cannot drift from what
  signtool will demand.

  Both values are validated before anything is written. A manifest half
  rewritten with a bad version is a package that installs and then cannot be
  updated.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$ManifestPath,
    [Parameter(Mandatory = $true)][string]$Version,
    [Parameter(Mandatory = $true)][string]$Publisher
)

$ErrorActionPreference = 'Stop'

if ($Version -notmatch '^[0-9]+\.[0-9]+\.[0-9]+$') {
    throw "Version must be three-part numeric (e.g. 1.2.3); got '$Version'."
}

# The subject must be an X.500 name signtool can match, and the appinstaller
# file is documented as accepting only ASCII. A non-ASCII subject would
# produce a feed Windows rejects at parse time, days after release.
if ($Publisher -notmatch '^[\x20-\x7E]+$') {
    throw "Publisher contains non-ASCII characters, which an .appinstaller file cannot carry."
}
if ($Publisher -notmatch 'CN=') {
    throw "Publisher does not look like a certificate subject (no CN=): '$Publisher'."
}

$quad = "$Version.0"

$xml = New-Object System.Xml.XmlDocument
$xml.PreserveWhitespace = $true
$xml.Load((Resolve-Path $ManifestPath).Path)

$ns = New-Object System.Xml.XmlNamespaceManager($xml.NameTable)
$ns.AddNamespace('f', 'http://schemas.microsoft.com/appx/manifest/foundation/windows10')

$identity = $xml.SelectSingleNode('/f:Package/f:Identity', $ns)
if ($null -eq $identity) {
    throw "No /Package/Identity element in $ManifestPath."
}

$identity.SetAttribute('Version', $quad)
$identity.SetAttribute('Publisher', $Publisher)

$xml.Save((Resolve-Path $ManifestPath).Path)

Write-Output $quad
```

- [ ] **Step 2: Run it against a copy and confirm both attributes changed**

Run (from the repository root, on Windows):

```powershell
Copy-Item windows\src\TraceCommons.App\Package.appxmanifest "$env:TEMP\probe.appxmanifest" -Force
$quad = & windows\scripts\stamp-package-identity.ps1 `
  -ManifestPath "$env:TEMP\probe.appxmanifest" `
  -Version 1.2.3 -Publisher "CN=Iqlusion Inc, O=Iqlusion Inc, C=US"
$quad
[xml]$m = Get-Content "$env:TEMP\probe.appxmanifest"
$m.Package.Identity.Version
$m.Package.Identity.Publisher
$m.Package.Identity.Name
```

Expected output:

```
1.2.3.0
1.2.3.0
CN=Iqlusion Inc, O=Iqlusion Inc, C=US
ai.tracecommons.Contributor
```

- [ ] **Step 3: Confirm it refuses bad input rather than writing a broken manifest**

Run:

```powershell
& windows\scripts\stamp-package-identity.ps1 `
  -ManifestPath "$env:TEMP\probe.appxmanifest" -Version 1.2 `
  -Publisher "CN=Iqlusion Inc"
```

Expected: a terminating error reading `Version must be three-part numeric (e.g. 1.2.3); got '1.2'.` and a non-zero exit. Then run:

```powershell
& windows\scripts\stamp-package-identity.ps1 `
  -ManifestPath "$env:TEMP\probe.appxmanifest" -Version 1.2.3 `
  -Publisher "Iqlusion Inc"
```

Expected: a terminating error reading `Publisher does not look like a certificate subject (no CN=): 'Iqlusion Inc'.`

- [ ] **Step 4: Commit**

```bash
git add windows/scripts/stamp-package-identity.ps1
git commit -m "Stamp release version and certificate subject into the package manifest"
```

---

### Task 3: Extract the Trusted Signing steps into scripts both jobs use

**Files:**
- Create: `scripts/windows/setup-trusted-signing.ps1`
- Create: `scripts/windows/sign-with-trusted-signing.ps1`
- Modify: `.github/workflows/release-apps.yml` (the `windows` job's "Set up Trusted Signing", "Sign" and "Verify the signatures" steps)

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces:
  - `scripts/windows/setup-trusted-signing.ps1` — parameters `-Endpoint -Account -Profile -ExpectedSha256 [-NuGetVersion]`. Writes `dlib=`, `metadata=` and `signtool=` to `$env:GITHUB_OUTPUT` when set, and always emits the same three as `name=value` lines on stdout.
  - `scripts/windows/sign-with-trusted-signing.ps1` — parameters `-SignTool -Dlib -Metadata -Path <string[]>`. Signs each path with an RFC3161 countersignature and then verifies it, throwing on either failure.

**Why extract now.** The MSIX job needs the identical setup, and the setup carries a hand-verified SHA-256 pin of Microsoft's signing client. Two copies of a pin is a pin that drifts, and the copy that drifts is the one nobody bumped. One script, two callers.

- [ ] **Step 1: Write the setup script**

Create `scripts/windows/setup-trusted-signing.ps1`:

```powershell
#Requires -Version 5.1
<#
.SYNOPSIS
  Fetches and verifies Microsoft's Trusted Signing client, and locates signtool.

.DESCRIPTION
  No signing key reaches the runner: Trusted Signing issues a short-lived
  certificate against the job's OIDC token, and the dlib authenticates
  through DefaultAzureCredential picking up the session azure/login left
  behind. The caller must therefore have run azure/login first, and its job
  must declare `environment: release` -- the federated credential's subject is
  repo:...:environment:release and no other environment matches.

  Microsoft's client is fetched rather than vendored, so it is verified BY
  CONTENT before anything is extracted or executed. A job that runs this holds
  signing authority; a tampered download here would sign whatever an attacker
  liked with a public-trust certificate.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Endpoint,
    [Parameter(Mandatory = $true)][string]$Account,
    [Parameter(Mandatory = $true)][string]$Profile,
    [Parameter(Mandatory = $true)][string]$ExpectedSha256,
    [string]$NuGetVersion = '1.0.95'
)

$ErrorActionPreference = 'Stop'

$workDir = if ($env:RUNNER_TEMP) { $env:RUNNER_TEMP } else { $env:TEMP }
$pkgZip = Join-Path $workDir 'mtsc.zip'
$pkgDir = Join-Path $workDir 'mtsc'

Invoke-WebRequest -UseBasicParsing `
  -Uri "https://www.nuget.org/api/v2/package/Microsoft.Trusted.Signing.Client/$NuGetVersion" `
  -OutFile $pkgZip

$actual = (Get-FileHash -Path $pkgZip -Algorithm SHA256).Hash
Write-Host "Microsoft.Trusted.Signing.Client $NuGetVersion SHA-256: $actual"
if ([string]::IsNullOrWhiteSpace($ExpectedSha256)) {
    throw "ExpectedSha256 is not set (observed: $actual). Refusing to extract an unverified signing package."
}
if ($actual -ne $ExpectedSha256.Trim().ToUpperInvariant()) {
    throw "Hash mismatch for Microsoft.Trusted.Signing.Client $NuGetVersion. Expected $($ExpectedSha256.Trim().ToUpperInvariant()), got $actual. Refusing to expand a potentially tampered signing package."
}

Expand-Archive -Path $pkgZip -DestinationPath $pkgDir -Force
$dlib = Join-Path $pkgDir 'bin\x64\Azure.CodeSigning.Dlib.dll'
if (-not (Test-Path $dlib)) {
    $dlib = (Get-ChildItem -Path $pkgDir -Recurse -Filter 'Azure.CodeSigning.Dlib.dll' |
             Select-Object -First 1 -ExpandProperty FullName)
}
if (-not $dlib) { throw 'Azure.CodeSigning.Dlib.dll not found in package' }

$metadata = Join-Path $workDir 'ts-metadata.json'
[ordered]@{
    Endpoint               = $Endpoint
    CodeSigningAccountName = $Account
    CertificateProfileName = $Profile
} | ConvertTo-Json | Out-File -FilePath $metadata -Encoding utf8

$signtool = (Get-ChildItem 'C:\Program Files (x86)\Windows Kits\10\bin\*\x64\signtool.exe' `
               -ErrorAction SilentlyContinue |
             Sort-Object { [version]$_.Directory.Parent.Name } -Descending |
             Select-Object -First 1 -ExpandProperty FullName)
if (-not $signtool) { throw 'signtool.exe not found in Windows SDK' }

$lines = @("dlib=$dlib", "metadata=$metadata", "signtool=$signtool")
if ($env:GITHUB_OUTPUT) {
    $lines | Out-File -FilePath $env:GITHUB_OUTPUT -Encoding utf8 -Append
}
$lines | ForEach-Object { Write-Output $_ }
```

- [ ] **Step 2: Write the sign-and-verify script**

Create `scripts/windows/sign-with-trusted-signing.ps1`:

```powershell
#Requires -Version 5.1
<#
.SYNOPSIS
  Signs files with Azure Trusted Signing and verifies each signature.

.DESCRIPTION
  Signing and verification are one script, not two steps, so there is no
  arrangement of the workflow in which a file is signed and then shipped
  without a verifier having looked at it. `signtool verify /pa` applies the
  Authenticode policy an end user's machine applies, which is the only
  opinion that matters.

  /tr plus /td is the RFC3161 countersignature and it is NOT optional.
  Trusted Signing certificates carry roughly three-day validity, and the
  timestamp is the only reason a signature outlives them. An untimestamped
  artifact starts failing validation days after release -- a failure no
  same-day test would catch.

  Works on .exe, .dll and .msix alike: signtool treats an MSIX as a signable
  container and rewrites its AppxSignature.p7x.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$SignTool,
    [Parameter(Mandatory = $true)][string]$Dlib,
    [Parameter(Mandatory = $true)][string]$Metadata,
    [Parameter(Mandatory = $true)][string[]]$Path
)

$ErrorActionPreference = 'Stop'

if ($Path.Count -eq 0) {
    throw 'No files were passed to sign. Refusing to report success over an empty set.'
}

foreach ($file in $Path) {
    $full = (Resolve-Path $file).Path

    & $SignTool sign /v /fd SHA256 `
      /tr http://timestamp.acs.microsoft.com /td SHA256 `
      /dlib $Dlib /dmdf $Metadata $full
    if ($LASTEXITCODE -ne 0) { throw "signing failed for $(Split-Path -Leaf $full)" }

    & $SignTool verify /pa /v $full
    if ($LASTEXITCODE -ne 0) { throw "signature verification failed for $(Split-Path -Leaf $full)" }
}
```

- [ ] **Step 3: Point the existing CLI job at the scripts**

In `.github/workflows/release-apps.yml`, in the `windows` job, replace the whole `- name: Set up Trusted Signing (SHA-verified dlib + signtool)` step body's `run:` block with a call to the script, keeping the `id: ts` and every `env:` value unchanged:

```yaml
      - name: Set up Trusted Signing (SHA-verified dlib + signtool)
        id: ts
        shell: pwsh
        env:
          TS_ENDPOINT: ${{ vars.AZURE_SIGNING_ENDPOINT }}
          TS_ACCOUNT: ${{ vars.AZURE_SIGNING_ACCOUNT }}
          TS_PROFILE: ${{ vars.AZURE_SIGNING_PROFILE }}
          # Independently verified 2026-08-16 by downloading the package and
          # hashing it. When bumping the version, re-derive this from a
          # trusted machine -- do not copy a hash from anywhere.
          TRUSTED_SIGNING_CLIENT_SHA256: 3BFCF1E0A3CB42AF1692F0A8ED45C15DE070C2DE86F28A59B2795D904D8A920F
        run: |
          ./scripts/windows/setup-trusted-signing.ps1 `
            -Endpoint $env:TS_ENDPOINT `
            -Account $env:TS_ACCOUNT `
            -Profile $env:TS_PROFILE `
            -ExpectedSha256 $env:TRUSTED_SIGNING_CLIENT_SHA256
```

Then replace the two steps `- name: Sign` and `- name: Verify the signatures` with the single step:

```yaml
      - name: Sign and verify
        shell: pwsh
        env:
          TS_SIGNTOOL: ${{ steps.ts.outputs.signtool }}
          TS_DLIB: ${{ steps.ts.outputs.dlib }}
          TS_METADATA: ${{ steps.ts.outputs.metadata }}
        run: |
          $ErrorActionPreference = "Stop"
          $files = @(Get-ChildItem signed\*.exe | ForEach-Object { $_.FullName })
          ./scripts/windows/sign-with-trusted-signing.ps1 `
            -SignTool $env:TS_SIGNTOOL -Dlib $env:TS_DLIB `
            -Metadata $env:TS_METADATA -Path $files
```

- [ ] **Step 4: Prove the extraction against the real signing service without publishing**

`workflow_dispatch` builds and signs but never publishes — the `publish` job is gated on `github.event_name == 'push'`. So this exercises the whole signing path with no release consequence.

Run:

```bash
gh workflow run release-apps.yml --repo TraceCommons/trace-commons-server \
  -f platform=windows -f version=0.0.0
gh run list --repo TraceCommons/trace-commons-server \
  --workflow release-apps.yml --limit 1
```

Then watch it and confirm the `Windows signed binaries` job succeeds, with the `Sign and verify` step's log containing `Successfully verified: ` and a `The signature is timestamped:` line.

- [ ] **Step 5: Commit**

```bash
git add scripts/windows/setup-trusted-signing.ps1 \
        scripts/windows/sign-with-trusted-signing.ps1 \
        .github/workflows/release-apps.yml
git commit -m "Extract the Trusted Signing setup and signing steps into scripts"
```

---

### Task 4: Author the .appinstaller feed

**Files:**
- Create: `windows/scripts/make-appinstaller.ps1`

**Interfaces:**
- Consumes: the package identity from Task 1 (`ai.tracecommons.Contributor`) and the quad version from Task 2.
- Produces: `windows/scripts/make-appinstaller.ps1`, invoked as
  `make-appinstaller.ps1 -PackageName <name> -Publisher <subject> -Version <quad> -ProcessorArchitecture <arch> -BaseUri <uri> -PackageFileName <name.msix> -OutputPath <path>`.

**What was verified about the schema, and what was chosen because of it.**

- The root default namespace is `http://schemas.microsoft.com/appx/appinstaller/2017/2` and the 2021 namespace is declared *alongside* it as a prefix, canonically `s4`: `xmlns:s4="http://schemas.microsoft.com/appx/appinstaller/2021"`. That is the shape the schema reference for `<AppInstaller>` gives, and the shape `<UpdateSettings>` gives for its children.
- `UpdateSettings` accepts exactly three children, in this order: `OnLaunch`, `s4:AutomaticBackgroundTask`, `s4:ForceUpdateFromAnyVersion`. `AutomaticBackgroundTask` is `s4:`-prefixed, so the 2021 namespace declaration is genuinely required and is not decoration.
- `OnLaunch/@HoursBetweenUpdateChecks` is unprefixed and belongs to the 2017/2 namespace. `ShowPrompt` and `UpdateBlocksActivation` are `s4:`-prefixed.
- **`s4:ShowPrompt` and `s4:UpdateBlocksActivation` are deliberately not used.** The `OnLaunch` reference states plainly that `ShowPrompt="true"` "currently shows a prompt for UWP applications but not for desktop applications that have been packaged in a Windows app package" — for a packaged desktop app like this one they provide a silent update, the same as the default. They are also precisely the two attributes that widely reported "The XML in the .appinstaller file is not valid" failures name. Spending schema risk on two attributes documented as no-ops for our app type is a bad trade; the in-app `InfoBar` from Task 7 is the prompt, and it works.
- **`s4:ForceUpdateFromAnyVersion` is deliberately not used.** Without it, Windows moves the package only to a strictly higher version. That is the spec's downgrade protection, obtained by omission.

The genuine validator is not a schema file — it is Windows. Task 8 installs from the generated feed on the runner before anything is published, which is the only check that proves the deployment service accepts it.

- [ ] **Step 1: Write the generator**

Create `windows/scripts/make-appinstaller.ps1`:

```powershell
#Requires -Version 5.1
<#
.SYNOPSIS
  Writes the .appinstaller feed the Windows deployment service polls.

.DESCRIPTION
  The feed is this subsystem's own discovery channel. It is NOT
  updates/latest.json and NOT updates/appcast.xml -- nothing in this app
  reads either of those, because on Windows desktop the platform installer,
  not the app, discovers and applies updates. The only thing the three share
  is the bucket they are published to.

  Written with XmlWriter rather than string concatenation so the file cannot
  be malformed by an unescaped character in a certificate subject, and with
  a UTF8Encoding that emits no byte-order mark: the AppInstaller element's
  documented requirement is encoding="UTF-8" with no escape characters and
  no non-ascii characters.

  Namespaces: the root is the 2017/2 namespace, with the 2021 namespace
  declared as s4 because AutomaticBackgroundTask lives there. ShowPrompt and
  UpdateBlocksActivation are not written -- both are documented as no-ops for
  desktop applications packaged in a Windows app package, and the in-app
  InfoBar is the prompt instead. ForceUpdateFromAnyVersion is not written
  either: without it Windows will only move to a strictly higher version,
  which is the downgrade protection this project requires.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$PackageName,
    [Parameter(Mandatory = $true)][string]$Publisher,
    [Parameter(Mandatory = $true)][string]$Version,
    [Parameter(Mandatory = $true)][ValidateSet('x86', 'x64', 'arm64', 'neutral')][string]$ProcessorArchitecture,
    [Parameter(Mandatory = $true)][string]$BaseUri,
    [Parameter(Mandatory = $true)][string]$PackageFileName,
    [Parameter(Mandatory = $true)][string]$OutputPath
)

$ErrorActionPreference = 'Stop'

$ns2017 = 'http://schemas.microsoft.com/appx/appinstaller/2017/2'
$ns2021 = 'http://schemas.microsoft.com/appx/appinstaller/2021'

if ($Version -notmatch '^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$') {
    throw "Version must be a quad (e.g. 1.2.3.0); got '$Version'."
}
foreach ($pair in @(
        @{ Name = 'PackageName'; Value = $PackageName },
        @{ Name = 'Publisher';   Value = $Publisher },
        @{ Name = 'BaseUri';     Value = $BaseUri })) {
    if ($pair.Value -notmatch '^[\x20-\x7E]+$') {
        throw "$($pair.Name) contains non-ASCII characters, which an .appinstaller file cannot carry."
    }
}

$base = $BaseUri.TrimEnd('/')
if ($base -notmatch '^https://') {
    throw "BaseUri must be https; got '$BaseUri'."
}

$feedName = 'TraceCommons.appinstaller'
$selfUri = "$base/$feedName"
$packageUri = "$base/$PackageFileName"

$settings = New-Object System.Xml.XmlWriterSettings
$settings.Indent = $true
$settings.IndentChars = '  '
$settings.Encoding = New-Object System.Text.UTF8Encoding($false)

$dir = Split-Path -Parent $OutputPath
if ($dir -and -not (Test-Path $dir)) {
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
}

$writer = [System.Xml.XmlWriter]::Create($OutputPath, $settings)
try {
    $writer.WriteStartDocument()

    $writer.WriteStartElement('AppInstaller', $ns2017)
    $writer.WriteAttributeString('xmlns', 's4', $null, $ns2021)
    $writer.WriteAttributeString('Version', $Version)
    $writer.WriteAttributeString('Uri', $selfUri)

    $writer.WriteStartElement('MainPackage', $ns2017)
    $writer.WriteAttributeString('Name', $PackageName)
    $writer.WriteAttributeString('Publisher', $Publisher)
    $writer.WriteAttributeString('Version', $Version)
    $writer.WriteAttributeString('ProcessorArchitecture', $ProcessorArchitecture)
    $writer.WriteAttributeString('Uri', $packageUri)
    $writer.WriteEndElement()

    $writer.WriteStartElement('UpdateSettings', $ns2017)

    # 8 hours rather than 0. A check on every launch is a network round trip
    # in front of a window a contributor opened to approve one session, and
    # the background task below already covers a machine that stays open.
    $writer.WriteStartElement('OnLaunch', $ns2017)
    $writer.WriteAttributeString('HoursBetweenUpdateChecks', '8')
    $writer.WriteEndElement()

    # Every 8 hours whether or not the app was launched. This is the branch
    # that reaches a contributor who leaves the app closed for a week, which
    # is exactly the population running the oldest redaction code.
    $writer.WriteStartElement('AutomaticBackgroundTask', $ns2021)
    $writer.WriteEndElement()

    $writer.WriteEndElement()  # UpdateSettings
    $writer.WriteEndElement()  # AppInstaller
    $writer.WriteEndDocument()
}
finally {
    $writer.Flush()
    $writer.Close()
}

Write-Output (Resolve-Path $OutputPath).Path
```

- [ ] **Step 2: Generate a feed and confirm its exact shape**

Run (from the repository root; works on Windows PowerShell and on pwsh anywhere):

```powershell
& windows/scripts/make-appinstaller.ps1 `
  -PackageName ai.tracecommons.Contributor `
  -Publisher "CN=Iqlusion Inc, O=Iqlusion Inc, C=US" `
  -Version 1.2.3.0 -ProcessorArchitecture x64 `
  -BaseUri https://storage.googleapis.com/tracecommons-flatpak/windows `
  -PackageFileName ai.tracecommons.Contributor_1.2.3.0_x64.msix `
  -OutputPath "$env:TEMP/TraceCommons.appinstaller"
Get-Content "$env:TEMP/TraceCommons.appinstaller" -Raw
```

Expected output (after the resolved path line):

```xml
<?xml version="1.0" encoding="utf-8"?>
<AppInstaller xmlns:s4="http://schemas.microsoft.com/appx/appinstaller/2021" Version="1.2.3.0" Uri="https://storage.googleapis.com/tracecommons-flatpak/windows/TraceCommons.appinstaller" xmlns="http://schemas.microsoft.com/appx/appinstaller/2017/2">
  <MainPackage Name="ai.tracecommons.Contributor" Publisher="CN=Iqlusion Inc, O=Iqlusion Inc, C=US" Version="1.2.3.0" ProcessorArchitecture="x64" Uri="https://storage.googleapis.com/tracecommons-flatpak/windows/ai.tracecommons.Contributor_1.2.3.0_x64.msix" />
  <UpdateSettings>
    <OnLaunch HoursBetweenUpdateChecks="8" />
    <s4:AutomaticBackgroundTask />
  </UpdateSettings>
</AppInstaller>
```

(`XmlWriter` emits the `xmlns:s4` declaration before the default `xmlns`; attribute order within a tag is not significant to any XML parser.)

- [ ] **Step 3: Confirm it parses and the namespaces resolve as intended**

Run:

```powershell
[xml]$feed = Get-Content "$env:TEMP/TraceCommons.appinstaller"
$ns = New-Object System.Xml.XmlNamespaceManager($feed.NameTable)
$ns.AddNamespace('a', 'http://schemas.microsoft.com/appx/appinstaller/2017/2')
$ns.AddNamespace('s4', 'http://schemas.microsoft.com/appx/appinstaller/2021')
$feed.SelectSingleNode('/a:AppInstaller/a:UpdateSettings/a:OnLaunch', $ns).HoursBetweenUpdateChecks
($feed.SelectNodes('/a:AppInstaller/a:UpdateSettings/s4:AutomaticBackgroundTask', $ns)).Count
($feed.SelectNodes('//s4:ForceUpdateFromAnyVersion', $ns)).Count
```

Expected output:

```
8
1
0
```

- [ ] **Step 4: Confirm it refuses a non-https base and a bad version**

Run:

```powershell
& windows/scripts/make-appinstaller.ps1 -PackageName ai.tracecommons.Contributor `
  -Publisher "CN=Iqlusion Inc" -Version 1.2.3.0 -ProcessorArchitecture x64 `
  -BaseUri http://example.invalid/windows -PackageFileName a.msix `
  -OutputPath "$env:TEMP/bad.appinstaller"
```

Expected: a terminating error reading `BaseUri must be https; got 'http://example.invalid/windows'.` Then run the same command with `-BaseUri https://example.invalid/windows -Version 1.2.3`; expected: `Version must be a quad (e.g. 1.2.3.0); got '1.2.3'.`

- [ ] **Step 5: Commit**

```bash
git add windows/scripts/make-appinstaller.ps1
git commit -m "Generate the appinstaller feed with the 2021 namespace for the background task"
```

---

### Task 5: Read the daemon's quiesce answer

**Files:**
- Create: `windows/src/TraceCommons.Interop/UpdateProtocol.cs`
- Modify: `windows/src/TraceCommons.Interop/DaemonProtocol.cs` (add `Methods.Quiesce`)
- Create: `windows/tests/TraceCommons.Interop.Tests/UpdateProtocolTests.cs`

**Interfaces:**
- Consumes: `DaemonResponse`, `DaemonError`, `DaemonProtocol.SerializerOptions` from `DaemonProtocol.cs`.
- Produces, all in namespace `TraceCommons.Interop`:
  - `public const string DaemonProtocol.Methods.Quiesce = "quiesce"`
  - `public enum TcQuiesceOutcome { Quiesced, Busy, TimedOut, Unsupported, Unavailable }`
  - `public sealed class QuiesceOutcome` with `public TcQuiesceOutcome Outcome { get; }`, `public long WaitedMs { get; }`, `public bool CanUpdate { get; }`
  - `public static QuiesceOutcome UpdateProtocol.ReadQuiesce(DaemonResponse response)`
  - `public static string UpdateProtocol.DescribeRefusal(TcQuiesceOutcome outcome)`

This file is pure JSON handling and lives in the `net8.0` interop assembly on purpose: its tests then run on macOS and Linux alongside the existing 23, which is the whole reason that assembly is not `net8.0-windows`.

- [ ] **Step 1: Write the failing tests**

Create `windows/tests/TraceCommons.Interop.Tests/UpdateProtocolTests.cs`:

```csharp
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// Tests for reading the daemon's answer to <c>quiesce</c>.
///
/// This mapping is the gate in front of an update: everything except a
/// confirmed quiesce must refuse, because the alternative is App Installer
/// terminating the process while an upload is on the wire. So each refusal
/// shape gets its own test rather than being lumped into a catch-all --
/// a refusal misread as success is the one failure that costs a contributor
/// a half-uploaded trace.
///
/// Pure JSON, no native library and no daemon, so these run anywhere.
/// </summary>
public class UpdateProtocolTests
{
    [Fact]
    public void AConfirmedQuiesceAllowsTheUpdate()
    {
        QuiesceOutcome outcome = UpdateProtocol.ReadQuiesce(
            DaemonResponse.Parse("{\"id\":1,\"result\":{\"quiesced\":true,\"waited_ms\":1200}}"));

        Assert.Equal(TcQuiesceOutcome.Quiesced, outcome.Outcome);
        Assert.Equal(1200, outcome.WaitedMs);
        Assert.True(outcome.CanUpdate);
    }

    [Fact]
    public void ATimeoutIsDistinguishedFromAPlainRefusal()
    {
        QuiesceOutcome outcome = UpdateProtocol.ReadQuiesce(
            DaemonResponse.Parse(
                "{\"id\":1,\"error\":{\"code\":\"busy\",\"message\":\"quiesce-timeout\"}}"));

        Assert.Equal(TcQuiesceOutcome.TimedOut, outcome.Outcome);
        Assert.False(outcome.CanUpdate);
    }

    [Fact]
    public void ABusyDaemonRefusesWithoutTimingOut()
    {
        QuiesceOutcome outcome = UpdateProtocol.ReadQuiesce(
            DaemonResponse.Parse(
                "{\"id\":1,\"error\":{\"code\":\"busy\",\"message\":\"another-quiesce-in-flight\"}}"));

        Assert.Equal(TcQuiesceOutcome.Busy, outcome.Outcome);
        Assert.False(outcome.CanUpdate);
    }

    [Fact]
    public void ADaemonTooOldToKnowTheMethodIsUnsupportedNotBroken()
    {
        QuiesceOutcome outcome = UpdateProtocol.ReadQuiesce(
            DaemonResponse.Parse(
                "{\"id\":1,\"error\":{\"code\":\"unknown_method\",\"message\":\"quiesce\"}}"));

        Assert.Equal(TcQuiesceOutcome.Unsupported, outcome.Outcome);
        Assert.False(outcome.CanUpdate);
    }

    [Fact]
    public void TheSynchronousRefusalIsAlsoUnsupported()
    {
        QuiesceOutcome outcome = UpdateProtocol.ReadQuiesce(
            DaemonResponse.Parse(
                "{\"id\":1,\"error\":{\"code\":\"bad_params\",\"message\":\"quiesce-requires-async\"}}"));

        Assert.Equal(TcQuiesceOutcome.Unsupported, outcome.Outcome);
    }

    [Fact]
    public void AStoppedDaemonIsUnavailable()
    {
        QuiesceOutcome outcome = UpdateProtocol.ReadQuiesce(
            DaemonResponse.Parse(
                "{\"error\":{\"code\":\"unavailable\",\"message\":\"daemon-not-started\"}}"));

        Assert.Equal(TcQuiesceOutcome.Unavailable, outcome.Outcome);
        Assert.False(outcome.CanUpdate);
    }

    [Fact]
    public void AResultFrameThatDoesNotClaimQuiescedIsNotTreatedAsSuccess()
    {
        QuiesceOutcome outcome = UpdateProtocol.ReadQuiesce(
            DaemonResponse.Parse("{\"id\":1,\"result\":{\"quiesced\":false,\"waited_ms\":0}}"));

        Assert.Equal(TcQuiesceOutcome.Unavailable, outcome.Outcome);
        Assert.False(outcome.CanUpdate);
    }

    [Fact]
    public void MalformedJsonRefusesRatherThanThrowing()
    {
        QuiesceOutcome outcome = UpdateProtocol.ReadQuiesce(DaemonResponse.Parse("not json"));

        Assert.Equal(TcQuiesceOutcome.Unavailable, outcome.Outcome);
    }

    [Fact]
    public void EveryRefusalHasAContributorFacingSentence()
    {
        foreach (TcQuiesceOutcome value in new[]
                 {
                     TcQuiesceOutcome.Busy,
                     TcQuiesceOutcome.TimedOut,
                     TcQuiesceOutcome.Unsupported,
                     TcQuiesceOutcome.Unavailable,
                 })
        {
            string text = UpdateProtocol.DescribeRefusal(value);
            Assert.False(string.IsNullOrWhiteSpace(text));
            Assert.EndsWith(".", text);
        }
    }

    [Fact]
    public void TheQuiesceMethodNameMatchesTheDaemonContract()
    {
        Assert.Equal("quiesce", DaemonProtocol.Methods.Quiesce);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run (from the repository root):

```bash
cargo build -p trace-commons-contributor-ffi
dotnet test windows/tests/TraceCommons.Interop.Tests/TraceCommons.Interop.Tests.csproj \
  --filter "FullyQualifiedName~UpdateProtocolTests"
```

Expected: FAIL at compile time with `error CS0246: The type or namespace name 'QuiesceOutcome' could not be found`, `'UpdateProtocol' does not exist`, `'TcQuiesceOutcome' could not be found`, and `error CS0117: 'DaemonProtocol.Methods' does not contain a definition for 'Quiesce'`.

- [ ] **Step 3: Add the method name**

In `windows/src/TraceCommons.Interop/DaemonProtocol.cs`, inside `public static class Methods`, add after the `Shutdown` line:

```csharp
        /// <summary>
        /// Drains in-flight uploads and parks the queue, bounded by
        /// <c>timeout_secs</c>. Answered only by the async dispatcher, which
        /// is what <c>tc_call</c> reaches: it routes through
        /// <c>ipc::handle_local</c> to <c>handle_request_async</c>, so the
        /// socket path's "quiesce-requires-async" refusal never applies here.
        /// </summary>
        public const string Quiesce = "quiesce";
```

- [ ] **Step 4: Write the implementation**

Create `windows/src/TraceCommons.Interop/UpdateProtocol.cs`:

```csharp
using System;
using System.Text.Json.Serialization;

namespace TraceCommons.Interop;

/// <summary>What the daemon said when asked to quiesce.</summary>
public enum TcQuiesceOutcome
{
    /// <summary>Nothing is in flight and the queue is parked. Safe to update.</summary>
    Quiesced,

    /// <summary>The daemon refused for a reason other than a timeout.</summary>
    Busy,

    /// <summary>An upload was still in flight when the timeout expired.</summary>
    TimedOut,

    /// <summary>This daemon does not implement quiesce.</summary>
    Unsupported,

    /// <summary>No daemon answered, or the answer made no sense.</summary>
    Unavailable,
}

/// <summary>
/// The daemon's answer to <c>quiesce</c>, reduced to the one question the
/// caller has: may an update proceed.
/// </summary>
public sealed class QuiesceOutcome
{
    internal QuiesceOutcome(TcQuiesceOutcome outcome, long waitedMs)
    {
        Outcome = outcome;
        WaitedMs = waitedMs;
    }

    public TcQuiesceOutcome Outcome { get; }

    /// <summary>How long the daemon waited for the drain, in milliseconds.</summary>
    public long WaitedMs { get; }

    /// <summary>
    /// True for exactly one outcome. Written as an explicit equality rather
    /// than "not a failure" so that a future enum member defaults to
    /// refusing: an update is not something to fall into.
    /// </summary>
    public bool CanUpdate => Outcome == TcQuiesceOutcome.Quiesced;
}

/// <summary>
/// Reading the daemon's quiesce answer, and turning a refusal into something
/// a contributor can act on.
///
/// Deliberately pure: no native call, no WinRT, no platform. It lives in the
/// interop assembly so its tests run on macOS and Linux with the rest.
/// </summary>
public static class UpdateProtocol
{
    private sealed class QuiescePayload
    {
        [JsonPropertyName("quiesced")]
        public bool Quiesced { get; set; }

        [JsonPropertyName("waited_ms")]
        public long WaitedMs { get; set; }
    }

    /// <summary>
    /// Maps a raw <c>quiesce</c> response onto <see cref="QuiesceOutcome"/>.
    ///
    /// Every shape that is not an explicit <c>quiesced: true</c> maps to a
    /// refusal. That includes a well-formed result frame saying
    /// <c>quiesced: false</c>, which the daemon is not expected to send --
    /// but "unexpected" and "safe to update through" are different claims,
    /// and only one of them is ours to make.
    /// </summary>
    public static QuiesceOutcome ReadQuiesce(DaemonResponse response)
    {
        ArgumentNullException.ThrowIfNull(response);

        if (response.IsError)
        {
            DaemonError error = response.Error!;
            TcQuiesceOutcome outcome = error.Code switch
            {
                "unknown_method" => TcQuiesceOutcome.Unsupported,
                "bad_params" when error.Message == "quiesce-requires-async"
                    => TcQuiesceOutcome.Unsupported,
                "busy" when error.Message == "quiesce-timeout"
                    => TcQuiesceOutcome.TimedOut,
                "busy" => TcQuiesceOutcome.Busy,
                _ => TcQuiesceOutcome.Unavailable,
            };

            return new QuiesceOutcome(outcome, 0);
        }

        QuiescePayload? payload = response.ResultAs<QuiescePayload>();
        if (payload is null || !payload.Quiesced)
        {
            return new QuiesceOutcome(TcQuiesceOutcome.Unavailable, 0);
        }

        return new QuiesceOutcome(TcQuiesceOutcome.Quiesced, payload.WaitedMs);
    }

    /// <summary>
    /// One sentence per refusal, for the update banner.
    ///
    /// Fixed strings with no interpolation of anything the daemon said: this
    /// text goes on screen and into whatever the contributor screenshots, and
    /// the repo's rule is that such surfaces carry labels, never payload.
    /// </summary>
    public static string DescribeRefusal(TcQuiesceOutcome outcome) => outcome switch
    {
        TcQuiesceOutcome.Quiesced =>
            "Ready to update.",
        TcQuiesceOutcome.TimedOut =>
            "An upload is still finishing. The update will install the next time you open the app.",
        TcQuiesceOutcome.Busy =>
            "The daemon is busy. The update will install the next time you open the app.",
        TcQuiesceOutcome.Unsupported =>
            "This version cannot pause uploads to update safely. Windows will install the update the next time you open the app.",
        _ =>
            "The daemon is not available. The update will install the next time you open the app.",
    };
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run:

```bash
dotnet test windows/tests/TraceCommons.Interop.Tests/TraceCommons.Interop.Tests.csproj \
  --filter "FullyQualifiedName~UpdateProtocolTests"
```

Expected: `Passed!  - Failed: 0, Passed: 10`.

- [ ] **Step 6: Run the whole interop suite to confirm nothing regressed**

Run:

```bash
dotnet test windows/tests/TraceCommons.Interop.Tests/TraceCommons.Interop.Tests.csproj
```

Expected: `Failed: 0` with the total at 33 (the existing 23 plus these 10).

- [ ] **Step 7: Commit**

```bash
git add windows/src/TraceCommons.Interop/UpdateProtocol.cs \
        windows/src/TraceCommons.Interop/DaemonProtocol.cs \
        windows/tests/TraceCommons.Interop.Tests/UpdateProtocolTests.cs
git commit -m "Read the daemon quiesce answer as an explicit update gate"
```

---

### Task 6: Model update availability without WinRT

**Files:**
- Modify: `windows/src/TraceCommons.Interop/UpdateProtocol.cs`
- Modify: `windows/tests/TraceCommons.Interop.Tests/UpdateProtocolTests.cs`

**Interfaces:**
- Consumes: `UpdateProtocol` from Task 5.
- Produces, in namespace `TraceCommons.Interop`:
  - `public enum TcUpdateAvailability { Unknown = 0, NoUpdates = 1, Available = 2, Required = 3, Error = 4 }`
  - `public static bool UpdateProtocol.ShouldOfferUpdate(TcUpdateAvailability availability)`
  - `public static string UpdateProtocol.DescribeAvailability(TcUpdateAvailability availability)`

`TcUpdateAvailability`'s members and numeric values mirror WinRT's `Windows.ApplicationModel.PackageUpdateAvailability` exactly (`Unknown` 0, `NoUpdates` 1, `Available` 2, `Required` 3, `Error` 4). It exists as a separate type because the interop assembly targets plain `net8.0` and must never reference a `Windows.*` type; Task 7 maps the WinRT enum onto this one with an explicit switch rather than a numeric cast, so a future WinRT member cannot silently arrive as a wrong value.

- [ ] **Step 1: Write the failing tests**

Append to `windows/tests/TraceCommons.Interop.Tests/UpdateProtocolTests.cs`, inside the `UpdateProtocolTests` class:

```csharp
    [Theory]
    [InlineData(TcUpdateAvailability.Available, true)]
    [InlineData(TcUpdateAvailability.Required, true)]
    [InlineData(TcUpdateAvailability.NoUpdates, false)]
    [InlineData(TcUpdateAvailability.Unknown, false)]
    [InlineData(TcUpdateAvailability.Error, false)]
    public void OnlyARealOfferPutsTheBannerOnScreen(
        TcUpdateAvailability availability, bool expected)
    {
        Assert.Equal(expected, UpdateProtocol.ShouldOfferUpdate(availability));
    }

    [Fact]
    public void UnknownIsNotAnErrorMessage()
    {
        // Unknown is what a build with no .appinstaller association reports,
        // which is a normal state for a locally built app rather than a
        // fault worth alarming a contributor about.
        Assert.Equal(
            "Updates are not managed for this installation.",
            UpdateProtocol.DescribeAvailability(TcUpdateAvailability.Unknown));
    }

    [Fact]
    public void EveryAvailabilityHasASentence()
    {
        foreach (TcUpdateAvailability value in new[]
                 {
                     TcUpdateAvailability.Unknown,
                     TcUpdateAvailability.NoUpdates,
                     TcUpdateAvailability.Available,
                     TcUpdateAvailability.Required,
                     TcUpdateAvailability.Error,
                 })
        {
            string text = UpdateProtocol.DescribeAvailability(value);
            Assert.False(string.IsNullOrWhiteSpace(text));
            Assert.EndsWith(".", text);
        }
    }

    [Fact]
    public void TheEnumMirrorsTheWinRtNumbering()
    {
        // Task 7 maps WinRT's PackageUpdateAvailability onto this enum with
        // an explicit switch, but the numbering is asserted here so the two
        // cannot quietly disagree if anyone reaches for a cast.
        Assert.Equal(0, (int)TcUpdateAvailability.Unknown);
        Assert.Equal(1, (int)TcUpdateAvailability.NoUpdates);
        Assert.Equal(2, (int)TcUpdateAvailability.Available);
        Assert.Equal(3, (int)TcUpdateAvailability.Required);
        Assert.Equal(4, (int)TcUpdateAvailability.Error);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run:

```bash
dotnet test windows/tests/TraceCommons.Interop.Tests/TraceCommons.Interop.Tests.csproj \
  --filter "FullyQualifiedName~UpdateProtocolTests"
```

Expected: FAIL at compile time with `error CS0246: The type or namespace name 'TcUpdateAvailability' could not be found`.

- [ ] **Step 3: Write the implementation**

In `windows/src/TraceCommons.Interop/UpdateProtocol.cs`, add this enum immediately after the `TcQuiesceOutcome` enum:

```csharp
/// <summary>
/// Whether an update is waiting, mirroring WinRT's
/// <c>Windows.ApplicationModel.PackageUpdateAvailability</c> member for
/// member and value for value.
///
/// A separate type because this assembly targets plain net8.0 and must never
/// name a Windows type -- that is what keeps its tests running on macOS and
/// Linux against the same Rust crate.
/// </summary>
public enum TcUpdateAvailability
{
    /// <summary>The package has no App Installer association.</summary>
    Unknown = 0,

    /// <summary>Up to date.</summary>
    NoUpdates = 1,

    /// <summary>An update is waiting and is optional.</summary>
    Available = 2,

    /// <summary>An update is waiting and the feed marks it required.</summary>
    Required = 3,

    /// <summary>The check itself failed.</summary>
    Error = 4,
}
```

and add these two methods inside `public static class UpdateProtocol`, after `DescribeRefusal`:

```csharp
    /// <summary>
    /// Whether to put the update banner on screen.
    ///
    /// An allow-list of two, not a deny-list. <c>Unknown</c> and
    /// <c>Error</c> both mean the check told us nothing, and offering an
    /// update we cannot confirm exists is how a contributor ends up
    /// restarting an app for no reason.
    /// </summary>
    public static bool ShouldOfferUpdate(TcUpdateAvailability availability) =>
        availability == TcUpdateAvailability.Available
        || availability == TcUpdateAvailability.Required;

    /// <summary>
    /// One sentence per availability, for the banner and the status line.
    /// Fixed strings only, per the repo's label-only rule for anything that
    /// reaches a screen or a log.
    /// </summary>
    public static string DescribeAvailability(TcUpdateAvailability availability) =>
        availability switch
        {
            TcUpdateAvailability.Available =>
                "A newer version is ready to install.",
            TcUpdateAvailability.Required =>
                "A required update is ready to install.",
            TcUpdateAvailability.NoUpdates =>
                "Trace Commons is up to date.",
            TcUpdateAvailability.Unknown =>
                "Updates are not managed for this installation.",
            _ =>
                "The update check did not complete.",
        };
```

- [ ] **Step 4: Run the tests to verify they pass**

Run:

```bash
dotnet test windows/tests/TraceCommons.Interop.Tests/TraceCommons.Interop.Tests.csproj \
  --filter "FullyQualifiedName~UpdateProtocolTests"
```

Expected: `Passed!  - Failed: 0, Passed: 18` (10 from Task 5, plus 5 theory cases and 3 facts here).

- [ ] **Step 5: Commit**

```bash
git add windows/src/TraceCommons.Interop/UpdateProtocol.cs \
        windows/tests/TraceCommons.Interop.Tests/UpdateProtocolTests.cs
git commit -m "Model update availability in the interop layer without naming WinRT"
```

---

### Task 7: The app's update service and the banner

**Files:**
- Create: `windows/src/TraceCommons.App/AppUpdater.cs`
- Modify: `windows/src/TraceCommons.App/ViewModels/MainViewModel.cs`
- Modify: `windows/src/TraceCommons.App/MainWindow.xaml`
- Modify: `windows/src/TraceCommons.App/MainWindow.xaml.cs`

**Interfaces:**
- Consumes: `DaemonHost.CallAsync`, `DaemonHost.DisposeAsync`, `DaemonProtocol.Methods.Quiesce`, `UpdateProtocol.ReadQuiesce`, `UpdateProtocol.DescribeRefusal`, `UpdateProtocol.ShouldOfferUpdate`, `UpdateProtocol.DescribeAvailability`, `TcUpdateAvailability`, `QuiesceOutcome`.
- Produces:
  - `public sealed class AppUpdater` with:
    - `public static Uri FeedUri { get; }`
    - `public AppUpdater(DaemonHost host)`
    - `public Task<TcUpdateAvailability> CheckAsync()`
    - `public Task<QuiesceOutcome> QuiesceAsync(int timeoutSeconds = 60)`
    - `public Task<bool> ApplyAsync()`
  - `MainViewModel` gains `public MainViewModel(DaemonHost host, AppUpdater? updater = null)`, `public bool IsUpdateBannerVisible { get; }`, `public bool IsUpdateApplyEnabled { get; }`, `public string UpdateStatusText { get; }`, `public Task CheckForUpdateAsync()`, `public Task ApplyUpdateAsync()`.

**Why the app asks rather than replaces.** On Windows desktop the platform installer owns the bytes, exactly as Homebrew, flatpak and winget do on the other paths. `RequestAddPackageByAppInstallerFileAsync` hands the whole operation to the deployment service, which shuts this process down and brings the updated one back. Nothing here writes to its own install directory. The one thing the app does own is the moment: it will not hand off while an upload is on the wire.

**Why not `ms-appinstaller:`.** Microsoft disabled the `ms-appinstaller` protocol handler by default after it was abused as a malware delivery vector. A design that launches a URI scheme the platform turns off is a design that silently stops working. The `PackageManager` API is the supported path, and the `packageManagement` restricted capability declared in Task 1 is its price.

- [ ] **Step 1: Write the updater**

Create `windows/src/TraceCommons.App/AppUpdater.cs`:

```csharp
using System;
using System.Globalization;
using System.Runtime.InteropServices;
using System.Threading.Tasks;
using TraceCommons.Interop;
using Windows.ApplicationModel;
using Windows.Management.Deployment;

namespace TraceCommons.App;

/// <summary>
/// The app's half of the MSIX update flow.
///
/// The governing rule is that whoever installed the binary owns replacing it,
/// and on Windows desktop that is the deployment service. This class never
/// touches the install directory: it asks whether an update exists, it makes
/// sure nothing is mid-upload, and it hands off. App Installer performs the
/// swap and restarts the app.
///
/// Every call here needs package identity, which is why the project is
/// packaged. An unpackaged build reports
/// <see cref="TcUpdateAvailability.Unknown"/> rather than throwing, so a
/// developer running an unpackaged build sees "updates are not managed for
/// this installation" instead of a crash.
/// </summary>
public sealed class AppUpdater
{
    private readonly DaemonHost _host;

    public AppUpdater(DaemonHost host)
    {
        _host = host ?? throw new ArgumentNullException(nameof(host));
    }

    /// <summary>
    /// The feed the deployment service polls and this class hands back to.
    ///
    /// Hard-coded rather than configurable. A configurable update source is
    /// a configurable place to be handed a different app, and the signature
    /// check that would have to defend it happens inside Windows, against
    /// whatever URI it is given.
    /// </summary>
    public static Uri FeedUri { get; } =
        new Uri("https://storage.googleapis.com/tracecommons-flatpak/windows/TraceCommons.appinstaller");

    /// <summary>
    /// Asks the deployment service whether the feed offers something newer.
    ///
    /// The package is looked up through <see cref="PackageManager"/> rather
    /// than used straight off <c>Package.Current</c>: calling
    /// <c>CheckUpdateAvailabilityAsync</c> on the object
    /// <c>Package.Current</c> returns fails with access denied, which is a
    /// documented known issue and not something to discover at runtime.
    /// </summary>
    public async Task<TcUpdateAvailability> CheckAsync()
    {
        try
        {
            var manager = new PackageManager();
            Package current = manager.FindPackageForUser(
                string.Empty, Package.Current.Id.FullName);

            PackageUpdateAvailabilityResult result =
                await current.CheckUpdateAvailabilityAsync().AsTask().ConfigureAwait(true);

            return result.Availability switch
            {
                PackageUpdateAvailability.Available => TcUpdateAvailability.Available,
                PackageUpdateAvailability.Required => TcUpdateAvailability.Required,
                PackageUpdateAvailability.NoUpdates => TcUpdateAvailability.NoUpdates,
                PackageUpdateAvailability.Unknown => TcUpdateAvailability.Unknown,
                _ => TcUpdateAvailability.Error,
            };
        }
        catch (Exception ex) when (
            ex is InvalidOperationException      // no package identity
            or UnauthorizedAccessException
            or COMException)
        {
            // Deliberately not logging the exception. Its message can carry
            // a package full name and a path, and this is the one class that
            // sits between the deployment service and a log file.
            return TcUpdateAvailability.Unknown;
        }
    }

    /// <summary>
    /// Asks the in-process daemon to drain in-flight uploads and park the
    /// queue, bounded by <paramref name="timeoutSeconds"/>.
    ///
    /// The daemon is hosted IN THIS PROCESS, so this is not the CLI's
    /// separate-process problem: the update terminates this process and takes
    /// the daemon with it. That makes the drain the whole safety property.
    /// A refusal is honoured -- the app does not hand off, and the scheduled
    /// OnLaunch check installs the update at a calmer moment instead.
    /// </summary>
    public async Task<QuiesceOutcome> QuiesceAsync(int timeoutSeconds = 60)
    {
        string paramsJson = string.Format(
            CultureInfo.InvariantCulture,
            "{{\"timeout_secs\":{0}}}",
            timeoutSeconds);

        DaemonResponse response = await _host
            .CallAsync(DaemonProtocol.Methods.Quiesce, paramsJson)
            .ConfigureAwait(true);

        return UpdateProtocol.ReadQuiesce(response);
    }

    /// <summary>
    /// Hands the update to the deployment service.
    ///
    /// <c>ForceTargetAppShutdown</c> because the package being replaced is
    /// the one running this code; without it registration cannot proceed
    /// past a live process. The caller must therefore have quiesced and torn
    /// the daemon down first, because control does not reliably come back:
    /// on the success path this process is terminated part-way through the
    /// await.
    ///
    /// Returns false rather than throwing when the request is refused, so a
    /// contributor gets a sentence instead of a crash. The commonest refusal
    /// is a policy that blocks non-Store deployment, and there is nothing the
    /// app can do about it except say so.
    /// </summary>
    public async Task<bool> ApplyAsync()
    {
        try
        {
            var manager = new PackageManager();
            DeploymentResult result = await manager
                .RequestAddPackageByAppInstallerFileAsync(
                    FeedUri,
                    AddPackageByAppInstallerOptions.ForceTargetAppShutdown,
                    null!)
                .AsTask()
                .ConfigureAwait(true);

            return result.ExtendedErrorCode is null;
        }
        catch (Exception ex) when (
            ex is UnauthorizedAccessException or COMException or InvalidOperationException)
        {
            return false;
        }
    }
}
```

- [ ] **Step 2: Build the app to confirm the WinRT projections resolve**

Run (from `windows/`, on Windows):

```powershell
msbuild src\TraceCommons.App\TraceCommons.App.csproj -restore `
  -p:Configuration=Release -p:Platform=x64
```

Expected: `Build succeeded.` with `0 Warning(s)` and `0 Error(s)`. A failure naming `PackageManager` or `AsTask` means the target framework moniker is wrong; it must be `net8.0-windows10.0.19041.0`.

- [ ] **Step 3: Wire the view model**

In `windows/src/TraceCommons.App/ViewModels/MainViewModel.cs`, add `using TraceCommons.App;` is not needed (same assembly, and `AppUpdater` is in namespace `TraceCommons.App` while this file is in `TraceCommons.App.ViewModels`) — instead add this using with the others at the top:

```csharp
using TraceCommons.App;
```

Replace the fields block:

```csharp
    private readonly DaemonHost _host;
    private readonly SemaphoreSlim _refreshGate = new(1, 1);
    private string _statusText = "Starting…";
    private bool _isBusy;
```

with:

```csharp
    private readonly DaemonHost _host;
    private readonly AppUpdater? _updater;
    private readonly SemaphoreSlim _refreshGate = new(1, 1);
    private string _statusText = "Starting…";
    private string _updateStatusText = string.Empty;
    private bool _isBusy;
    private bool _isUpdateBannerVisible;
    private bool _isUpdateApplyEnabled;
```

Replace the constructor:

```csharp
    public MainViewModel(DaemonHost host)
    {
        _host = host ?? throw new ArgumentNullException(nameof(host));
        _host.QueueChanged += OnQueueChanged;
        _host.StatusChanged += OnStatusChanged;
        _host.Lagged += OnLagged;
    }
```

with:

```csharp
    /// <summary>
    /// <paramref name="updater"/> is optional so the view model stays
    /// constructible without package identity. An unpackaged developer build
    /// then simply never shows the banner, rather than throwing at launch.
    /// </summary>
    public MainViewModel(DaemonHost host, AppUpdater? updater = null)
    {
        _host = host ?? throw new ArgumentNullException(nameof(host));
        _updater = updater;
        _host.QueueChanged += OnQueueChanged;
        _host.StatusChanged += OnStatusChanged;
        _host.Lagged += OnLagged;
    }
```

Add these properties immediately after the `IsEmpty` property:

```csharp
    /// <summary>
    /// Whether the update banner is on screen. Only ever true for a
    /// confirmed offer -- see <c>UpdateProtocol.ShouldOfferUpdate</c>.
    /// </summary>
    public bool IsUpdateBannerVisible
    {
        get => _isUpdateBannerVisible;
        private set => Set(ref _isUpdateBannerVisible, value);
    }

    /// <summary>
    /// Whether the banner's action button is live. Goes false for the
    /// duration of an apply so a second click cannot start a second
    /// handoff.
    /// </summary>
    public bool IsUpdateApplyEnabled
    {
        get => _isUpdateApplyEnabled;
        private set => Set(ref _isUpdateApplyEnabled, value);
    }

    /// <summary>
    /// The banner's message. Fixed labels only, from
    /// <c>UpdateProtocol</c> -- nothing the deployment service or the daemon
    /// said reaches this string.
    /// </summary>
    public string UpdateStatusText
    {
        get => _updateStatusText;
        private set => Set(ref _updateStatusText, value);
    }
```

Add these two methods immediately after `RefreshAsync`:

```csharp
    /// <summary>
    /// Asks the deployment service whether the feed offers something newer,
    /// and raises the banner if it does.
    ///
    /// Never surfaces a failed check. Windows checks the feed on its own
    /// schedule regardless of what this call returns, so a check that could
    /// not complete costs a contributor nothing and telling them about it
    /// buys nothing either.
    /// </summary>
    public async Task CheckForUpdateAsync()
    {
        if (_updater is null)
        {
            return;
        }

        TcUpdateAvailability availability = await _updater.CheckAsync().ConfigureAwait(true);
        if (!UpdateProtocol.ShouldOfferUpdate(availability))
        {
            IsUpdateBannerVisible = false;
            return;
        }

        UpdateStatusText = UpdateProtocol.DescribeAvailability(availability);
        IsUpdateApplyEnabled = true;
        IsUpdateBannerVisible = true;
    }

    /// <summary>
    /// Drains, tears the daemon down, and hands the update to Windows.
    ///
    /// The order is the whole point. Quiesce first, because App Installer
    /// terminates this process and a half-uploaded trace must never be the
    /// cost of an update. Then dispose the host, so the C ABI's ordered
    /// teardown runs while there is still a process to run it in. Only then
    /// hand off -- and on the success path control does not return from that
    /// call, because the process is gone.
    /// </summary>
    public async Task ApplyUpdateAsync()
    {
        if (_updater is null)
        {
            return;
        }

        IsUpdateApplyEnabled = false;
        UpdateStatusText = "Finishing any upload in progress…";

        QuiesceOutcome quiesce = await _updater.QuiesceAsync().ConfigureAwait(true);
        if (!quiesce.CanUpdate)
        {
            UpdateStatusText = UpdateProtocol.DescribeRefusal(quiesce.Outcome);
            return;
        }

        UpdateStatusText = "Installing the update…";
        await _host.DisposeAsync().ConfigureAwait(true);

        bool handedOff = await _updater.ApplyAsync().ConfigureAwait(true);
        if (!handedOff)
        {
            UpdateStatusText =
                "The update could not be installed. Windows will try again on its own schedule.";
        }
    }
```

- [ ] **Step 4: Add the banner to the window**

In `windows/src/TraceCommons.App/MainWindow.xaml`, change the root grid's row definitions:

```xml
    <Grid RowDefinitions="Auto,*,Auto" Padding="0">
```

to:

```xml
    <Grid RowDefinitions="Auto,Auto,*,Auto" Padding="0">
```

Then renumber every existing `Grid.Row` in that grid, in this order:
- the header `<Grid Grid.Row="0" ... ColumnDefinitions="*,Auto"` stays `Grid.Row="0"`.
- the `<ListView Grid.Row="1"` becomes `Grid.Row="2"`.
- the empty-state `<StackPanel Grid.Row="1"` becomes `Grid.Row="2"`.
- the `<ProgressBar Grid.Row="2"` becomes `Grid.Row="3"`.

Then insert this element immediately after the closing `</Grid>` of the header block and before the `<ListView>`:

```xml
        <!-- The update offer.

             An InfoBar rather than a dialog: an available update is not an
             interruption, and a contributor who came here to approve a
             session should be able to finish doing that. IsClosable is false
             because the banner is already conditional on a real offer, and a
             dismissed banner is an update that quietly never happens. -->
        <InfoBar Grid.Row="1"
                 IsOpen="{x:Bind ViewModel.IsUpdateBannerVisible, Mode=OneWay}"
                 IsClosable="False"
                 Severity="Informational"
                 Title="Update available"
                 Message="{x:Bind ViewModel.UpdateStatusText, Mode=OneWay}">
            <InfoBar.ActionButton>
                <Button Content="Update and restart"
                        Click="OnApplyUpdateClick"
                        IsEnabled="{x:Bind ViewModel.IsUpdateApplyEnabled, Mode=OneWay}"
                        AutomationProperties.Name="Install the update and restart Trace Commons" />
            </InfoBar.ActionButton>
        </InfoBar>
```

- [ ] **Step 5: Wire the window's code-behind**

In `windows/src/TraceCommons.App/MainWindow.xaml.cs`, replace:

```csharp
        _host = new DaemonHost(Microsoft.UI.Dispatching.DispatcherQueue.GetForCurrentThread());
        ViewModel = new MainViewModel(_host);
```

with:

```csharp
        _host = new DaemonHost(Microsoft.UI.Dispatching.DispatcherQueue.GetForCurrentThread());
        ViewModel = new MainViewModel(_host, new AppUpdater(_host));
```

Replace the `OnFirstActivated` body:

```csharp
        Activated -= OnFirstActivated;
        await ViewModel.InitializeAsync();
```

with:

```csharp
        Activated -= OnFirstActivated;
        await ViewModel.InitializeAsync();

        // After the queue is on screen, not before. The update check is a
        // network round trip through the deployment service and nothing
        // about it should stand between a contributor and the sessions they
        // opened the app to review.
        await ViewModel.CheckForUpdateAsync();
```

Add this handler immediately after `OnRefreshClick`:

```csharp
    /// <summary>
    /// Hands the update to Windows. Fire-and-forget in the same sense
    /// OnClosed is: the click handler cannot be awaited, and on the success
    /// path this process is terminated part-way through the call anyway.
    /// </summary>
    private async void OnApplyUpdateClick(object sender, RoutedEventArgs e)
    {
        await ViewModel.ApplyUpdateAsync();
    }
```

- [ ] **Step 6: Build and confirm zero warnings**

Run (from `windows/`):

```powershell
msbuild src\TraceCommons.App\TraceCommons.App.csproj -restore `
  -p:Configuration=Release -p:Platform=x64
```

Expected: `Build succeeded.` with `0 Warning(s)` and `0 Error(s)`. `TreatWarningsAsErrors` is on, so an unused field or a nullable mismatch fails here rather than in CI.

- [ ] **Step 7: Confirm the interop suite still passes**

Run (from the repository root):

```bash
dotnet test windows/tests/TraceCommons.Interop.Tests/TraceCommons.Interop.Tests.csproj
```

Expected: `Failed: 0`, total 41 (the original 23 plus the 10 from Task 5 and the 8 from Task 6).

- [ ] **Step 8: Commit**

```bash
git add windows/src/TraceCommons.App/AppUpdater.cs \
        windows/src/TraceCommons.App/ViewModels/MainViewModel.cs \
        windows/src/TraceCommons.App/MainWindow.xaml \
        windows/src/TraceCommons.App/MainWindow.xaml.cs
git commit -m "Offer the waiting update in the window and drain uploads before applying it"
```

---

### Task 8: Build, sign, install-verify and publish the MSIX

**Files:**
- Modify: `.github/workflows/release-apps.yml` (add a `windows-app` job; add it to the `publish` job's `needs` and gate)
- Modify: `.github/workflows/ci.yml` (extend the `windows-contributor-app` job)
- Modify: `windows/README.md`

**Interfaces:**
- Consumes: `windows/scripts/make-app-icons.ps1` and the manifest from Task 1, `windows/scripts/stamp-package-identity.ps1` from Task 2, `scripts/windows/setup-trusted-signing.ps1` and `scripts/windows/sign-with-trusted-signing.ps1` from Task 3, `windows/scripts/make-appinstaller.ps1` from Task 4.
- Produces: `gs://tracecommons-flatpak/windows/TraceCommons.appinstaller` and `gs://tracecommons-flatpak/windows/ai.tracecommons.Contributor_<quad>_x64.msix`, plus a `windows-msix` build artifact.

- [ ] **Step 1: Add the release job**

In `.github/workflows/release-apps.yml`, insert this job immediately after the `windows` job and before `linux-flatpak`:

```yaml
  windows-app:
    name: Windows signed MSIX
    needs: version
    if: >-
      github.event_name == 'push' ||
      inputs.platform == 'all' || inputs.platform == 'windows'
    runs-on: windows-latest
    # Same reason as the `windows` job above: the federated credential's
    # subject is repo:...:environment:release, so this job must declare the
    # environment or the OIDC token will not match and signing fails at auth.
    environment: release
    timeout-minutes: 60
    steps:
      - uses: actions/checkout@d23441a48e516b6c34aea4fa41551a30e30af803 # v6
      - uses: dtolnay/rust-toolchain@3c5f7ea28cd621ae0bf5283f0e981fb97b8a7af9 # master@2026-05-27
        with:
          toolchain: "1.92"
          targets: x86_64-pc-windows-msvc
      # windows/global.json pins the SDK to 8.0.x. MSBuild otherwise selects
      # the newest installed SDK regardless of what setup-dotnet put there,
      # and the runner image ships .NET 10, whose layout Windows App SDK 1.6
      # cannot build against.
      - uses: actions/setup-dotnet@v4
        with:
          dotnet-version: '8.0.x'
      # MSBuild from Visual Studio, not `dotnet build`. A WinUI 3 project
      # invokes tasks from Microsoft.Build.AppxPackage.dll and
      # Microsoft.Build.Packaging.Pri.Tasks.dll, which are never part of the
      # .NET SDK.
      - uses: microsoft/setup-msbuild@v2
      # No build cache in a release job, deliberately. A cache is a
      # write-target a lower-privilege job can poison, and this job holds
      # signing authority.

      - name: Build the contributor FFI cdylib
        run: cargo build -p trace-commons-contributor-ffi --release

      # Compile first, package later. The publisher the package must carry is
      # not known until a certificate has actually signed something, so the
      # manifest cannot be finalized before this point.
      - name: Build the app
        working-directory: windows
        env:
          TC_FFI_LIB_DIR: ${{ github.workspace }}\target\release
        run: msbuild src\TraceCommons.App\TraceCommons.App.csproj -restore -p:Configuration=Release -p:Platform=x64

      - uses: azure/login@a457da9ea143d694b1b9c7c869ebb04ebe844ef5 # v2
        with:
          client-id: ${{ vars.AZURE_SIGNING_CLIENT_ID }}
          tenant-id: ${{ vars.AZURE_SIGNING_TENANT_ID }}
          subscription-id: ${{ vars.AZURE_SIGNING_SUBSCRIPTION_ID }}

      - name: Set up Trusted Signing (SHA-verified dlib + signtool)
        id: ts
        shell: pwsh
        env:
          TS_ENDPOINT: ${{ vars.AZURE_SIGNING_ENDPOINT }}
          TS_ACCOUNT: ${{ vars.AZURE_SIGNING_ACCOUNT }}
          TS_PROFILE: ${{ vars.AZURE_SIGNING_PROFILE }}
          # Independently verified 2026-08-16. When bumping the version,
          # re-derive this from a trusted machine.
          TRUSTED_SIGNING_CLIENT_SHA256: 3BFCF1E0A3CB42AF1692F0A8ED45C15DE070C2DE86F28A59B2795D904D8A920F
        run: |
          ./scripts/windows/setup-trusted-signing.ps1 `
            -Endpoint $env:TS_ENDPOINT `
            -Account $env:TS_ACCOUNT `
            -Profile $env:TS_PROFILE `
            -ExpectedSha256 $env:TRUSTED_SIGNING_CLIENT_SHA256

      # Identity/@Publisher must match the signing certificate's subject
      # EXACTLY or signtool refuses the package. Rather than configure that
      # subject and let it drift the day the profile is re-issued, sign a
      # throwaway copy of the app's own executable and read back the subject
      # Windows itself reports. The value cannot be wrong, because it is the
      # value the real signature will carry.
      - name: Discover the certificate subject
        id: subject
        shell: pwsh
        env:
          TS_SIGNTOOL: ${{ steps.ts.outputs.signtool }}
          TS_DLIB: ${{ steps.ts.outputs.dlib }}
          TS_METADATA: ${{ steps.ts.outputs.metadata }}
        run: |
          $ErrorActionPreference = "Stop"
          $exe = @(Get-ChildItem -Recurse -Filter TraceCommons.exe `
                     -Path windows\src\TraceCommons.App\bin\x64\Release)
          if ($exe.Count -lt 1) { throw "TraceCommons.exe was not found in the build output" }
          New-Item -ItemType Directory -Force -Path "$env:RUNNER_TEMP\probe" | Out-Null
          $probe = Join-Path "$env:RUNNER_TEMP\probe" "subject-probe.exe"
          Copy-Item $exe[0].FullName $probe -Force
          ./scripts/windows/sign-with-trusted-signing.ps1 `
            -SignTool $env:TS_SIGNTOOL -Dlib $env:TS_DLIB `
            -Metadata $env:TS_METADATA -Path @($probe)
          $subject = (Get-AuthenticodeSignature $probe).SignerCertificate.Subject
          if ([string]::IsNullOrWhiteSpace($subject)) {
            throw "could not read the certificate subject back from the probe signature"
          }
          Write-Host "certificate subject: $subject"
          "subject=$subject" >> $env:GITHUB_OUTPUT
          Remove-Item $probe -Force

      - name: Stamp the package identity
        id: identity
        shell: pwsh
        env:
          SHORT_VERSION: ${{ needs.version.outputs.short }}
          CERT_SUBJECT: ${{ steps.subject.outputs.subject }}
        run: |
          $ErrorActionPreference = "Stop"
          $quad = & windows\scripts\stamp-package-identity.ps1 `
            -ManifestPath windows\src\TraceCommons.App\Package.appxmanifest `
            -Version $env:SHORT_VERSION -Publisher $env:CERT_SUBJECT
          "quad=$quad" >> $env:GITHUB_OUTPUT

      - name: Package
        working-directory: windows
        env:
          TC_FFI_LIB_DIR: ${{ github.workspace }}\target\release
        run: msbuild src\TraceCommons.App\TraceCommons.App.csproj -p:Configuration=Release -p:Platform=x64 -p:GenerateAppxPackageOnBuild=true

      # Exactly one, asserted rather than assumed -- the same shape as the
      # CLI job's staging check. A second .msix in the output directory means
      # a stale build is about to be signed and published as this release.
      - name: Locate the package
        id: pkg
        shell: pwsh
        run: |
          $ErrorActionPreference = "Stop"
          $found = @(Get-ChildItem -Recurse -Filter *.msix -Path windows\dist\msix)
          if ($found.Count -ne 1) {
            throw "expected exactly 1 .msix, found $($found.Count): $($found.Name -join ', ')"
          }
          "path=$($found[0].FullName)" >> $env:GITHUB_OUTPUT
          "name=$($found[0].Name)" >> $env:GITHUB_OUTPUT

      - name: Sign and verify
        shell: pwsh
        env:
          TS_SIGNTOOL: ${{ steps.ts.outputs.signtool }}
          TS_DLIB: ${{ steps.ts.outputs.dlib }}
          TS_METADATA: ${{ steps.ts.outputs.metadata }}
          PKG: ${{ steps.pkg.outputs.path }}
        run: |
          ./scripts/windows/sign-with-trusted-signing.ps1 `
            -SignTool $env:TS_SIGNTOOL -Dlib $env:TS_DLIB `
            -Metadata $env:TS_METADATA -Path @($env:PKG)

      # Installing is the only real validation of a manifest. A schema that
      # parses and a package Windows will register are different claims, and
      # this job holds the second one BEFORE anything is published.
      - name: Install it on the runner
        shell: pwsh
        env:
          PKG: ${{ steps.pkg.outputs.path }}
          QUAD: ${{ steps.identity.outputs.quad }}
        run: |
          $ErrorActionPreference = "Stop"
          Add-AppxPackage -Path $env:PKG
          $installed = Get-AppxPackage -Name ai.tracecommons.Contributor
          if (-not $installed) { throw "the package did not register" }
          if ($installed.Version -ne $env:QUAD) {
            throw "installed version $($installed.Version) does not match the stamped $env:QUAD"
          }
          Write-Host "registered $($installed.PackageFullName)"

      # The virtualization check. Files a packaged desktop app creates under
      # %LOCALAPPDATA% are redirected to a per-package private location
      # unless write virtualization is disabled, and under redirection this
      # app would keep a SECOND queue the contributor CLI cannot see. The
      # manifest disables it; this proves the manifest worked, inside the
      # real container, on a real installed package.
      - name: Confirm the state directory is not virtualized
        shell: pwsh
        run: |
          $ErrorActionPreference = "Stop"
          $pkg = Get-AppxPackage -Name ai.tracecommons.Contributor
          $dir = Join-Path $env:LOCALAPPDATA 'trace-commons'
          $probe = Join-Path $dir 'msix-virtualization-probe.txt'
          if (Test-Path $probe) { Remove-Item $probe -Force }
          $inner = "New-Item -ItemType Directory -Force -Path '$dir' | Out-Null; Set-Content -Path '$probe' -Value ok"
          Invoke-CommandInDesktopPackage -PackageFamilyName $pkg.PackageFamilyName `
            -AppId TraceCommons `
            -Command "$env:SystemRoot\System32\WindowsPowerShell\v1.0\powershell.exe" `
            -Args "-NoProfile -NonInteractive -Command `"$inner`""
          $deadline = (Get-Date).AddSeconds(60)
          while (-not (Test-Path $probe) -and (Get-Date) -lt $deadline) {
            Start-Sleep -Seconds 2
          }
          if (-not (Test-Path $probe)) {
            throw "the packaged app's write to $dir did not land at the real path. Write virtualization is still on, which means this app would keep a second queue the contributor CLI cannot see."
          }
          Remove-Item $probe -Force
          Remove-AppxPackage -Package $pkg.PackageFullName

      - name: Generate the appinstaller feed
        shell: pwsh
        env:
          QUAD: ${{ steps.identity.outputs.quad }}
          CERT_SUBJECT: ${{ steps.subject.outputs.subject }}
          PKG_NAME: ${{ steps.pkg.outputs.name }}
        run: |
          $ErrorActionPreference = "Stop"
          New-Item -ItemType Directory -Force -Path dist | Out-Null
          Copy-Item "${{ steps.pkg.outputs.path }}" "dist\$env:PKG_NAME" -Force
          ./windows/scripts/make-appinstaller.ps1 `
            -PackageName ai.tracecommons.Contributor `
            -Publisher $env:CERT_SUBJECT `
            -Version $env:QUAD `
            -ProcessorArchitecture x64 `
            -BaseUri https://storage.googleapis.com/tracecommons-flatpak/windows `
            -PackageFileName $env:PKG_NAME `
            -OutputPath dist\TraceCommons.appinstaller
          $hash = (Get-FileHash "dist\$env:PKG_NAME" -Algorithm SHA256).Hash.ToLowerInvariant()
          "$hash  $env:PKG_NAME" | Out-File "dist\$env:PKG_NAME.sha256" -Encoding ascii -NoNewline

      # After signing, so provenance covers the signed bytes rather than the
      # unsigned build output.
      - uses: actions/attest-build-provenance@a2bbfa25375fe432b6a289bc6b6cd05ecd0c4c32 # v4
        with:
          subject-path: dist/*.msix

      - uses: actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a # v7
        with:
          name: windows-msix
          path: dist/

      # Publication is tag pushes only. A dispatch run still builds, signs,
      # installs and verifies above, so the whole path is provable -- but it
      # must never move what an installed client would actually pull.
      - name: Authenticate to GCP (workload identity federation; no key on the runner)
        if: github.event_name == 'push'
        uses: google-github-actions/auth@7c6bc770dae815cd3e89ee6cdf493a5fab2cc093 # v3
        with:
          workload_identity_provider: ${{ secrets.GCP_WIF_PROVIDER }}
          service_account: ${{ secrets.GCP_FLATPAK_PUBLISHER_SA }}

      - uses: google-github-actions/setup-gcloud@aa5489c8933f4cc7a4f7d45035b3b1440c9c10db # v3.0.1
        if: github.event_name == 'push'

      # The package goes up FIRST. The feed is what clients follow, so a feed
      # that names a package not yet in the bucket is a window in which every
      # update attempt 404s.
      #
      # Content types matter: the deployment service dispatches on them.
      # Cache-Control matters more -- a cached .appinstaller is a release
      # nobody receives, so the feed is explicitly uncacheable while the
      # immutable, version-named package is cached hard.
      - name: Publish
        if: github.event_name == 'push'
        shell: pwsh
        env:
          BUCKET: tracecommons-flatpak
          PKG_NAME: ${{ steps.pkg.outputs.name }}
        run: |
          $ErrorActionPreference = "Stop"
          gcloud storage cp "dist\$env:PKG_NAME" "gs://$env:BUCKET/windows/$env:PKG_NAME" `
            --content-type=application/msix --cache-control="public, max-age=31536000, immutable"
          if ($LASTEXITCODE -ne 0) { throw "uploading the package failed" }
          gcloud storage cp "dist\TraceCommons.appinstaller" "gs://$env:BUCKET/windows/TraceCommons.appinstaller" `
            --content-type=application/appinstaller --cache-control="no-cache, max-age=0"
          if ($LASTEXITCODE -ne 0) { throw "uploading the feed failed" }
```

- [ ] **Step 2: Include the new job in the release**

In the `publish` job, change:

```yaml
    needs: [version, macos, windows, linux-flatpak]
```

to:

```yaml
    needs: [version, macos, windows, windows-app, linux-flatpak]
```

and change the gate:

```yaml
      ${{ always() && github.event_name == 'push' &&
          (needs.macos.result == 'success' || needs.windows.result == 'success' ||
           needs.linux-flatpak.result == 'success') }}
```

to:

```yaml
      ${{ always() && github.event_name == 'push' &&
          (needs.macos.result == 'success' || needs.windows.result == 'success' ||
           needs.windows-app.result == 'success' ||
           needs.linux-flatpak.result == 'success') }}
```

In the same job's `download-artifact` step, change:

```yaml
          pattern: "{macos-dmg,windows-zip}"
```

to:

```yaml
          pattern: "{macos-dmg,windows-zip,windows-msix}"
```

And in the `Publish` step, add `WINDOWS_APP_RESULT` to the `env:` block:

```yaml
          WINDOWS_APP_RESULT: ${{ needs.windows-app.result }}
```

then add this paragraph immediately after the existing `if [ "$WINDOWS_RESULT" = success ]; then ... fi` block:

```bash
          if [ "$WINDOWS_APP_RESULT" = success ]; then
            NOTES="$NOTES

          Windows app: install the signed MSIX from
          https://storage.googleapis.com/tracecommons-flatpak/windows/TraceCommons.appinstaller
          Windows keeps it up to date from that same address afterwards."
          fi
```

- [ ] **Step 3: Catch packaging breakage on every pull request**

In `.github/workflows/ci.yml`, in the `windows-contributor-app` job, replace the final step:

```yaml
      - name: Build the WinUI app
        working-directory: windows
        env:
          TC_FFI_LIB_DIR: ${{ github.workspace }}\target\release
        run: msbuild src/TraceCommons.App/TraceCommons.App.csproj -restore -p:Configuration=Release -p:Platform=x64
```

with:

```yaml
      # Packaging, not just compiling. Package.appxmanifest, MakePri and the
      # visual assets are only exercised by a packaging build, and every one
      # of them can break without touching a line of C#. The package is
      # unsigned here -- this runner holds no signing authority and must not.
      - name: Build and package the WinUI app
        working-directory: windows
        env:
          TC_FFI_LIB_DIR: ${{ github.workspace }}\target\release
        run: msbuild src/TraceCommons.App/TraceCommons.App.csproj -restore -p:Configuration=Release -p:Platform=x64 -p:GenerateAppxPackageOnBuild=true

      - name: Confirm exactly one package was produced
        working-directory: windows
        shell: pwsh
        run: |
          $ErrorActionPreference = "Stop"
          $found = @(Get-ChildItem -Recurse -Filter *.msix -Path dist\msix)
          if ($found.Count -ne 1) {
            throw "expected exactly 1 .msix, found $($found.Count): $($found.Name -join ', ')"
          }
          Write-Host $found[0].Name

      # The feed generator runs against the same identity the manifest
      # carries, so a rename on either side fails here rather than at
      # release time.
      - name: Generate and parse the appinstaller feed
        shell: pwsh
        run: |
          $ErrorActionPreference = "Stop"
          ./windows/scripts/make-appinstaller.ps1 `
            -PackageName ai.tracecommons.Contributor `
            -Publisher "CN=TraceCommons Development, O=TraceCommons, C=US" `
            -Version 0.0.1.0 -ProcessorArchitecture x64 `
            -BaseUri https://storage.googleapis.com/tracecommons-flatpak/windows `
            -PackageFileName probe.msix `
            -OutputPath "$env:RUNNER_TEMP\TraceCommons.appinstaller"
          [xml]$feed = Get-Content "$env:RUNNER_TEMP\TraceCommons.appinstaller"
          $ns = New-Object System.Xml.XmlNamespaceManager($feed.NameTable)
          $ns.AddNamespace('a', 'http://schemas.microsoft.com/appx/appinstaller/2017/2')
          $ns.AddNamespace('s4', 'http://schemas.microsoft.com/appx/appinstaller/2021')
          if (-not $feed.SelectSingleNode('/a:AppInstaller/a:UpdateSettings/s4:AutomaticBackgroundTask', $ns)) {
            throw "the feed lost its background update task"
          }
          if ($feed.SelectSingleNode('//s4:ForceUpdateFromAnyVersion', $ns)) {
            throw "the feed allows downgrades, which this project forbids"
          }
          $main = $feed.SelectSingleNode('/a:AppInstaller/a:MainPackage', $ns)
          [xml]$manifest = Get-Content windows\src\TraceCommons.App\Package.appxmanifest
          if ($main.Name -ne $manifest.Package.Identity.Name) {
            throw "feed package name $($main.Name) does not match manifest identity $($manifest.Package.Identity.Name)"
          }
```

- [ ] **Step 4: Document the distribution path**

In `windows/README.md`, add this to the end of the "Packaging" section added in Task 1:

```markdown
### Distribution

The release job publishes two objects to the public bucket:

| Object | Content type | Cache-Control |
| --- | --- | --- |
| `windows/ai.tracecommons.Contributor_<version>_x64.msix` | `application/msix` | `public, max-age=31536000, immutable` |
| `windows/TraceCommons.appinstaller` | `application/appinstaller` | `no-cache, max-age=0` |

The package is uploaded before the feed, so there is never a window in which
the feed names an object that is not there yet. The feed is uncacheable on
purpose: a cached `.appinstaller` is a release nobody receives.

Contributors install once from
`https://storage.googleapis.com/tracecommons-flatpak/windows/TraceCommons.appinstaller`.
After that Windows checks the feed on app launch, at most once every 8 hours,
and again every 8 hours in the background whether or not the app was opened.
The app additionally surfaces a banner and an apply-now action, which drains
any in-flight upload before handing the update to the deployment service.

The app never replaces its own bytes. That is the same rule Homebrew, flatpak
and winget enforce on the other three paths.
```

- [ ] **Step 5: Prove the whole release path without publishing**

`workflow_dispatch` runs every step above except the two guarded by
`github.event_name == 'push'`, so this builds, signs, installs, verifies the
virtualization behavior and generates the feed, and publishes nothing.

Run:

```bash
gh workflow run release-apps.yml --repo TraceCommons/trace-commons-server \
  -f platform=windows -f version=0.0.0
gh run list --repo TraceCommons/trace-commons-server \
  --workflow release-apps.yml --limit 1
```

Then confirm, in the `Windows signed MSIX` job's logs:
- `Discover the certificate subject` prints a `certificate subject: CN=...` line.
- `Sign and verify` prints `Successfully verified:` and `The signature is timestamped:` for the `.msix`.
- `Install it on the runner` prints `registered ai.tracecommons.Contributor_0.0.0.0_x64__<hash>`.
- `Confirm the state directory is not virtualized` completes without throwing.
- The `windows-msix` artifact contains one `.msix`, one `.msix.sha256` and one `TraceCommons.appinstaller`.

- [ ] **Step 6: Commit**

```bash
git add .github/workflows/release-apps.yml .github/workflows/ci.yml windows/README.md
git commit -m "Build, sign, install-verify and publish the Windows MSIX and its feed"
```

---

## Verification

Every one of these must pass before the branch is done.

Cross-platform (macOS, Linux or Windows), from the repository root:

```bash
cargo build -p trace-commons-contributor-ffi
dotnet test windows/tests/TraceCommons.Interop.Tests/TraceCommons.Interop.Tests.csproj
```
Expected: `Failed: 0`, 41 tests total.

On Windows, from `windows/`:

```powershell
cargo build -p trace-commons-contributor-ffi --release
msbuild src\TraceCommons.App\TraceCommons.App.csproj -restore -p:Configuration=Release -p:Platform=x64 -p:GenerateAppxPackageOnBuild=true
```
Expected: `Build succeeded.`, `0 Warning(s)`, `0 Error(s)`, and exactly one `.msix` under `windows\dist\msix`.

On Windows, from the repository root:

```powershell
powershell -ExecutionPolicy Bypass -File windows\scripts\make-app-icons.ps1
& windows\scripts\stamp-package-identity.ps1 -ManifestPath "$env:TEMP\probe.appxmanifest" -Version 1.2.3 -Publisher "CN=Iqlusion Inc"
& windows\scripts\make-appinstaller.ps1 -PackageName ai.tracecommons.Contributor -Publisher "CN=Iqlusion Inc" -Version 1.2.3.0 -ProcessorArchitecture x64 -BaseUri https://storage.googleapis.com/tracecommons-flatpak/windows -PackageFileName probe.msix -OutputPath "$env:TEMP\TraceCommons.appinstaller"
```
Expected: three PNGs at 44x44, 150x150 and 50x50; `1.2.3.0` on stdout from the stamper (after copying the manifest to `$env:TEMP\probe.appxmanifest`); a feed whose `UpdateSettings` contains `OnLaunch` and `s4:AutomaticBackgroundTask` and no `s4:ForceUpdateFromAnyVersion`.

Against real infrastructure:

```bash
gh workflow run release-apps.yml --repo TraceCommons/trace-commons-server -f platform=windows -f version=0.0.0
```
Expected: both the `Windows signed binaries` and `Windows signed MSIX` jobs succeed, the `.msix` verifies with `signtool verify /pa /v` including a timestamp line, `Get-AppxPackage -Name ai.tracecommons.Contributor` reports version `0.0.0.0` on the runner, the virtualization probe lands at the real `%LOCALAPPDATA%\trace-commons`, and nothing is uploaded to the bucket.

## Operator prerequisites

None of these are code, and the plan cannot complete without them.

1. **Bucket prefix.** `gs://tracecommons-flatpak/windows/` must be writable by the service account behind the `GCP_FLATPAK_PUBLISHER_SA` repository secret, and publicly readable the same way the flatpak repo prefix already is (`allUsers` -> `roles/storage.objectViewer`). Confirm with:
   ```bash
   gcloud storage buckets describe gs://tracecommons-flatpak --format='value(name)'
   gcloud storage buckets get-iam-policy gs://tracecommons-flatpak --format=json | grep -A2 allUsers
   ```
2. **No bucket-level default that overrides the publish step's headers.** The `.appinstaller` must be served as `application/appinstaller` with `Cache-Control: no-cache`. The publish step sets both per object; confirm afterwards with:
   ```bash
   curl -sSI https://storage.googleapis.com/tracecommons-flatpak/windows/TraceCommons.appinstaller | grep -i 'content-type\|cache-control'
   ```
3. **The Trusted Signing certificate subject.** It is discovered automatically at release time and never configured, but it does become the package's permanent publisher identity. Read it once, deliberately, before the first real release, and confirm it is the identity Trace Commons should ship under:
   ```powershell
   # On Windows, against any already-released signed CLI binary.
   (Get-AuthenticodeSignature .\trace-commons-contributor.exe).SignerCertificate.Subject
   ```
   If the Trusted Signing profile's identity validation is ever re-issued with a *different* subject, every installed copy stops updating: Windows treats a package with a different publisher as a different app. Treat a subject change as a migration, not a release.
4. **Package identity is permanent.** `ai.tracecommons.Contributor` and application id `TraceCommons` cannot be changed after the first public release without stranding every installed copy. Approve them now.
5. **First install is manual.** Nothing in this plan installs the app on a contributor's machine for the first time. The release notes and the website must carry the one-time link `https://storage.googleapis.com/tracecommons-flatpak/windows/TraceCommons.appinstaller`.
6. **Restricted capabilities are a Store decision.** The package declares `packageManagement` and `unvirtualizedResources`, both restricted. That is correct for `.appinstaller` distribution and forecloses Microsoft Store submission without Microsoft's approval. If Store distribution is ever wanted, `unvirtualizedResources` is the one to revisit first, and revisiting it means solving the shared-state-directory problem another way.
