# Install the Mocky Mail Store package for testing, before the Store signs it.
# Needs Developer Mode: Settings > System > For developers > Developer Mode on.
# Run from this folder in PowerShell:  powershell -ExecutionPolicy Bypass -File .\install-test.ps1
# Remove afterwards with:  Get-AppxPackage *MockyMail* | Remove-AppxPackage
$ErrorActionPreference = 'Stop'
$layout = Join-Path $PSScriptRoot 'MockyMail'
if (-not (Test-Path (Join-Path $layout 'AppxManifest.xml'))) {
  throw "Can't find MockyMail\AppxManifest.xml next to this script."
}
Add-AppxPackage -Register (Join-Path $layout 'AppxManifest.xml')
Write-Host 'Installed. Open "Mocky Mail" from the Start menu.'
