param(
  [string]$ProjectRoot = (Resolve-Path "$PSScriptRoot\..").Path
)

$cargoBin = Join-Path $env:USERPROFILE ".cargo\bin"
$msvcRoot = "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Tools\MSVC\14.43.34808"
$windowsSdkVersion = "10.0.22000.0"
$windowsSdkRoot = "C:\Program Files (x86)\Windows Kits\10"

$env:Path = "$cargoBin;$msvcRoot\bin\HostX64\x64;$env:Path"
$env:LIB = "$msvcRoot\lib\x64;$windowsSdkRoot\Lib\$windowsSdkVersion\um\x64;$windowsSdkRoot\Lib\$windowsSdkVersion\ucrt\x64"
$env:INCLUDE = "$msvcRoot\include;$windowsSdkRoot\Include\$windowsSdkVersion\um;$windowsSdkRoot\Include\$windowsSdkVersion\ucrt;$windowsSdkRoot\Include\$windowsSdkVersion\shared"

npm --prefix $ProjectRoot run tauri -- build --debug
