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

# The publisher must be an X.500 name matching what stamp-package-identity.ps1
# wrote into the manifest and what signtool actually signed with -- not just
# any ASCII string. Same check, same rationale, as Task 2.
if ($Publisher -notmatch 'CN=') {
    throw "Publisher does not look like a certificate subject (no CN=): '$Publisher'."
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
