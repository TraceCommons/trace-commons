# Enable OpenSSH Server on the Windows dev box, for headless builds.
#
# RDP is for watching the app render. It is the wrong channel for running
# `cargo build` and reading a compiler error, which is most of the work. This
# script adds an SSH surface so the toolchain can be driven non-interactively
# over the same IAP tunnel.
#
# ACCESS MODEL. sshd listens only on the loopback-reachable interface of a VM
# that has NO external IP, and the only route to it is `gcloud compute
# start-iap-tunnel`, which is authenticated by Google IAM before a packet ever
# reaches this host. Password authentication is disabled outright: the key
# below is the sole credential, and a password prompt reachable through a
# tunnel is a brute-force target with no upside.
#
# Replaces the bootstrap script in instance metadata, so it runs on boot. It is
# idempotent for the same reason the bootstrap is -- GCE re-runs startup
# scripts on every boot, and reinstalling a Windows capability each time is
# both slow and pointless.

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

function Write-Log($message) {
    Write-Host "[tc-ssh $((Get-Date).ToString('s'))] $message"
}

# The public half of a keypair generated for this box alone. Rotating it means
# replacing this line and restarting; nothing else depends on it.
$authorizedKey = 'ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIC9E3wcedTEU/yOlL8zV1tR4HbF6k418nylpcZajUK+s tc-win-dev headless build access'

Write-Log 'Installing the OpenSSH Server capability'
$capability = Get-WindowsCapability -Online -Name 'OpenSSH.Server*'
if ($capability.State -ne 'Installed') {
    Add-WindowsCapability -Online -Name $capability.Name
    Write-Log 'OpenSSH Server installed'
} else {
    Write-Log 'OpenSSH Server already installed'
}

Write-Log 'Starting sshd'
Set-Service -Name sshd -StartupType Automatic
Start-Service sshd

# ---------------------------------------------------------------------------
# The administrators_authorized_keys gotcha.
#
# For any account in the Administrators group, Windows OpenSSH ignores
# ~/.ssh/authorized_keys and reads this file instead. It also REFUSES the file
# unless its ACL grants only Administrators and SYSTEM -- inherited user ACEs
# make sshd silently reject every key, which presents as "permission denied
# (publickey)" with nothing useful in the log at default verbosity.
# ---------------------------------------------------------------------------
$adminKeys = 'C:\ProgramData\ssh\administrators_authorized_keys'
Write-Log "Writing $adminKeys"
Set-Content -Path $adminKeys -Value $authorizedKey -Encoding ascii

$acl = Get-Acl $adminKeys
$acl.SetAccessRuleProtection($true, $false)   # disable inheritance, drop inherited ACEs
$acl.Access | ForEach-Object { $acl.RemoveAccessRule($_) | Out-Null }
foreach ($principal in @('NT AUTHORITY\SYSTEM', 'BUILTIN\Administrators')) {
    $rule = New-Object System.Security.AccessControl.FileSystemAccessRule(
        $principal, 'FullControl', 'Allow')
    $acl.AddAccessRule($rule)
}
Set-Acl -Path $adminKeys -AclObject $acl
Write-Log 'ACL locked to SYSTEM + Administrators'

# ---------------------------------------------------------------------------
# sshd config: keys only.
# ---------------------------------------------------------------------------
$sshdConfig = 'C:\ProgramData\ssh\sshd_config'
$config = Get-Content $sshdConfig -Raw
$config = $config -replace '(?m)^#?\s*PasswordAuthentication\s+.*$', 'PasswordAuthentication no'
$config = $config -replace '(?m)^#?\s*PubkeyAuthentication\s+.*$', 'PubkeyAuthentication yes'
if ($config -notmatch '(?m)^PasswordAuthentication no$') {
    $config += "`r`nPasswordAuthentication no`r`n"
}
Set-Content -Path $sshdConfig -Value $config -Encoding ascii
Write-Log 'sshd_config set to public-key authentication only'

# PowerShell rather than cmd.exe as the SSH shell, so remote commands behave
# the way the build instructions assume.
New-ItemProperty -Path 'HKLM:\SOFTWARE\OpenSSH' `
    -Name DefaultShell `
    -Value 'C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe' `
    -PropertyType String -Force | Out-Null

Restart-Service sshd
Write-Log 'sshd restarted'

# The Windows firewall rule. The GCE-level firewall still only admits the IAP
# range, so this is the second of two gates rather than the only one.
if (-not (Get-NetFirewallRule -Name 'sshd-tc' -ErrorAction SilentlyContinue)) {
    New-NetFirewallRule -Name 'sshd-tc' -DisplayName 'OpenSSH Server (tc-win-dev)' `
        -Enabled True -Direction Inbound -Protocol TCP -Action Allow -LocalPort 22 | Out-Null
    Write-Log 'Firewall rule created for tcp:22'
}

Write-Log 'SSH enablement complete.'
