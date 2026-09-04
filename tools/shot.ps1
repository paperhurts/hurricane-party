<#
    shot.ps1 - screenshot one of the running app's windows into .sid/.

      powershell -NoProfile -ExecutionPolicy Bypass -File tools\shot.ps1 -List
      powershell -NoProfile -ExecutionPolicy Bypass -File tools\shot.ps1 -Match main
      powershell -NoProfile -ExecutionPolicy Bypass -File tools\shot.ps1 -Match library -Out .sid\lib.png

    Finds the hurricane-party process's visible top-level windows and picks the
    one whose title contains -Match (case-insensitive substring; the titles
    carry an em dash that does not survive a command line, so exact match is
    the wrong tool). Renders the window's own content with PrintWindow, so it
    works while something else is on top; -Screen copies the screen rectangle
    instead, which shows neighbours and seams but also whatever covers the app.
    Physical pixels either way. The process is made DPI-aware first; without
    that a 150% monitor hands back a virtualised rectangle and the capture is
    off by half.

    Windows PowerShell 5.1. .sid/ is gitignored scratch (CLAUDE.md).
#>
param(
    [string]$Match = "",
    [string]$Out = "",
    [int]$Pad = 0,
    [switch]$List,
    [switch]$Screen
)

Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Text;
using System.Collections.Generic;
using System.Runtime.InteropServices;
public static class Shot {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
    public delegate bool EnumProc(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr l);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
    public static List<object[]> Windows(uint pid) {
        var found = new List<object[]>();
        EnumWindows((h, l) => {
            uint p; GetWindowThreadProcessId(h, out p);
            if (p != pid || !IsWindowVisible(h)) return true;
            var sb = new StringBuilder(256); GetWindowTextW(h, sb, 256);
            RECT r; GetWindowRect(h, out r);
            found.Add(new object[] { h, sb.ToString(), r.L, r.T, r.R - r.L, r.B - r.T });
            return true;
        }, IntPtr.Zero);
        return found;
    }
}
"@

[void][Shot]::SetProcessDPIAware()

$proc = Get-Process -Name hurricane-party -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $proc) { Write-Error "hurricane-party is not running"; exit 1 }

$wins = [Shot]::Windows([uint32]$proc.Id)
if ($List -or -not $Match) {
    foreach ($w in $wins) { Write-Output ("{0,-40} {1}x{2} at {3},{4}" -f $w[1], $w[4], $w[5], $w[2], $w[3]) }
    exit 0
}

$pick = $wins | Where-Object { $_[1].ToLower().Contains($Match.ToLower()) } | Select-Object -First 1
if (-not $pick) { Write-Error "no visible window matching '$Match'; run with -List"; exit 1 }

$x = [int]$pick[2] - $Pad; $y = [int]$pick[3] - $Pad
$w = [int]$pick[4] + 2 * $Pad; $hgt = [int]$pick[5] + 2 * $Pad
if ($w -le 0 -or $hgt -le 0) { Write-Error "empty rect"; exit 1 }

if (-not $Out) {
    $Out = Join-Path ".sid" ("$Match-" + (Get-Date -Format "HHmmss") + ".png")
}
$dir = Split-Path -Parent $Out
if ($dir -and -not (Test-Path $dir)) { New-Item -ItemType Directory -Path $dir | Out-Null }

$bmp = New-Object System.Drawing.Bitmap $w, $hgt
$g = [System.Drawing.Graphics]::FromImage($bmp)
if ($Screen) {
    # What is composited on screen at that rectangle: neighbours and seams
    # included, but also whatever is on top of the app.
    $g.CopyFromScreen($x, $y, 0, 0, (New-Object System.Drawing.Size $w, $hgt))
} else {
    # The window's own content, even when something covers it. Flag 2 is
    # PW_RENDERFULLCONTENT, which is what makes a WebView2 surface come out.
    $hdc = $g.GetHdc()
    [void][Shot]::PrintWindow([IntPtr]$pick[0], $hdc, 2)
    $g.ReleaseHdc($hdc)
}
$g.Dispose()
$full = Join-Path (Resolve-Path -LiteralPath $dir) (Split-Path -Leaf $Out)
$bmp.Save($full, [System.Drawing.Imaging.ImageFormat]::Png)
$bmp.Dispose()

Write-Output ("{0}  ({1}x{2} at {3},{4})" -f $Out, $w, $hgt, $x, $y)
