<#
    fetch-sidecars.ps1 — populate src-tauri/binaries/ for Tauri's externalBin.

    Three sidecars (D46, D47, D133):
      yt-dlp  — the OFFICIAL standalone exe, not the pip package. The pip form is a
                Python zipapp and cannot be bundled. The official exe also ships the
                EJS challenge code, so --remote-components is unnecessary (D47).
      deno    — the JS runtime yt-dlp enables by default, required for its EJS
                challenge system. This replaced the PO-token provider (D25 -> D46).
      ffmpeg  — extracts MP3 from the downloaded video (D3), and does yt-dlp's
                merging, tagging and thumbnail conversion. yt-dlp's own build
                (github.com/yt-dlp/FFmpeg-Builds), win64 GPL, pinned to one dated
                autobuild and checked against its SHA-256 (D133).

    Versions are PINNED (O11). A surprise yt-dlp bump the day before a storm is the
    wrong failure. Bump deliberately, test, then commit the new pin, with
    licenses/THIRD-PARTY-NOTICES.md and the matching licence text updated in the
    same change: the release zip carries that folder (D133).

    Tauri resolves externalBin by appending the target triple, so each file is
    named <tool>-x86_64-pc-windows-msvc.exe. binaries/ is gitignored.
#>
[CmdletBinding()]
param(
    [string]$YtDlpVersion = "2026.08.19",
    [string]$DenoVersion  = "2.6.4",
    # yt-dlp's ffmpeg build (D133): a dated autobuild, never "latest", which
    # moves daily. The zip is checked against the SHA-256 yt-dlp publishes for
    # it, and ffmpeg.exe against the one inside it, so a machine still holding
    # an older ffmpeg fetches this one without -Force.
    [string]$FfmpegUrl    = "https://github.com/yt-dlp/FFmpeg-Builds/releases/download/autobuild-2026-09-11-17-43/ffmpeg-N-126504-g1b8a2b690b-win64-gpl.zip",
    [string]$FfmpegZipSha = "2d2b30a1e31bbc3dde699d95e699f6a32178febffdf2bf552e5d6384fed97859",
    [string]$FfmpegExeSha = "c130d1abd89a5c832a51b2415fdccc4b8dafdb8055b8b7977c0ecd94148b7afc",
    [switch]$Force
)

$ErrorActionPreference = "Stop"
$triple  = "x86_64-pc-windows-msvc"
$root    = Split-Path -Parent $PSScriptRoot
$binDir  = Join-Path $root "src-tauri\binaries"
$tmp     = Join-Path $env:TEMP "hp-sidecars"

New-Item -ItemType Directory -Force -Path $binDir, $tmp | Out-Null

function Sha256($path) {
    return (Get-FileHash -Algorithm SHA256 -Path $path).Hash.ToLowerInvariant()
}

# The sidecar's destination when it needs fetching, or $null when it is there.
# With -Sha, "there" means that exact file: anything else is fetched again.
function Need($name, $Sha = "") {
    $dest = Join-Path $binDir "$name-$triple.exe"
    if ((Test-Path $dest) -and -not $Force) {
        if (-not $Sha -or (Sha256 $dest) -eq $Sha) {
            Write-Host "  $name already present, skipping (use -Force to refetch)" -ForegroundColor DarkGray
            return $null
        }
        Write-Host "  $name present but not the pinned build, fetching it" -ForegroundColor Yellow
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
$dest = Need "ffmpeg" $FfmpegExeSha
if ($dest) {
    $zip = Join-Path $tmp "ffmpeg.zip"
    Write-Host "fetching ffmpeg (D133: yt-dlp's build, ~185 MB)" -ForegroundColor Cyan
    Invoke-WebRequest -Uri $FfmpegUrl -OutFile $zip -UseBasicParsing
    # A download that is not the pinned build is refused, not shipped.
    $got = Sha256 $zip
    if ($got -ne $FfmpegZipSha) {
        throw "ffmpeg: the download's SHA-256 is $got, not the pinned $FfmpegZipSha"
    }
    $ex = Join-Path $tmp "ffmpeg"
    Remove-Item -Recurse -Force $ex -ErrorAction SilentlyContinue
    Expand-Archive -Path $zip -DestinationPath $ex -Force
    Copy-Item (Get-ChildItem -Path $ex -Filter "ffmpeg.exe" -Recurse | Select-Object -First 1).FullName $dest -Force
    if ((Sha256 $dest) -ne $FfmpegExeSha) {
        throw "ffmpeg: ffmpeg.exe in the pinned zip is not the pinned exe"
    }
}

Write-Host "`nsrc-tauri/binaries/:" -ForegroundColor Green
Get-ChildItem $binDir -Filter "*.exe" | ForEach-Object {
    "{0,-44} {1,7:N1} MB" -f $_.Name, ($_.Length / 1MB)
}
