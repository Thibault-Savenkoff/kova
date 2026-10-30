# Installs the ready-made kova of the latest release, for this user only (no administrator rights):
#
#   irm https://raw.githubusercontent.com/Thibault-Savenkoff/kova/main/install.ps1 | iex
#
# Into %LOCALAPPDATA%\Programs\kova, added to this user's PATH. Nothing else is needed to use kova.
$ErrorActionPreference = 'Stop'
$name = if ($env:PROCESSOR_ARCHITECTURE -eq 'ARM64') { 'windows-arm64' } else { 'windows-x64' }
$dir = Join-Path $env:LOCALAPPDATA 'Programs\kova'
$zip = Join-Path $env:TEMP "kova-$name.zip"
Invoke-WebRequest "https://github.com/Thibault-Savenkoff/kova/releases/latest/download/kova-$name.zip" -OutFile $zip
New-Item -ItemType Directory -Force $dir | Out-Null
Expand-Archive $zip $dir -Force
Remove-Item $zip
$path = [Environment]::GetEnvironmentVariable('Path', 'User')
if (($path -split ';') -notcontains $dir) {
    [Environment]::SetEnvironmentVariable('Path', "$path;$dir", 'User')
    $env:Path += ";$dir"
}
Write-Host "✓ $(& "$dir\kova.exe" --version) installed: $dir\kova.exe (open a new terminal to type ``kova``)"
