$ErrorActionPreference = 'SilentlyContinue'
$deadline = (Get-Date).AddSeconds(@@WAIT_SECONDS@@)
while ((Get-Date) -lt $deadline) {
  Start-Sleep -Seconds 2
  $p = Get-AppxPackage -Name '@@PACKAGE_NAME@@' | Where-Object { $_.PackageFamilyName -eq '@@PACKAGE_FAMILY@@' } | Select-Object -First 1
  if ($p -and $p.Version.ToString() -ne '@@CURRENT_VERSION@@') {
    Start-Sleep -Seconds 2
    Start-Process 'shell:AppsFolder\@@PACKAGE_FAMILY@@!@@APP_ID@@'
    break
  }
}
