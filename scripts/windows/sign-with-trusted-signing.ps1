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
