<#
    check-prereqs.ps1 - is this machine ready to build and run hurricane-party?

    Run it on a fresh clone before anything else. It reports what is missing
    and where to get it, and installs exactly one thing: the repo's git hooks
    (tools/git-hooks), which keep main from being pushed directly.

      powershell -NoProfile -ExecutionPolicy Bypass -File tools\check-prereqs.ps1

    Windows PowerShell 5.1 is enough. pwsh is not required anywhere in this repo.
#>
[CmdletBinding()]
param(
    # Report only; do not touch git config.
    [switch]$NoHooks
)

$ErrorActionPreference = "Continue"
$root = Split-Path -Parent $PSScriptRoot
$script:ok = $true

function Has($cmd) { return [bool](Get-Command $cmd -ErrorAction SilentlyContinue) }
function Check($name, $test, $hint) {
    if ($test) {
        Write-Host ("  [ok]   {0}" -f $name) -ForegroundColor Green
    } else {
        Write-Host ("  [MISS] {0}" -f $name) -ForegroundColor Yellow
        Write-Host ("         -> {0}" -f $hint) -ForegroundColor DarkGray
        $script:ok = $false
    }
}

Write-Host "hurricane-party prerequisites" -ForegroundColor Cyan

Check "git"              (Has "git")   "https://git-scm.com"
Check "gh (GitHub CLI)"  (Has "gh")    "winget install GitHub.cli   then: gh auth login"
Check "rustup / cargo"   (Has "cargo") "https://rustup.rs  (MSVC toolchain; it prompts for the VS Build Tools)"

$components = ""
if (Has "rustup") { $components = (& rustup component list --installed 2>$null) -join "`n" }
Check "rustfmt"          ($components -match "rustfmt") "rustup component add rustfmt"
Check "clippy"           ($components -match "clippy")  "rustup component add clippy"

Check "node"             (Has "node")  "https://nodejs.org  (24.x)"
Check "pnpm"             (Has "pnpm")  "npm install -g pnpm@9"

$wvKeys = @(
    "HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}",
    "HKCU:\Software\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}"
)
$wv = $false
foreach ($k in $wvKeys) { if (Test-Path $k) { $wv = $true } }
Check "WebView2 runtime" $wv "Windows 11 ships it. Otherwise https://developer.microsoft.com/microsoft-edge/webview2/"

$bin = Join-Path $root "src-tauri\binaries"
$triple = "x86_64-pc-windows-msvc"
foreach ($t in "yt-dlp", "deno", "ffmpeg") {
    Check ("sidecar: {0}" -f $t) (Test-Path (Join-Path $bin ("{0}-{1}.exe" -f $t, $triple))) "powershell -NoProfile -ExecutionPolicy Bypass -File tools\fetch-sidecars.ps1"
}

Check "node_modules"     (Test-Path (Join-Path $root "node_modules")) "pnpm install"

if (-not $NoHooks -and (Has "git")) {
    $current = & git -C $root config --get core.hooksPath
    if ($current -eq "tools/git-hooks") {
        Write-Host "  [ok]   git hooks (tools/git-hooks): main is pull-request only" -ForegroundColor Green
    } else {
        & git -C $root config core.hooksPath tools/git-hooks
        Write-Host "  [set]  core.hooksPath = tools/git-hooks: main is pull-request only; merge on GitHub" -ForegroundColor Green
    }
}

if ($script:ok) {
    Write-Host "`nReady:  pnpm tauri dev" -ForegroundColor Green
    exit 0
} else {
    Write-Host "`nFix the [MISS] lines, then run this again." -ForegroundColor Yellow
    exit 1
}
