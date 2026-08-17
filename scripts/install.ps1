<#
.SYNOPSIS
Install the Trace Commons contributor CLI on Windows.

.DESCRIPTION
The PowerShell counterpart to scripts/install.sh.

    irm https://raw.githubusercontent.com/TraceCommons/trace-commons-server/main/scripts/install.ps1 -OutFile install.ps1
    .\install.ps1

Reading it before running it is encouraged, which is why the two-step form is
documented first. There is deliberately no prettier URL yet: a redirect from
tracecommons.ai would be nicer to type and would add a second thing that can be
wrong, so this points at the one copy that exists.

WHAT THIS REFUSES TO DO

It will not install a binary it cannot verify. The published SHA-256 must match,
and the Authenticode signature must be valid AND name our organisation. There is
no -Force and no -SkipVerify: this tool reads your coding transcripts, so an
installer that can be talked into skipping the checks is worse than no
installer. On failure you get the reason, a non-zero exit, and nothing on your
PATH.

The checksum alone is weak -- it travels from the same origin over the same
connection as the binary, so it mostly catches corruption. The signature is the
stronger claim: it verifies against Microsoft's roots rather than against us, so
it still means something if this file or the binary were served by somebody else.

Checking the signer and not merely that a signature is present is the part that
matters. A validly signed binary from an unexpected publisher is exactly the case
a status-only check waves through.

.PARAMETER Dir
Where to install. Defaults to %LOCALAPPDATA%\Programs\TraceCommons.

.PARAMETER Version
Which release to install, e.g. 0.1.0. Defaults to the newest CLI release.
#>
[CmdletBinding()]
param(
    [string] $Dir,
    [string] $Version = 'latest'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$Repo = 'TraceCommons/trace-commons-server'

# Match on the organisation rather than a full distinguished name: .NET's
# rendering of the subject is not something to depend on, and O= is the part
# that identifies us. Azure Trusted Signing issues a fresh certificate per
# signing job, so the leaf changes constantly while this does not.
$ExpectedSigner = 'O=Iqlusion Inc'

if (-not $Dir) { $Dir = Join-Path $env:LOCALAPPDATA 'Programs\TraceCommons' }

function Die([string] $Message) {
    Write-Host ''
    Write-Host "install failed: $Message" -ForegroundColor Red
    Write-Host 'Nothing was installed.' -ForegroundColor Red
    exit 1
}

# Windows PowerShell 5.1 still negotiates TLS 1.0 by default on some hosts, and
# GitHub refuses it outright -- which surfaces as an unhelpful "could not create
# SSL/TLS secure channel". Ask for 1.2 explicitly.
try {
    [Net.ServicePointManager]::SecurityProtocol =
        [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
} catch {
    # PowerShell 7 on a modern host manages this itself; nothing to do.
}

# ---- which build do you need -------------------------------------------
$arch = $env:PROCESSOR_ARCHITECTURE
if ($env:PROCESSOR_ARCHITEW6432) { $arch = $env:PROCESSOR_ARCHITEW6432 }

switch ($arch) {
    'AMD64' { $target = 'x86_64-pc-windows-msvc' }
    'ARM64' {
        # No native arm64 build is published. Windows on Arm runs x64 under
        # emulation, so this works -- say so rather than silently installing a
        # binary of a different architecture than the machine.
        $target = 'x86_64-pc-windows-msvc'
        Write-Host 'note: no native Arm64 build is published yet; installing the x64 build,'
        Write-Host '      which Windows runs under emulation.'
    }
    default {
        Die "unsupported processor architecture: $arch. See https://docs.tracecommons.ai/cli/quickstart/"
    }
}

$asset = "trace-commons-contributor-$target.exe"

# NOT /releases/latest/download. This repository publishes two independent tag
# streams -- contributor-v* for the CLI and app-v* for the desktop app -- so
# GitHub's "latest release" is whichever was cut most recently and may hold no
# CLI assets at all. Resolve the newest contributor-v* tag explicitly.
if ($Version -eq 'latest') {
    try {
        $releases = Invoke-RestMethod -UseBasicParsing `
            -Uri "https://api.github.com/repos/$Repo/releases?per_page=50" `
            -Headers @{ 'Accept' = 'application/vnd.github+json' }
    } catch {
        Die @"
could not reach the GitHub API to work out the latest CLI release.
It may be rate-limiting you. Pass a version explicitly instead:
  .\install.ps1 -Version 0.1.0
"@
    }
    # Check the SHAPE before reading it. A rate-limited GitHub returns a JSON
    # object -- {message, documentation_url} -- rather than an array of
    # releases, and under `Set-StrictMode -Version Latest` reading .tag_name off
    # that dies with "The property 'tag_name' cannot be found on this object".
    # That is a true statement and a useless one: it sends the reader looking
    # for a malformed release instead of telling them they were throttled. The
    # unauthenticated limit is 60 requests/hour PER IP, so anyone behind a
    # shared address can hit it without having run this script before.
    $usable = @($releases | Where-Object { $_.PSObject.Properties.Name -contains 'tag_name' })
    if ($usable.Count -eq 0) {
        Die @"
GitHub did not return a release list. It is most likely rate-limiting you --
the unauthenticated limit is 60 requests/hour for everyone sharing your IP
address. Pass a version explicitly to skip this lookup entirely:
  .\install.ps1 -Version 0.1.0
"@
    }

    $tag = ($usable |
        Where-Object { $_.tag_name -like 'contributor-v*' } |
        Select-Object -First 1).tag_name
    if (-not $tag) {
        Die "found no contributor-v* release. Pass a version explicitly: .\install.ps1 -Version 0.1.0"
    }
} else {
    $tag = "contributor-v$Version"
}

$base = "https://github.com/$Repo/releases/download/$tag"

Write-Host "release  : $tag"
Write-Host "platform : Windows $arch -> $target"
Write-Host "source   : $base/$asset"
Write-Host "target   : $Dir"

# ---- fetch into a scratch dir, never straight onto PATH ----------------
$tmp = Join-Path ([IO.Path]::GetTempPath()) ("tc-install-" + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $tmp -Force | Out-Null
try {
    $exePath = Join-Path $tmp $asset
    $shaPath = "$exePath.sha256"

    try {
        Invoke-WebRequest -UseBasicParsing -Uri "$base/$asset" -OutFile $exePath
    } catch {
        Die "could not download $base/$asset"
    }
    try {
        Invoke-WebRequest -UseBasicParsing -Uri "$base/$asset.sha256" -OutFile $shaPath
    } catch {
        Die "could not download the checksum for $asset"
    }

    # ---- verify BEFORE anything is installed ---------------------------
    Write-Host ''
    Write-Host 'verifying...'

    # The sidecar is `hash  filename`, so take the first whitespace-separated
    # field rather than the whole line.
    $published = ((Get-Content -Raw $shaPath).Trim() -split '\s+')[0]
    if (-not $published) { Die 'the published checksum file was empty' }

    $actual = (Get-FileHash -Path $exePath -Algorithm SHA256).Hash
    if ($actual -ne $published) {
        Die @"
checksum mismatch for $asset
  published: $published
  actual:    $actual
This means the file you received is not the file that was published -- do not
use it.
"@
    }
    Write-Host "  checksum ok ($($published.ToLower()))"

    $sig = Get-AuthenticodeSignature -FilePath $exePath
    if ($sig.Status -ne 'Valid') {
        Die "the Authenticode signature on $asset is not valid: $($sig.Status) - $($sig.StatusMessage)"
    }
    $subject = $sig.SignerCertificate.Subject
    if ($subject -notmatch [regex]::Escape($ExpectedSigner)) {
        Die @"
unexpected signing identity on $asset
  expected to contain: $ExpectedSigner
  found:               $subject
"@
    }
    Write-Host "  signed by $subject"
    Write-Host '  (the signing certificate is short-lived by design; the RFC3161'
    Write-Host '   timestamp is what keeps the signature valid afterwards)'

    # ---- install ------------------------------------------------------
    New-Item -ItemType Directory -Path $Dir -Force | Out-Null
    $dest = Join-Path $Dir 'trace-commons-contributor.exe'
    try {
        Move-Item -Path $exePath -Destination $dest -Force
    } catch {
        Die @"
could not write $dest
If the CLI or its daemon is running, stop it and run this again -- Windows will
not replace a file that is in use.
"@
    }

    Write-Host ''
    Write-Host "installed: $dest"
    & $dest --version

    # PATH, for this user only. Nothing here needs administrator rights.
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    $onPath = ($userPath -split ';' | Where-Object { $_.TrimEnd('\') -ieq $Dir.TrimEnd('\') })
    if (-not $onPath) {
        $newPath = if ($userPath) { "$userPath;$Dir" } else { $Dir }
        [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
        Write-Host ''
        Write-Host "Added $Dir to your PATH. Reopen your terminal for it to take effect."
    }

    Write-Host ''
    Write-Host 'next: trace-commons-contributor login --invite <url>'
    Write-Host 'docs: https://docs.tracecommons.ai/cli/quickstart/'
} finally {
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}
