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

# ---- the social preview ---------------------------------------------------------
#
# 1280 x 640, what a pasted link shows in a chat (og:image) and, uploaded by
# hand in the repo's Settings, the GitHub social preview. The Florida truck
# with the whole crew (#62) on the void, the name beside it. Text is drawn
# with Iosevka where it is installed and Consolas where it is not (the CI
# runner), so the file is honest either way.

function Find-Font([string[]]$Names, [single]$Size, [System.Drawing.FontStyle]$Style) {
    foreach ($n in $Names) {
        try { return New-Object System.Drawing.Font $n, $Size, $Style, ([System.Drawing.GraphicsUnit]::Pixel) } catch {}
    }
    New-Object System.Drawing.Font ([System.Drawing.FontFamily]::GenericMonospace), $Size, $Style, ([System.Drawing.GraphicsUnit]::Pixel)
}

function Build-Social([string]$Out) {
    $W = 1280; $H = 640
    $bmp = New-Object System.Drawing.Bitmap $W, $H, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    try {
        $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
        $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
        $g.TextRenderingHint = [System.Drawing.Text.TextRenderingHint]::AntiAliasGridFit
        $g.Clear([System.Drawing.ColorTranslator]::FromHtml($colors.void))

        # The art takes the right 600 px; the words keep the left 640 clear.
        $art = [System.Drawing.Bitmap]::FromFile((Join-Path $root "design\icon\capybara-florida-1254.png"))
        try {
            $ah = 600; $aw = [int]($art.Width * ($ah / $art.Height))
            $g.DrawImage($art, $W - $aw, 20, $aw, $ah)
        } finally { $art.Dispose() }

        $ink = New-Object System.Drawing.SolidBrush ([System.Drawing.ColorTranslator]::FromHtml($colors.filament))
        $arc = New-Object System.Drawing.SolidBrush ([System.Drawing.ColorTranslator]::FromHtml($colors.arc))
        $strike = New-Object System.Drawing.SolidBrush ([System.Drawing.ColorTranslator]::FromHtml($colors.strike))
        $dim = New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(150, [System.Drawing.ColorTranslator]::FromHtml($colors.filament)))
        $mono = @("Iosevka", "Consolas", "Cascadia Mono")
        $eyebrow = Find-Font $mono 18 ([System.Drawing.FontStyle]::Regular)
        $title = Find-Font $mono 92 ([System.Drawing.FontStyle]::Bold)
        $tag = Find-Font @("Iosevka Aile", "Segoe UI", "Consolas") 26 ([System.Drawing.FontStyle]::Regular)
        $url = Find-Font $mono 20 ([System.Drawing.FontStyle]::Regular)

        # The dot as a code point: PowerShell 5.1 reads this file as ANSI, and
        # a literal middle dot comes out as two characters.
        $g.DrawString(("OFFLINE MEDIA PLAYER  {0}  WINDOWS" -f [char]0xB7), $eyebrow, $strike, 72, 150)
        $g.DrawString("hurricane-", $title, $ink, 60, 185)
        $g.DrawString("party", $title, $ink, 60, 280)
        $g.DrawString("Save YouTube videos and MP3s to disk." + [Environment]::NewLine + "Play them when the internet is down.", $tag, $dim, 72, 400)
        $g.DrawString("paperhurts.github.io/hurricane-party", $url, $arc, 72, 520)
    } finally {
        $g.Dispose()
    }
    $bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
}
Build-Social (Join-Path $dist "social.png")

$total = (Get-ChildItem $dist -Recurse -File | Measure-Object Length -Sum).Sum
Write-Host ("built site/dist: {0} {1}, {2:0.0} MB" -f $Version, $sizeLabel, ($total / 1MB))
