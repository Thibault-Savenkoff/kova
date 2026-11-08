# Checks what only a real Windows can show: the right-click menu, an image in a window, the Start menu
# shortcut of `kova install`. Run it in PowerShell, in the folder of the repository's clone:
#   powershell -ExecutionPolicy Bypass -File tests\manual\windows\check.ps1
# It writes its results to tests\manual\windows\windows_check_result.txt and pushes them to the branch
# `windows-check-result`, so nobody has to copy anything.
$ErrorActionPreference = "Continue"
$repo = (Resolve-Path "$PSScriptRoot\..\..\..").Path
$here = $PSScriptRoot
$result = "$here\windows_check_result.txt"
Remove-Item $result -ErrorAction SilentlyContinue
function Note($line) { Add-Content -Path $result -Value $line; Write-Host $line }

Set-Location $repo
git pull --ff-only
Note "commit: $(git rev-parse --short HEAD)"
cargo install --path . --locked
$kova = "$env:USERPROFILE\.cargo\bin\kova.exe"
Note "version: $(& $kova --version)"

# 1 and 2: the window.
Set-Location $here
Write-Host "A window opens: right-click its first line, choose 'Say hello', answer the picture question, then press Done."
& $kova run window.kova
if (-not (Select-String -Path $result -Pattern "right-click menu: OK" -Quiet)) { Note "right-click menu: NOT used" }

# 3: the Start menu.
& $kova install start_menu.kova
$lnk = "$env:APPDATA\Microsoft\Windows\Start Menu\Programs\kovacheck.lnk"
$exe = "$env:LOCALAPPDATA\Programs\kovacheck\kovacheck.exe"
Note "start menu shortcut file: $(Test-Path $lnk)"
Note "installed program file: $(Test-Path $exe)"
$answer = Read-Host "Open the Start menu and type 'kovacheck'. Is it there? (y/n)"
Note "start menu entry seen: $answer"
& $kova uninstall start_menu.kova
Note "shortcut removed by uninstall: $(-not (Test-Path $lnk))"

# Send the results back.
Set-Location $repo
git checkout -B windows-check-result
git add -f "tests/manual/windows/windows_check_result.txt"
git commit -m "Windows check results"
git push -f origin windows-check-result
git checkout main
Write-Host "Done. You can close this window."
