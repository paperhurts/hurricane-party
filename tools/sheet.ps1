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
    Rows are the format's states in the format's order. The sheet is eight
    cells wide, or as wide as the longest state when one has more, up to 32
    (D183), so a pack of eight or fewer a state is laid out as it always was.
    A state with no files is left out of the manifest and the app falls back
    to idle for it. No idle is an error here, as it is in the app.

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
    [string]$Cells = "",
    # Also write sheet@2x.png: the same sheet at twice the cell size, packed
    # from the same sources, so a companion beside 2x chrome (D76) is drawn
    # from real detail and not from its 1x frames doubled (D160). A state with
    # ready cells needs them at 2x too, in -Cells2x, with the same names.
    [switch]$Double,
    [string]$Cells2x = "",
    # Where a pose sits across its cell. box: the middle of its opaque box, as
    # the captain was packed. mass: the middle of its weight, then nudged only
    # as far as it must be to stay inside the cell, so a tail swung out to one
    # side or a hand reaching in does not slide the body sideways between
    # frames (Wee Man, D164).
    [ValidateSet("box", "mass")][string]$Centre = "box"
)
if ($Smooth) { $Filter = "bicubic" }

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing

# The format's vocabulary, fixed (purricane.md): the row order on the sheet,
# and the timing each state carries in the manifest.
$States = @("idle", "sleep", "dance", "walk", "startle", "pet", "carry")
# The sheet is never narrower than eight cells, the layout every pack had
# before D183, and a state holds at most 32 poses: 32 cells of 128 px, a
# 64 px pack's twin, is 4096 px wide.
$MinColumns = 8
$MaxColumns = 32
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
    if ($files.Count -gt $MaxColumns) { throw "$state has $($files.Count) frames; a state holds $MaxColumns" }
    $i = 0
    foreach ($f in $files) {
        $bmp = [System.Drawing.Bitmap]::FromFile($f.FullName)
        if ($ready -and ($bmp.Width -ne $Frame -or $bmp.Height -ne $Frame)) {
            throw "$($f.FullName) is $($bmp.Width)x$($bmp.Height); a ready cell must be exactly ${Frame}x${Frame}"
        }
        $bmp2 = $null
        if ($ready -and $Double) {
            $twin = if ($Cells2x) { Join-Path $Cells2x $f.Name } else { "" }
            if (-not $twin -or -not (Test-Path -LiteralPath $twin)) {
                throw "$($f.Name) is a ready cell, so -Double needs it at 2x in -Cells2x too"
            }
            $bmp2 = [System.Drawing.Bitmap]::FromFile($twin)
            if ($bmp2.Width -ne 2 * $Frame -or $bmp2.Height -ne 2 * $Frame) {
                throw "$twin is $($bmp2.Width)x$($bmp2.Height); a 2x ready cell must be exactly $(2 * $Frame)x$(2 * $Frame)"
            }
        }
        # Objects, not hashtables: Where-Object and Group-Object resolve a
        # property, and Windows PowerShell 5.1 does not read a hashtable's
        # keys as properties there.
        $frames += [pscustomobject]@{ state = $state; index = $i; path = $f.FullName; bmp = $bmp; bmp2 = $bmp2; box = $null; massX = $null; ready = $ready }
        $i++
    }
}
if (-not ($frames | Where-Object { $_.state -eq "idle" })) { throw "no idle-*.png in $In; idle is the one state a pack cannot go without" }
# As wide as the longest state, and never narrower than eight.
$Columns = $MinColumns
foreach ($fr in $frames) { if ($fr.index + 1 -gt $Columns) { $Columns = $fr.index + 1 } }
if ($Columns * $Frame -gt 4096) {
    throw "$Columns cells of $Frame px make a sheet $($Columns * $Frame) px wide; the app reads 4096 a side"
}

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

# Where a pose's weight sits across it: the alpha-weighted mean column, from
# the left of its opaque box.
function Get-MassX([System.Drawing.Bitmap]$b, [System.Drawing.Rectangle]$box) {
    $rect = New-Object System.Drawing.Rectangle 0, 0, $b.Width, $b.Height
    $data = $b.LockBits($rect, [System.Drawing.Imaging.ImageLockMode]::ReadOnly, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    try {
        $bytes = New-Object byte[] ($data.Stride * $b.Height)
        [System.Runtime.InteropServices.Marshal]::Copy($data.Scan0, $bytes, 0, $bytes.Length)
        $weight = 0.0; $moment = 0.0
        for ($y = $box.Top; $y -lt $box.Bottom; $y++) {
            $row = $y * $data.Stride
            for ($x = $box.Left; $x -lt $box.Right; $x++) {
                $a = $bytes[$row + $x * 4 + 3]
                if ($a -gt 8) { $weight += $a; $moment += $a * ($x - $box.Left + 0.5) }
            }
        }
    } finally { $b.UnlockBits($data) }
    $moment / $weight
}

$maxW = 0; $maxH = 0
foreach ($fr in ($frames | Where-Object { -not $_.ready })) {
    $box = Get-OpaqueBox $fr.bmp
    if ($null -eq $box) { throw "$($fr.path) is fully transparent" }
    $fr.box = $box
    if ($Centre -eq "mass") { $fr.massX = Get-MassX $fr.bmp $box }
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

# One sheet at cell size $px: the scaled poses at $mul times the one factor,
# and the ready cells as they are (their 2x twins when $twox).
function Write-Sheet([int]$px, [double]$mul, [bool]$twox, [string]$sheetfile) {
$sheet = New-Object System.Drawing.Bitmap ($Columns * $px), ($rows * $px), ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
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
            $src = if ($twox) { $fr.bmp2 } else { $fr.bmp }
            $cell = New-Object System.Drawing.Rectangle ($fr.index * $px), ($row * $px), $px, $px
            $all = New-Object System.Drawing.Rectangle 0, 0, $px, $px
            $g.DrawImage($src, $cell, $all, [System.Drawing.GraphicsUnit]::Pixel)
            $g.InterpolationMode = $scaled
            continue
        }
        $w = [Math]::Max(1, [int][Math]::Round($fr.box.Width * $factor * $mul))
        $h = [Math]::Max(1, [int][Math]::Round($fr.box.Height * $factor * $mul))
        # Centred on the pose's own width (or its weight, nudged to stay in
        # the cell), feet on the bottom edge.
        $left = [int][Math]::Floor(($px - $w) / 2)
        if ($Centre -eq "mass") {
            $left = [int][Math]::Round($px / 2 - $fr.massX * $factor * $mul)
            $left = [Math]::Min([Math]::Max($left, 0), $px - $w)
        }
        $x = $fr.index * $px + $left
        $y = $row * $px + ($px - $h)
        $dest = New-Object System.Drawing.Rectangle $x, $y, $w, $h
        $g.DrawImage($fr.bmp, $dest, $fr.box, [System.Drawing.GraphicsUnit]::Pixel)
    }
} finally { $g.Dispose() }
$sheet.Save((Join-Path $Out $sheetfile), [System.Drawing.Imaging.ImageFormat]::Png)
$sheet.Dispose()
}

Write-Sheet $Frame 1.0 $false "sheet.png"
if ($Double) { Write-Sheet (2 * $Frame) 2.0 $true "sheet@2x.png" }
foreach ($fr in $frames) { $fr.bmp.Dispose(); if ($fr.bmp2) { $fr.bmp2.Dispose() } }

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
if ($Double) { Write-Host ("sheet@2x.png {0}x{1}, {2} px cells" -f ($Columns * 2 * $Frame), ($rows * 2 * $Frame), (2 * $Frame)) }
