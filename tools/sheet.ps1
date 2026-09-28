<#
    sheet.ps1 - pack companion frames into an hp-companion/1 sprite sheet.

      powershell -NoProfile -ExecutionPolicy Bypass -File tools\sheet.ps1 -In .sid\captain-keyed -Out skins\companions\captain -Frame 64 -Name "Cap'n Capy" -Count 1
      (the captain's poses in design\sprites\captain are the raw green-screen ones;
       key each into .sid\captain-keyed with keyout.ps1 -Size 1024 -NoCrop first)
      powershell -NoProfile -ExecutionPolicy Bypass -File tools\sheet.ps1 -In frames -Out pack -Filter nearest   # real pixel art at its own size

    Reads <state>-<n>.png from -In (idle-0.png, idle-1.png, walk-0.png ...), each a
    transparent PNG of one pose at any size, and writes -Out\sheet.png plus
    -Out\companion.json (purricane.md, hp-companion/1).

    Every frame is scaled by ONE factor, chosen so the tallest and widest
    poses fit a -Frame cell: a crouch stays small and a jump stays tall, and
    the feet land on the cell's bottom edge, which is what `anchor` points at.
    The default -Filter, area, averages what each cell pixel covers, which is
    even across frames; nearest is for pixel art drawn at (or an integer
    multiple of) the cell size, where there is nothing to average.
    Rows are the format's states in the format's order, eight cells each; a
    state with no files is left out of the manifest and the app falls back to
    idle for it. No idle is an error here, as it is in the app.

    Windows PowerShell 5.1, System.Drawing only. See docs/companion-art.md.
#>
param(
    [Parameter(Mandatory = $true)][string]$In,
    [Parameter(Mandatory = $true)][string]$Out,
    [int]$Frame = 64,
    [string]$Name = "Companion",
    [ValidateSet("fixed", "theme")][string]$Palette = "fixed",
    [int]$WalkPxPerSec = 24,
    # How many the app shows by default: the kittens' two (purricane.md), or
    # one for a character like Cap'n Capy (#192).
    [int]$Count = 2,
    # nearest: real pixel art at or near its native size. area: an image
    # model's "pixel art", which is drawn at ~10 px per fake pixel and does not
    # divide evenly into the cell, so nearest keeps or drops whole fake pixels
    # by where the grid falls and every frame comes out blocky differently;
    # averaging over the area each cell pixel covers is even across frames and
    # keeps the outline. bicubic: painted sources.
    [ValidateSet("nearest", "area", "bicubic")][string]$Filter = "area",
    [switch]$Smooth,
    # Ready-made cells: <state>-<n>.png at exactly -Frame x -Frame, placed as
    # they are, pixel for pixel. A state with any here takes all its frames
    # from here and none from -In, so its poses neither get rescaled nor sway
    # the one factor the rest share. For true pixel art drawn at the cell size
    # (docs/companion-art.md, "A pixel-art bot, at native size").
    [string]$Cells = ""
)
if ($Smooth) { $Filter = "bicubic" }

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing

# The format's vocabulary, fixed (purricane.md): the row order on the sheet,
# and the timing each state carries in the manifest.
$States = @("idle", "sleep", "dance", "walk", "startle", "pet", "carry")
$Columns = 8
$Timing = @{
    idle    = @{ fps = 4;  loop = $true }
    sleep   = @{ fps = 1;  loop = $true }
    dance   = @{ syncTo = "beat" }
    walk    = @{ fps = 8;  loop = $true }
    startle = @{ fps = 12; loop = $false; then = "idle" }
    pet     = @{ fps = 6;  loop = $false; then = "idle" }
    carry   = @{ fps = 3;  loop = $true }
}

# ---- gather the frames ----------------------------------------------------------

function Get-StateFiles([string]$dir, [string]$state) {
    if (-not $dir) { return @() }
    @(Get-ChildItem -LiteralPath $dir -Filter "$state-*.png" -File -ErrorAction SilentlyContinue |
        Where-Object { $_.BaseName -match "^$state-(\d+)$" } |
        Sort-Object { [int]($_.BaseName -replace "^$state-", "") })
}

$frames = @()   # @{ state; index; path; bmp; box; ready }
foreach ($state in $States) {
    $files = Get-StateFiles $Cells $state
    $ready = $files.Count -gt 0
    if (-not $ready) { $files = Get-StateFiles $In $state }
    if ($files.Count -gt $Columns) { throw "$state has $($files.Count) frames; the sheet holds $Columns per state" }
    $i = 0
    foreach ($f in $files) {
        $bmp = [System.Drawing.Bitmap]::FromFile($f.FullName)
        if ($ready -and ($bmp.Width -ne $Frame -or $bmp.Height -ne $Frame)) {
            throw "$($f.FullName) is $($bmp.Width)x$($bmp.Height); a ready cell must be exactly ${Frame}x${Frame}"
        }
        # Objects, not hashtables: Where-Object and Group-Object resolve a
        # property, and Windows PowerShell 5.1 does not read a hashtable's
        # keys as properties there.
        $frames += [pscustomobject]@{ state = $state; index = $i; path = $f.FullName; bmp = $bmp; box = $null; ready = $ready }
        $i++
    }
}
if (-not ($frames | Where-Object { $_.state -eq "idle" })) { throw "no idle-*.png in $In; idle is the one state a pack cannot go without" }

# Opaque bounding box of one bitmap: the pose, not the canvas it was drawn on.
function Get-OpaqueBox([System.Drawing.Bitmap]$b) {
    $rect = New-Object System.Drawing.Rectangle 0, 0, $b.Width, $b.Height
    $data = $b.LockBits($rect, [System.Drawing.Imaging.ImageLockMode]::ReadOnly, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    try {
        $bytes = New-Object byte[] ($data.Stride * $b.Height)
        [System.Runtime.InteropServices.Marshal]::Copy($data.Scan0, $bytes, 0, $bytes.Length)
        $minX = $b.Width; $minY = $b.Height; $maxX = -1; $maxY = -1
        for ($y = 0; $y -lt $b.Height; $y++) {
            $row = $y * $data.Stride
            for ($x = 0; $x -lt $b.Width; $x++) {
                if ($bytes[$row + $x * 4 + 3] -gt 8) {
                    if ($x -lt $minX) { $minX = $x }; if ($x -gt $maxX) { $maxX = $x }
                    if ($y -lt $minY) { $minY = $y }; if ($y -gt $maxY) { $maxY = $y }
                }
            }
        }
    } finally { $b.UnlockBits($data) }
    if ($maxX -lt 0) { return $null }
    New-Object System.Drawing.Rectangle $minX, $minY, ($maxX - $minX + 1), ($maxY - $minY + 1)
}

$maxW = 0; $maxH = 0
foreach ($fr in ($frames | Where-Object { -not $_.ready })) {
    $box = Get-OpaqueBox $fr.bmp
    if ($null -eq $box) { throw "$($fr.path) is fully transparent" }
    $fr.box = $box
    if ($box.Width -gt $maxW) { $maxW = $box.Width }
    if ($box.Height -gt $maxH) { $maxH = $box.Height }
}
# One factor for every frame, so the tallest pose fills the cell's height and
# the widest fits its width, and every other pose keeps its size relative to
# them.
$factor = [Math]::Min($Frame / $maxH, $Frame / $maxW)

# ---- draw the sheet --------------------------------------------------------------

New-Item -ItemType Directory -Force $Out | Out-Null
$rows = $States.Count
$sheet = New-Object System.Drawing.Bitmap ($Columns * $Frame), ($rows * $Frame), ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
$g = [System.Drawing.Graphics]::FromImage($sheet)
try {
    $g.CompositingMode = [System.Drawing.Drawing2D.CompositingMode]::SourceCopy
    switch ($Filter) {
        "nearest" {
            $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::NearestNeighbor
            $g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::Half
        }
        "area" {
            # GDI+'s high-quality bilinear prefilters when shrinking, which is
            # the box average this wants; bicubic rings and softens.
            $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBilinear
            $g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
        }
        "bicubic" {
            $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
            $g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
        }
    }
    $g.Clear([System.Drawing.Color]::Transparent)

    $scaled = $g.InterpolationMode
    foreach ($fr in $frames) {
        $row = [Array]::IndexOf($States, $fr.state)
        if ($fr.ready) {
            # As it is: same size in and out, nearest, so no pixel is resampled.
            $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::NearestNeighbor
            $cell = New-Object System.Drawing.Rectangle ($fr.index * $Frame), ($row * $Frame), $Frame, $Frame
            $all = New-Object System.Drawing.Rectangle 0, 0, $Frame, $Frame
            $g.DrawImage($fr.bmp, $cell, $all, [System.Drawing.GraphicsUnit]::Pixel)
            $g.InterpolationMode = $scaled
            continue
        }
        $w = [Math]::Max(1, [int][Math]::Round($fr.box.Width * $factor))
        $h = [Math]::Max(1, [int][Math]::Round($fr.box.Height * $factor))
        # Centred on the pose's own width, feet on the bottom edge.
        $x = $fr.index * $Frame + [int][Math]::Floor(($Frame - $w) / 2)
        $y = $row * $Frame + ($Frame - $h)
        $dest = New-Object System.Drawing.Rectangle $x, $y, $w, $h
        $g.DrawImage($fr.bmp, $dest, $fr.box, [System.Drawing.GraphicsUnit]::Pixel)
    }
} finally { $g.Dispose() }
$sheet.Save((Join-Path $Out "sheet.png"), [System.Drawing.Imaging.ImageFormat]::Png)
$sheet.Dispose()
foreach ($fr in $frames) { $fr.bmp.Dispose() }

# ---- the manifest ----------------------------------------------------------------

# Not `$states`: variable names are case-insensitive, and that would be the
# `$States` list above, emptied just before the loop that reads it.
$manifestStates = [ordered]@{}
foreach ($state in $States) {
    $mine = @($frames | Where-Object { $_.state -eq $state })
    if ($mine.Count -eq 0) { continue }
    $row = [Array]::IndexOf($States, $state)
    $slots = @($mine | ForEach-Object { $row * $Columns + $_.index })
    if ($state -eq "idle" -and $slots.Count -gt 1) {
        # idle-0 is the pose; each later idle frame (the blink, a small shift,
        # docs/companion-art.md) is a moment, not half the loop. Hold the pose
        # eleven ticks, then the moment for one: at 4 fps, a blink every 3 s.
        $held = @()
        foreach ($c in $slots[1..($slots.Count - 1)]) { $held += @($slots[0]) * 11; $held += $c }
        $slots = $held
    }
    $entry = [ordered]@{ frames = $slots }
    foreach ($k in @("fps", "syncTo", "loop", "then")) {
        if ($Timing[$state].ContainsKey($k)) { $entry[$k] = $Timing[$state][$k] }
    }
    $manifestStates[$state] = $entry
}
$manifest = [ordered]@{
    format       = "hp-companion/1"
    name         = $Name
    sprite       = "sheet.png"
    frameSize    = @($Frame, $Frame)
    # Parenthesised on purpose: the comma binds tighter than the minus, and
    # `@(a, $Frame - 1)` subtracts one from the pair.
    anchor       = @([int][Math]::Floor($Frame / 2), ($Frame - 1))
    palette      = $Palette
    states       = $manifestStates
    walkPxPerSec = $WalkPxPerSec
    defaultCount = $Count
}
$json = $manifest | ConvertTo-Json -Depth 6
[System.IO.File]::WriteAllText((Join-Path $Out "companion.json"), $json, (New-Object System.Text.UTF8Encoding $false))

$placed = ($frames | Group-Object state | ForEach-Object {
        $tag = if ($_.Group[0].ready) { " (ready)" } else { "" }
        "$($_.Name) $($_.Count)$tag"
    }) -join ", "
Write-Host ("sheet.png {0}x{1}, {2} px cells, scale {3:0.000}: {4}" -f ($Columns * $Frame), ($rows * $Frame), $Frame, $factor, $placed)
