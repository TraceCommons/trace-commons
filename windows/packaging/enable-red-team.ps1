<#
.SYNOPSIS
  Turns on what Red Team needs on Windows: the Hyper-V feature, and membership of
  the Hyper-V Administrators group for the person who asked.

.DESCRIPTION
  Shipped inside the signed package beside the app and run only by the app,
  elevated through a UAC prompt. The single input is -UserSid, the requesting
  user's SID, which the app reads from its own process token. The script checks
  its shape again and takes nothing else: no paths, no names, no commands.

  Exit codes (read by the app, trace-commons-contributor managed/red_team_host.rs):
    0     done: the user must sign out of Windows and back in for the group to apply
    3010  Hyper-V was turned on: restart Windows
    10    Windows Home, which has no Hyper-V
    11    -UserSid is not a user SID
    1     anything else failed
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$UserSid
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

# S-1-5-21 and at least four more decimal sub-authorities: a local, domain or
# Microsoft account. Never a well-known or built-in SID such as Administrators.
if ($UserSid -cnotmatch '^S-1-5-21(-[0-9]{1,10}){4,}$') { exit 11 }

$HyperVAdministrators = 'S-1-5-32-578'
$Feature = 'Microsoft-Hyper-V-All'
$restart = $false

try {
    # Core, Core N, Core Single Language and Core Country Specific: Windows Home.
    $sku = [int](Get-CimInstance -ClassName Win32_OperatingSystem).OperatingSystemSKU
    if ($sku -in 98, 99, 100, 101) { exit 10 }

    $state = (Get-WindowsOptionalFeature -Online -FeatureName $Feature).State
    if ($state -ne 'Enabled') {
        try {
            $result = Enable-WindowsOptionalFeature -Online -FeatureName $Feature -All -NoRestart
            if ($result.RestartNeeded) { $restart = $true }
        } catch {
            # The same change through DISM, which 3010 means "restart needed".
            & "$env:SystemRoot\System32\dism.exe" /online /enable-feature "/featurename:$Feature" /all /norestart | Out-Null
            if ($LASTEXITCODE -eq 3010) { $restart = $true }
            elseif ($LASTEXITCODE -ne 0) { exit 1 }
        }
    }

    try {
        Add-LocalGroupMember -SID $HyperVAdministrators -Member ([System.Security.Principal.SecurityIdentifier]$UserSid)
    } catch [Microsoft.PowerShell.Commands.MemberExistsException] {
        # Already a member: nothing to add.
    } catch [Microsoft.PowerShell.Commands.GroupNotFoundException] {
        # The group appears with the feature, which can need the restart first.
        # The app offers this again afterwards, and the add happens then.
        if (-not $restart) { exit 1 }
    }
} catch {
    exit 1
}

if ($restart) { exit 3010 }
exit 0
