<#
.SYNOPSIS
Captures a demo state of mantaray into a PNG, for the documentation.

.DESCRIPTION
Builds nothing - point it at an existing release build. Launches the
application off-screen-ish (position/size come from MANTARAY_WINDOW_POS and
MANTARAY_WINDOW_SIZE), captures the window by handle with PrintWindow, and
closes it again. The shell is made per-monitor DPI aware first; without that
GetWindowRect returns virtualized coordinates and the bitmap silently crops
the right and bottom of the window.

.EXAMPLE
./tools/screenshot.ps1 -Demo "analyse" -Out docs/screenshots/analyse.png

.EXAMPLE
./tools/screenshot.ps1 -Demo "insight" -Files @() -Out insight.png
#>
param(
    # MANTARAY_DEMO state, optionally prefixed with a theme: "paper:1".
    [string]$Demo = "1",
    [string]$Out = "shot.png",
    # Spectra to open; most demo states expect one.
    [string[]]$Files = @("$PSScriptRoot\..\crates\mantaray-formats\tests\fixtures\eu152_spectra.Spe"),
    [string]$Exe = "$PSScriptRoot\..\target\release\mantaray-gui.exe",
    # Window position (monitor choice) and logical size.
    [string]$Pos = "2560,20",
    [string]$Size = "1900,1060",
    # Add-Type cannot redefine a class in one shell; give each run a fresh name.
    [string]$ClassName = "Cap$([guid]::NewGuid().ToString('N').Substring(0,8))"
)
$env:MANTARAY_DEMO = $Demo
$env:MANTARAY_WINDOW_POS = $Pos
$env:MANTARAY_WINDOW_SIZE = $Size
if ($Files.Count -gt 0) {
    $p = Start-Process -FilePath $Exe -ArgumentList ($Files | ForEach-Object { '"' + $_ + '"' }) -PassThru
} else {
    $p = Start-Process -FilePath $Exe -PassThru
}
Start-Sleep -Seconds 4

Add-Type -Name $ClassName -Namespace Shot -MemberDefinition @'
[DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr hwnd, IntPtr hdc, uint flags);
[DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hwnd, out RECT rect);
[DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr hwnd, IntPtr after, int x, int y, int cx, int cy, uint flags);
[DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr value);
public struct RECT { public int Left, Top, Right, Bottom; }
'@
Add-Type -AssemblyName System.Drawing

$cls = "Shot.$ClassName" -as [type]
# -4 = DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2.
$cls::SetProcessDpiAwarenessContext([IntPtr]-4) | Out-Null

$h = (Get-Process -Id $p.Id).MainWindowHandle
if ($h -eq [IntPtr]::Zero) { Start-Sleep -Seconds 3; $h = (Get-Process -Id $p.Id).MainWindowHandle }
# Raise without stealing focus: SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE.
$cls::SetWindowPos($h, [IntPtr]::Zero, 0, 0, 0, 0, 0x10 -bor 0x2 -bor 0x1) | Out-Null
Start-Sleep -Milliseconds 800
$r = New-Object "Shot.$ClassName+RECT"
$cls::GetWindowRect($h, [ref]$r) | Out-Null
$w = $r.Right - $r.Left; $ht = $r.Bottom - $r.Top
$bmp = New-Object System.Drawing.Bitmap($w, $ht)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$hdc = $g.GetHdc()
# 2 = PW_RENDERFULLCONTENT, required for GPU-rendered windows.
$cls::PrintWindow($h, $hdc, 2) | Out-Null
$g.ReleaseHdc($hdc); $g.Dispose()
$bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
$bmp.Dispose()
Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
"saved $Out ($w x $ht)"
