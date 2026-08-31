<#
    fetch-sidecars.ps1 — populate src-tauri/binaries/ for Tauri's externalBin.

    Three sidecars (D46, D47, D48):
      yt-dlp  — the OFFICIAL standalone exe, not the pip package. The pip form is a
                Python zipapp and cannot be bundled. The official exe also ships the
                EJS challenge code, so --remote-components is unnecessary (D47).
      deno    — the JS runtime yt-dlp enables by default, required for its EJS
                challenge system. This replaced the PO-token provider (D25 -> D46).
      ffmpeg  — extracts MP3 from the downloaded video (D3).

    Versions are PINNED (O11). A surprise yt-dlp bump the day before a storm is the
    wrong failure. Bump deliberately, test, then commit the new pin.

    Tauri resolves externalBin by appending the target triple, so each file is
    named <tool>-x86_64-pc-windows-msvc.exe. binaries/ is gitignored.
#>
[CmdletBinding()]
param(
    [string]$YtDlpVersion = "2026.08.19",
    [string]$DenoVersion  = "2.6.4",
    # ffmpeg is the deferred one (D48). "essentials" is the dev stand-in; the
    # shipping build gets decided at v0.6 when packaging actually matters.
    [string]$FfmpegUrl    = "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip",
    [switch]$Force
)

$ErrorActionPreference = "Stop"
$triple  = "x86_64-pc-windows-msvc"
$root    = Split-Path -Parent $PSScriptRoot
$binDir  = Join-Path $root "src-tauri\binaries"
$tmp     = Join-Path $env:TEMP "hp-sidecars"

New-Item -ItemType Directory -Force -Path $binDir, $tmp | Out-Null

function Need($name) {
    $dest = Join-Path $binDir "$name-$triple.exe"
    if ((Test-Path $dest) -and -not $Force) {
        Write-Host "  $name already present, skipping (use -Force to refetch)" -ForegroundColor DarkGray
        return $null
    }
    return $dest
}

# --- yt-dlp: a single exe, no extraction ------------------------------------
$dest = Need "yt-dlp"
if ($dest) {
    $url = "https://github.com/yt-dlp/yt-dlp/releases/download/$YtDlpVersion/yt-dlp.exe"
    Write-Host "fetching yt-dlp $YtDlpVersion" -ForegroundColor Cyan
    Invoke-WebRequest -Uri $url -OutFile $dest -UseBasicParsing
}

# --- deno: zipped ------------------------------------------------------------
$dest = Need "deno"
if ($dest) {
    $url = "https://github.com/denoland/deno/releases/download/v$DenoVersion/deno-$triple.zip"
    $zip = Join-Path $tmp "deno.zip"
    Write-Host "fetching deno $DenoVersion" -ForegroundColor Cyan
    Invoke-WebRequest -Uri $url -OutFile $zip -UseBasicParsing
    $ex = Join-Path $tmp "deno"
    Remove-Item -Recurse -Force $ex -ErrorAction SilentlyContinue
    Expand-Archive -Path $zip -DestinationPath $ex -Force
    Copy-Item (Get-ChildItem -Path $ex -Filter "deno.exe" -Recurse | Select-Object -First 1).FullName $dest -Force
}

# --- ffmpeg: zipped, nested directory ---------------------------------------
$dest = Need "ffmpeg"
if ($dest) {
    $zip = Join-Path $tmp "ffmpeg.zip"
    Write-Host "fetching ffmpeg (D48: dev stand-in, shipping build TBD at v0.6)" -ForegroundColor Cyan
    Invoke-WebRequest -Uri $FfmpegUrl -OutFile $zip -UseBasicParsing
    $ex = Join-Path $tmp "ffmpeg"
    Remove-Item -Recurse -Force $ex -ErrorAction SilentlyContinue
    Expand-Archive -Path $zip -DestinationPath $ex -Force
    Copy-Item (Get-ChildItem -Path $ex -Filter "ffmpeg.exe" -Recurse | Select-Object -First 1).FullName $dest -Force
}

Write-Host "`nsrc-tauri/binaries/:" -ForegroundColor Green
Get-ChildItem $binDir -Filter "*.exe" | ForEach-Object {
    "{0,-44} {1,7:N1} MB" -f $_.Name, ($_.Length / 1MB)
}
