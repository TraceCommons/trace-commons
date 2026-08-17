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
