param(
  [string]$ProjectRoot = $PSScriptRoot
)

$ErrorActionPreference = "Stop"
$project = (Resolve-Path -LiteralPath $ProjectRoot).Path
$target = Join-Path $project "src-tauri\target"
$dist = Join-Path $project "dist"
$release = Join-Path $target "release"
$bundle = Join-Path $release "bundle"
$artifacts = Join-Path $project "build-artifacts"
$resourceDir = Join-Path $project "src-tauri\resources"
$bundledOAuthConfig = Join-Path $resourceDir "oauth-clients.local.json"
$oauthConfig = Join-Path $project "oauth-clients.local.json"

$cargoBin = Join-Path $env:USERPROFILE ".cargo\bin"
$msvcRoot = "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Tools\MSVC\14.43.34808"
$windowsSdkVersion = "10.0.22000.0"
$windowsSdkRoot = "C:\Program Files (x86)\Windows Kits\10"
if (-not (Test-Path -LiteralPath $msvcRoot) -or -not (Test-Path -LiteralPath "$windowsSdkRoot\Lib\$windowsSdkVersion")) {
  throw "The configured Visual Studio/MSVC and Windows SDK toolchain was not found."
}
$env:Path = "$cargoBin;$msvcRoot\bin\HostX64\x64;$env:Path"
$env:LIB = "$msvcRoot\lib\x64;$windowsSdkRoot\Lib\$windowsSdkVersion\um\x64;$windowsSdkRoot\Lib\$windowsSdkVersion\ucrt\x64"
$env:INCLUDE = "$msvcRoot\include;$windowsSdkRoot\Include\$windowsSdkVersion\um;$windowsSdkRoot\Include\$windowsSdkVersion\ucrt;$windowsSdkRoot\Include\$windowsSdkVersion\shared"

function Assert-ProjectPath([string]$Path) {
  $resolved = [System.IO.Path]::GetFullPath($Path).TrimEnd('\')
  $root = $project.TrimEnd('\')
  if ($resolved -eq $root -or -not $resolved.StartsWith("$root\", [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "Refusing to operate outside the project directory: $resolved"
  }
  return $resolved
}

New-Item -ItemType Directory -Path $resourceDir -Force | Out-Null
if (Test-Path -LiteralPath $oauthConfig) {
  Copy-Item -LiteralPath $oauthConfig -Destination $bundledOAuthConfig -Force
} else {
  Copy-Item -LiteralPath (Join-Path $project "oauth-clients.example.json") -Destination $bundledOAuthConfig -Force
  Write-Warning "No oauth-clients.local.json found. The installers will require OAuth client IDs in the app."
}

$buildArguments = @("--prefix", $project, "run", "tauri", "--", "build")
$signingConfig = $null
if ($env:KIZUMAIL_SIGN_CERT_THUMBPRINT) {
  $signingConfig = Join-Path $project "src-tauri\tauri.signing.local.json"
  @{
    bundle = @{
      windows = @{
        certificateThumbprint = $env:KIZUMAIL_SIGN_CERT_THUMBPRINT
        digestAlgorithm = "sha256"
        timestampUrl = if ($env:KIZUMAIL_SIGN_TIMESTAMP_URL) { $env:KIZUMAIL_SIGN_TIMESTAMP_URL } else { "http://timestamp.digicert.com" }
      }
    }
  } | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $signingConfig -Encoding UTF8
  $buildArguments += @("--config", $signingConfig)
  Write-Host "Building and signing KizuMail release..."
} else {
  Write-Host "Building unsigned KizuMail release..."
  Write-Warning "Set KIZUMAIL_SIGN_CERT_THUMBPRINT to sign future release builds."
}

try {
  & npm @buildArguments
} finally {
  if ($signingConfig -and (Test-Path -LiteralPath $signingConfig)) {
    Remove-Item -LiteralPath $signingConfig -Force
  }
}
if ($LASTEXITCODE -ne 0) {
  throw "Tauri build failed with exit code $LASTEXITCODE. Existing artifacts were kept."
}

$exe = Join-Path $release "kizumail.exe"
$msi = Get-ChildItem -LiteralPath (Join-Path $bundle "msi") -Filter "*.msi" -File | Select-Object -First 1
$nsis = Get-ChildItem -LiteralPath (Join-Path $bundle "nsis") -Filter "*.exe" -File | Select-Object -First 1
if (-not (Test-Path -LiteralPath $exe) -or $null -eq $msi -or $null -eq $nsis) {
  throw "Build completed but one or more expected outputs are missing. Temporary files were kept for inspection."
}

Write-Host "Refreshing build-artifacts..."
$artifactRoot = Assert-ProjectPath $artifacts
if (Test-Path -LiteralPath $artifactRoot) {
  Get-ChildItem -LiteralPath $artifactRoot -Force | Remove-Item -Recurse -Force
} else {
  New-Item -ItemType Directory -Path $artifactRoot | Out-Null
}

Copy-Item -LiteralPath $exe -Destination (Join-Path $artifactRoot "kizumail.exe") -Force
if (Test-Path -LiteralPath $oauthConfig) {
  Copy-Item -LiteralPath $oauthConfig -Destination (Join-Path $artifactRoot "oauth-clients.local.json") -Force
  Write-Host "Copied local OAuth configuration."
}

New-Item -ItemType Directory -Path (Join-Path $artifactRoot "bundle\msi") -Force | Out-Null
New-Item -ItemType Directory -Path (Join-Path $artifactRoot "bundle\nsis") -Force | Out-Null
Copy-Item -LiteralPath $msi.FullName -Destination (Join-Path $artifactRoot "bundle\msi\$($msi.Name)") -Force
Copy-Item -LiteralPath $nsis.FullName -Destination (Join-Path $artifactRoot "bundle\nsis\$($nsis.Name)") -Force
Copy-Item -LiteralPath (Join-Path $project "scripts\uninstall-kizumail.ps1") -Destination (Join-Path $artifactRoot "Uninstall-KizuMail.ps1") -Force

Write-Host "Removing temporary build output..."
Remove-Item -LiteralPath (Assert-ProjectPath $target) -Recurse -Force
if (Test-Path -LiteralPath $dist) {
  Remove-Item -LiteralPath (Assert-ProjectPath $dist) -Recurse -Force
}
if (Test-Path -LiteralPath $bundledOAuthConfig) {
  Remove-Item -LiteralPath (Assert-ProjectPath $bundledOAuthConfig) -Force
}

Write-Host "Done. Outputs:"
Get-ChildItem -LiteralPath $artifactRoot -Recurse -File | ForEach-Object {
  Write-Host (" - " + $_.FullName)
}
