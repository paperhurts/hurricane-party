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
                autobuild (D133).

    Versions are PINNED (O11). A surprise yt-dlp bump the day before a storm is the
    wrong failure. Every file comes from this project's own sidecars release, where
    the exact archives are kept (D183): yt-dlp prunes its dated ffmpeg autobuilds
    after a few weeks, which is how the first pin went 404. Each archive is checked
    against its SHA-256 and each program against its own, so a machine holding any
    other copy fetches this one without -Force.

    To bump: fetch the new archives from their publishers, check them against the
    publishers' checksums, upload them to a new sidecars-YYYY-MM release (a
    pre-release, never latest: the download page reads latest), and change the
    pins here and in fetch-sidecars.sh, with licenses/THIRD-PARTY-NOTICES.md and
    the matching licence text updated in the same change: the release zip carries
    that folder (D133).

    Tauri resolves externalBin by appending the target triple, so each file is
    named <tool>-x86_64-pc-windows-msvc.exe. binaries/ is gitignored.
#>
[CmdletBinding()]
param(
    # The sidecars release (D183) and what it holds: yt-dlp 2026.08.19, deno
    # 2.6.4, and yt-dlp's ffmpeg autobuild-2026-10-01-19-27.
    [string]$Mirror       = "https://github.com/paperhurts/hurricane-party/releases/download/sidecars-2026-10",
    [string]$YtDlpSha     = "66674953fe251b89f4d08c5f0e35e0728679bd67ab3d7d05c0562af101dd3e7a",
    [string]$DenoZip      = "deno-x86_64-pc-windows-msvc.zip",
    [string]$DenoZipSha   = "0774ae74018ef970ac7364ecec5ef1689b2d67c177be8a541540ef106149d1d9",
    [string]$DenoExeSha   = "58a59444d6318f933a9e115b4519df13d305b36818b724ca44aa2c4e7f42ad93",
    [string]$FfmpegZip    = "ffmpeg-N-127083-g65a3870462-win64-gpl.zip",
    [string]$FfmpegZipSha = "c10df25bfdd2f8ecfdbca2eefe6feb6391ae6d55c4d9795cf613ebd747d87ceb",
    [string]$FfmpegExeSha = "763ba7b90492a41f5f57cb7c9e3f488de6c44507fa5bb36e9b8917e8c8355a81",
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
# "There" means that exact file: anything else is fetched again.
function Need($name, $Sha) {
    $dest = Join-Path $binDir "$name-$triple.exe"
    if ((Test-Path $dest) -and -not $Force) {
        if ((Sha256 $dest) -eq $Sha) {
            Write-Host "  $name already present, skipping (use -Force to refetch)" -ForegroundColor DarkGray
            return $null
        }
        Write-Host "  $name present but not the pinned build, fetching it" -ForegroundColor Yellow
    }
    return $dest
}

# A download that is not the pinned file is refused, not shipped.
function Fetch($file, $Sha) {
    $out = Join-Path $tmp $file
    Invoke-WebRequest -Uri "$Mirror/$file" -OutFile $out -UseBasicParsing
    $got = Sha256 $out
    if ($got -ne $Sha) {
        throw "${file}: the download's SHA-256 is $got, not the pinned $Sha"
    }
    return $out
}

# One program out of a zip, checked against its own pin once it is out.
function Unzip($zip, $exe, $dest, $Sha) {
    $ex = Join-Path $tmp ([IO.Path]::GetFileNameWithoutExtension($zip))
    Remove-Item -Recurse -Force $ex -ErrorAction SilentlyContinue
    Expand-Archive -Path $zip -DestinationPath $ex -Force
    Copy-Item (Get-ChildItem -Path $ex -Filter $exe -Recurse | Select-Object -First 1).FullName $dest -Force
    if ((Sha256 $dest) -ne $Sha) {
        throw "$exe in the pinned zip is not the pinned $exe"
    }
}

# --- yt-dlp: a single exe, no extraction ------------------------------------
$dest = Need "yt-dlp" $YtDlpSha
if ($dest) {
    Write-Host "fetching yt-dlp" -ForegroundColor Cyan
    Copy-Item (Fetch "yt-dlp.exe" $YtDlpSha) $dest -Force
}

# --- deno: zipped ------------------------------------------------------------
$dest = Need "deno" $DenoExeSha
if ($dest) {
    Write-Host "fetching deno" -ForegroundColor Cyan
    Unzip (Fetch $DenoZip $DenoZipSha) "deno.exe" $dest $DenoExeSha
}

# --- ffmpeg: zipped, nested directory ---------------------------------------
$dest = Need "ffmpeg" $FfmpegExeSha
if ($dest) {
    Write-Host "fetching ffmpeg (D133: yt-dlp's build, ~190 MB)" -ForegroundColor Cyan
    Unzip (Fetch $FfmpegZip $FfmpegZipSha) "ffmpeg.exe" $dest $FfmpegExeSha
}

# A dev build runs its sidecars from target\debug (and a local release build
# from target\release), copied there by Tauri's build script, which does not
# run again just because a file in binaries\ changed. Found when the dev app
# kept running the old ffmpeg after this script fetched the new one (#18):
# refresh any copy that is already there, so the next launch runs the pin.
# Not $profile: that is PowerShell's own variable.
foreach ($flavour in "debug", "release") {
    foreach ($name in "yt-dlp", "deno", "ffmpeg") {
        $built = Join-Path $root "src-tauri\target\$flavour\$name.exe"
        $pinned = Join-Path $binDir "$name-$triple.exe"
        if ((Test-Path $built) -and (Test-Path $pinned) -and ((Sha256 $built) -ne (Sha256 $pinned))) {
            Copy-Item $pinned $built -Force
            Write-Host "  refreshed target\$flavour\$name.exe" -ForegroundColor Yellow
        }
    }
}

Write-Host "`nsrc-tauri/binaries/:" -ForegroundColor Green
Get-ChildItem $binDir -Filter "*.exe" | ForEach-Object {
    "{0,-44} {1,7:N1} MB" -f $_.Name, ($_.Length / 1MB)
}
