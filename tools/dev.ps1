<#
    dev.ps1 - start `pnpm tauri dev` on a clean slate (#11).

      powershell -NoProfile -ExecutionPolicy Bypass -File tools\dev.ps1

    `pnpm tauri dev` reaches vite through three cmd.exe shims (pnpm > cmd >
    tauri CLI > cmd > pnpm dev > cmd > vite) and no job object, so when the
    CLI is killed rather than allowed to exit (a stopped background task, a
    closed terminal) it takes its direct child with it and leaves vite
    listening on 1420. The port is fixed (`devUrl`, `strictPort`), so the next
    start fails; and a hurricane-party.exe left the same way keeps the
    database open beside the next one. This frees both, then starts the dev
    server in the foreground, where Ctrl+C stops it the way it always did.

    Windows PowerShell 5.1.
#>
$ErrorActionPreference = "Stop"
Set-Location (Split-Path -Parent $PSScriptRoot)

$listener = Get-NetTCPConnection -LocalPort 1420 -State Listen -ErrorAction SilentlyContinue | Select-Object -First 1
if ($listener) {
    $p = Get-Process -Id $listener.OwningProcess -ErrorAction SilentlyContinue
    if ($p) {
        "freeing port 1420 from $($p.ProcessName) (pid $($p.Id))"
        Stop-Process -Id $p.Id -Force
        Start-Sleep -Milliseconds 300
    }
}
Get-Process -Name "hurricane-party" -ErrorAction SilentlyContinue | ForEach-Object {
    "stopping a stray hurricane-party.exe (pid $($_.Id))"
    Stop-Process -Id $_.Id -Force
}

pnpm tauri dev
