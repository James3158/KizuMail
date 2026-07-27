$ErrorActionPreference = "Stop"

$registryPaths = @(
  "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*",
  "HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*",
  "HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*"
)

$entry = Get-ItemProperty -Path $registryPaths -ErrorAction SilentlyContinue |
  Where-Object { $_.DisplayName -eq "KizuMail" } |
  Select-Object -First 1

if (-not $entry) {
  throw "KizuMail ist für diesen Windows-Benutzer nicht installiert."
}

$command = if ($entry.QuietUninstallString) { $entry.QuietUninstallString } else { $entry.UninstallString }
if (-not $command) {
  throw "Der KizuMail-Deinstallationsbefehl fehlt in Windows."
}

$installCandidates = @(
  $entry.InstallLocation,
  (Join-Path $env:LOCALAPPDATA "KizuMail"),
  (Join-Path $env:ProgramFiles "KizuMail")
) | Where-Object { $_ }

$installedApp = $installCandidates |
  ForEach-Object { Join-Path $_ "kizumail.exe" } |
  Where-Object { Test-Path -LiteralPath $_ } |
  Select-Object -First 1

if ($installedApp) {
  $cleanup = Start-Process -FilePath $installedApp -ArgumentList "--purge-user-data" -Wait -PassThru
  if ($cleanup.ExitCode -ne 0) {
    throw "Lokale KizuMail-Daten konnten vor der Deinstallation nicht vollständig entfernt werden."
  }
}

if ($command -match '^\s*"([^"]+)"\s*(.*)$') {
  $executable = $Matches[1]
  $arguments = $Matches[2]
} elseif ($command -match '^\s*(.+?\.exe)\s*(.*)$') {
  $executable = $Matches[1]
  $arguments = $Matches[2]
} else {
  throw "Der KizuMail-Deinstallationsbefehl ist ungültig: $command"
}

$process = Start-Process -FilePath $executable -ArgumentList $arguments -Wait -PassThru
if ($process.ExitCode -ne 0) {
  throw "Die KizuMail-Deinstallation wurde mit Exitcode $($process.ExitCode) beendet."
}

Write-Host "KizuMail wurde deinstalliert. Lokale Konten, Maildaten und zugehörige Anmeldedaten wurden entfernt."
