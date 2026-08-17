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
