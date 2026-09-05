<#
    build-site.ps1 - build the download page (#66) into site/dist/ for GitHub Pages.

      powershell -NoProfile -ExecutionPolicy Bypass -File tools\build-site.ps1
      powershell -NoProfile -ExecutionPolicy Bypass -File tools\build-site.ps1 -SizeMb 103

    Fills site/index.html's {{tokens}}, {{version}} and {{size}} from
    design/tokens.json, package.json and the -SizeMb you pass (the Pages
    workflow reads it from the latest release), resizes the capybara PNGs from
    design/icon/ to a web width (the sources are 1254 px and 1-3 MB each; the
    page would weigh 10 MB), copies site/shots/, and writes a .nojekyll so
    Pages serves the folder as-is.

    Every colour on the page comes through {{tokens}}: the template holds no
    hex, the same rule the app keeps (CLAUDE.md).

    Windows PowerShell 5.1, System.Drawing only.
#>
param(
    [int]$SizeMb = 0,
    [string]$Version = "",
    [int]$ImageWidth = 720
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$site = Join-Path $root "site"
$dist = Join-Path $site "dist"

# ---- inputs -------------------------------------------------------------------

$tokens = Get-Content (Join-Path $root "design\tokens.json") -Raw -Encoding UTF8 | ConvertFrom-Json
$colors = $tokens.themes.eyewall.colors
$cssVars = ($colors.PSObject.Properties | ForEach-Object { "    --$($_.Name): $($_.Value);" }) -join "`n"

if (-not $Version) {
    $pkg = Get-Content (Join-Path $root "package.json") -Raw -Encoding UTF8 | ConvertFrom-Json
    $Version = "v" + $pkg.version
}
$sizeLabel = if ($SizeMb -gt 0) { "$SizeMb MB" } else { "about 100 MB" }

# ---- html ---------------------------------------------------------------------

New-Item -ItemType Directory -Force (Join-Path $dist "img") | Out-Null
New-Item -ItemType Directory -Force (Join-Path $dist "shots") | Out-Null

$html = Get-Content (Join-Path $site "index.html") -Raw -Encoding UTF8
$html = $html.Replace("{{tokens}}", $cssVars).Replace("{{version}}", $Version).Replace("{{size}}", $sizeLabel)
if ($html -match "\{\{[a-z]+\}\}") { throw "unfilled field in site/index.html: $($Matches[0])" }
[System.IO.File]::WriteAllText((Join-Path $dist "index.html"), $html, (New-Object System.Text.UTF8Encoding $false))
[System.IO.File]::WriteAllText((Join-Path $dist ".nojekyll"), "", (New-Object System.Text.UTF8Encoding $false))

# ---- art ----------------------------------------------------------------------

Add-Type -AssemblyName System.Drawing

function Resize-Png([string]$In, [string]$Out, [int]$Width) {
    $src = [System.Drawing.Bitmap]::FromFile($In)
    try {
        $w = [Math]::Min($Width, $src.Width)
        $h = [int][Math]::Round($src.Height * ($w / $src.Width))
        $dst = New-Object System.Drawing.Bitmap $w, $h, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
        try {
            $g = [System.Drawing.Graphics]::FromImage($dst)
            $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
            $g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
            $g.CompositingMode = [System.Drawing.Drawing2D.CompositingMode]::SourceCopy
            $g.DrawImage($src, 0, 0, $w, $h)
            $g.Dispose()
            $dst.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
        } finally { $dst.Dispose() }
    } finally { $src.Dispose() }
}

# Page name -> source file. The page names them by role; the sources carry
# their size in the name (design/icon convention).
$art = @{
    "capybara-boombox.png"     = "capybara-boombox-1254.png"
    "capybara-truck.png"       = "capybara-truck-1254.png"
    "capybara-lounging.png"    = "capybara-lounging-1254.png"
    "capybara-desk.png"        = "capybara-desk-1254.png"
    "hurricane-party-flag.png" = "hurricane-party-flag-1254.png"
}
foreach ($name in $art.Keys) {
    Resize-Png (Join-Path $root "design\icon\$($art[$name])") (Join-Path $dist "img\$name") $ImageWidth
}
# The app icon, small, as the tab icon.
Resize-Png (Join-Path $root "design\icon\capybara-1254.png") (Join-Path $dist "img\favicon.png") 64

# ---- screenshots --------------------------------------------------------------

$shots = Join-Path $site "shots"
if (Test-Path $shots) {
    Copy-Item (Join-Path $shots "*.png") (Join-Path $dist "shots") -Force
}
$missing = @("step1.png", "step2.png", "step3.png", "windows.png", "windowshade.png") | Where-Object { -not (Test-Path (Join-Path $dist "shots\$_")) }
if ($missing) { Write-Warning ("screenshots missing from site/shots: " + ($missing -join ", ")) }

$total = (Get-ChildItem $dist -Recurse -File | Measure-Object Length -Sum).Sum
Write-Host ("built site/dist: {0} {1}, {2:0.0} MB" -f $Version, $sizeLabel, ($total / 1MB))
