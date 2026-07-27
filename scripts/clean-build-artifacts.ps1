param(
  [string]$ProjectRoot = (Resolve-Path "$PSScriptRoot\..").Path,
  [switch]$KeepInstallers
)

$targetPath = Join-Path $ProjectRoot "src-tauri\target"
$distPath = Join-Path $ProjectRoot "dist"

if ($KeepInstallers -and (Test-Path $targetPath)) {
  $bundlePath = Join-Path $targetPath "debug\bundle"
  $exePath = Join-Path $targetPath "debug\kizumail.exe"
  $backupPath = Join-Path $ProjectRoot "build-artifacts"
  if (Test-Path $bundlePath) {
    New-Item -ItemType Directory -Force -Path $backupPath | Out-Null
    Copy-Item -LiteralPath $bundlePath -Destination $backupPath -Recurse -Force
  }
  if (Test-Path $exePath) {
    New-Item -ItemType Directory -Force -Path $backupPath | Out-Null
    Copy-Item -LiteralPath $exePath -Destination $backupPath -Force
  }
}

if (Test-Path $targetPath) {
  Remove-Item -LiteralPath $targetPath -Recurse -Force
}

if (Test-Path $distPath) {
  Remove-Item -LiteralPath $distPath -Recurse -Force
}

Write-Host "Build artifacts removed."
if ($KeepInstallers) {
  Write-Host "Installer bundles copied to build-artifacts when available."
}
