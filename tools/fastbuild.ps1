# Build the desktop application, drop it on the Desktop, and reset the config.
#
# For bench testing between fixes: no full test suite, no CI, no release. One
# build, one clean start. Run from the repository root.
$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
$dest = "C:\Users\gilde\Desktop\ortseam-build"

Push-Location $root
cargo build --release -p ortseam-gui
if ($LASTEXITCODE -ne 0) { Pop-Location; throw "build failed" }
Pop-Location

Copy-Item (Join-Path $root "target\release\ortseam-gui.exe") (Join-Path $dest "ortseam-gui.exe") -Force

# A fresh start every time: no remembered window, no recent files, no layout.
$app = Join-Path $env:APPDATA "ortseam"
if (Test-Path $app) { Remove-Item $app -Recurse -Force }

$exe = Get-Item (Join-Path $dest "ortseam-gui.exe")
"{0}  {1:N0} bytes  {2:HH:mm:ss}" -f $exe.Name, $exe.Length, $exe.LastWriteTime
"config reset - ortseam-gui.exe is ready on the Desktop"
