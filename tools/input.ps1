<#
    input.ps1 - real mouse input at physical screen coordinates, for testing the
    app the way a user does (CLAUDE.md, D43). Not synthetic DOM events: these go
    through the OS, so an invisible window eating clicks, a pointer capture
    that dies, or a double-click that WebView2 refuses to synthesise all show
    up here and nowhere else.

      powershell -NoProfile -ExecutionPolicy Bypass -File tools\input.ps1 click 1510 633
      powershell -NoProfile -ExecutionPolicy Bypass -File tools\input.ps1 dblclick 1419 518
      powershell -NoProfile -ExecutionPolicy Bypass -File tools\input.ps1 drag 270 199 270 260 [-Steps 12]
      powershell -NoProfile -ExecutionPolicy Bypass -File tools\input.ps1 move 1419 518

    Coordinates are the physical ones tools\shot.ps1 -List prints. The cursor
    is put back where it was afterwards (not after `move`, which is for hover).

    Windows PowerShell 5.1.
#>
param(
    [Parameter(Mandatory = $true, Position = 0)][ValidateSet("click", "dblclick", "drag", "move")][string]$Action,
    [Parameter(Mandatory = $true, Position = 1)][int]$X,
    [Parameter(Mandatory = $true, Position = 2)][int]$Y,
    [Parameter(Position = 3)][int]$X2,
    [Parameter(Position = 4)][int]$Y2,
    [int]$Steps = 12
)

Add-Type @"
using System;
using System.Runtime.InteropServices;
public static class HpInput {
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern bool GetCursorPos(out POINT p);
    [DllImport("user32.dll")] public static extern void mouse_event(uint flags, int dx, int dy, uint data, IntPtr extra);
}
"@
[void][HpInput]::SetProcessDPIAware()
$DOWN = 0x0002; $UP = 0x0004

function Press { [HpInput]::mouse_event($DOWN, 0, 0, 0, [IntPtr]::Zero); Start-Sleep -Milliseconds 40; [HpInput]::mouse_event($UP, 0, 0, 0, [IntPtr]::Zero) }

$was = New-Object HpInput+POINT
[void][HpInput]::GetCursorPos([ref]$was)
[void][HpInput]::SetCursorPos($X, $Y)
Start-Sleep -Milliseconds 80

switch ($Action) {
    "move" { "moved to $X,$Y"; exit 0 }
    "click" { Press; "clicked $X,$Y" }
    "dblclick" { Press; Start-Sleep -Milliseconds 200; Press; "double-clicked $X,$Y" }
    "drag" {
        [HpInput]::mouse_event($DOWN, 0, 0, 0, [IntPtr]::Zero)
        Start-Sleep -Milliseconds 80
        for ($i = 1; $i -le $Steps; $i++) {
            $x = [int]($X + ($X2 - $X) * $i / $Steps); $y = [int]($Y + ($Y2 - $Y) * $i / $Steps)
            [void][HpInput]::SetCursorPos($x, $y)
            Start-Sleep -Milliseconds 25
        }
        Start-Sleep -Milliseconds 80
        [HpInput]::mouse_event($UP, 0, 0, 0, [IntPtr]::Zero)
        "dragged $X,$Y -> $X2,$Y2"
    }
}
Start-Sleep -Milliseconds 60
[void][HpInput]::SetCursorPos($was.X, $was.Y)
