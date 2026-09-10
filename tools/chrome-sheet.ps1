<#
    chrome-sheet.ps1 - draw the Eyewall chrome sheet from its manifest (#3, D90).

      powershell -NoProfile -ExecutionPolicy Bypass -File tools\chrome-sheet.ps1

    The Eyewall skin is `art: mask` (D73): every sprite is a white shape whose
    alpha is the art, and the renderer tints it from the palette. So there is
    no colour in here, and "drawing" is filling rectangles and placing glyph
    pixels. The rectangles come from skins/eyewall/manifest.json, which is the
    one place geometry lives; this script only knows what each named element
    looks like, and writes the sheet at every scale the manifest lists (1x and
    2x, D73), with strokes that thick.

    The look is the CSS chrome that shipped in v0.4b, transcribed: a hairline
    frame at 30%, a title bar wash at 10% (6% when the group is inactive), and
    13 x 9 title-bar buttons with a 24% ring that goes full on hover, fills at
    14% while pressed, and dims to 45% when inactive.

    Windows PowerShell 5.1, System.Drawing only. Re-run after editing the
    manifest's rectangles or the glyphs below; commit the PNGs it writes.
#>
param(
    [string]$Manifest = "skins\eyewall\manifest.json",
    [string]$Out = "skins\eyewall"
)
$ErrorActionPreference = "Stop"
Set-Location (Split-Path -Parent $PSScriptRoot)
Add-Type -AssemblyName System.Drawing

$m = Get-Content -Raw -Encoding UTF8 $Manifest | ConvertFrom-Json

# ---- what each element looks like ----

# Alpha per state, 0..1. Ring is the 1px outline, fill the inside, glyph the mark.
$states = @{
    "normal"   = @{ ring = 0.24; fill = 0.00; glyph = 1.00 }
    "hover"    = @{ ring = 1.00; fill = 0.00; glyph = 1.00 }
    "active"   = @{ ring = 1.00; fill = 0.14; glyph = 1.00 }
    "inactive" = @{ ring = 0.24; fill = 0.00; glyph = 0.45 }
}

# Glyphs on a 13 x 9 button: "#" is a pixel. Row 0 and 8, column 0 and 12 are
# the ring, so a glyph lives in the 11 x 7 inside.
$glyphs = @{
    # Minimise: a bar (#86).
    "minimize" = @(
        ".............",
        ".............",
        ".............",
        ".............",
        "....#####....",
        ".............",
        ".............",
        ".............",
        "............."
    )
    # Windowshade (D60, #8): the arrow says which way the window goes. Down
    # while it is full (a click rolls it up), up while it is the strip.
    "shade" = @(
        ".............",
        ".............",
        ".............",
        "...#######...",
        "....#####....",
        ".....###.....",
        "......#......",
        ".............",
        "............."
    )
    "shade.on" = @(
        ".............",
        ".............",
        "......#......",
        ".....###.....",
        "....#####....",
        "...#######...",
        ".............",
        ".............",
        "............."
    )
    # 2x chrome, off: reads "2x", the size a click gives you (#47).
    "zoom" = @(
        ".............",
        ".............",
        "...###.......",
        ".....#.......",
        "...###.#.#...",
        "...#....#....",
        "...###.#.#...",
        ".............",
        "............."
    )
    # 2x chrome, on: reads "1x".
    "zoom.on" = @(
        ".............",
        ".............",
        "....#........",
        "...##........",
        "....#..#.#...",
        "....#...#....",
        "....#..#.#...",
        ".............",
        "............."
    )
}

# ---- collect every sprite the manifest draws, keyed by (sheet, rect) ----

$jobs = @{}   # "sheet|x,y,w,h" -> @{ sheet; rect; kind; state }

function Add-Job([string]$sheet, $rect, [string]$kind, [string]$state) {
    $key = "$sheet|$($rect -join ',')"
    if (-not $jobs.ContainsKey($key)) {
        $jobs[$key] = @{ sheet = $sheet; rect = @($rect); kind = $kind; state = $state }
    }
}

function Add-Element([string]$name, $e) {
    switch ($e.type) {
        "nineslice" { Add-Job $e.sprite.sheet $e.sprite.rect "frame" "normal" }
        "image" {
            Add-Job $e.sprite.sheet $e.sprite.rect $name "normal"
            if ($e.inactive) { Add-Job $e.inactive.sheet $e.inactive.rect $name "inactive" }
        }
        { $_ -eq "button" -or $_ -eq "toggle" } {
            foreach ($s in "sprite", "hover", "active", "inactive") {
                $ref = $e.$s
                if ($ref) {
                    $state = if ($s -eq "sprite") { "normal" } else { $s }
                    Add-Job $ref.sheet $ref.rect $name $state
                }
            }
            if ($e.on) {
                foreach ($s in "sprite", "hover", "active", "inactive") {
                    $ref = $e.on.$s
                    if ($ref) {
                        $state = if ($s -eq "sprite") { "normal" } else { $s }
                        Add-Job $ref.sheet $ref.rect "$name.on" $state
                    }
                }
            }
        }
    }
}

foreach ($w in "main", "equalizer", "playlist") {
    $win = $m.windows.$w
    foreach ($set in $win.elements, $win.shade.elements) {
        foreach ($p in $set.PSObject.Properties) { Add-Element $p.Name $p.Value }
    }
}

# ---- draw ----

# Not [Math]::Min/Max: PowerShell 5.1 picks the integer overload for
# `Min(1, 0.24)` and every fraction becomes 0. Found by probing the pixels.
function Alpha([double]$a) {
    if ($a -lt 0) { $a = 0 }
    if ($a -gt 1) { $a = 1 }
    [int][Math]::Round(255.0 * $a)
}

function Fill($g, [int]$x, [int]$y, [int]$w, [int]$h, [double]$a) {
    if ($w -le 0 -or $h -le 0) { return }
    $b = New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb((Alpha $a), 255, 255, 255))
    $g.FillRectangle($b, $x, $y, $w, $h)
    $b.Dispose()
}

# A ring one stroke thick around the rect, a fill inside it, both exact
# because compositing is SourceCopy: alpha is written, not blended.
function Draw-Box($g, [int]$x, [int]$y, [int]$w, [int]$h, [int]$s, [double]$ring, [double]$fill) {
    Fill $g $x $y $w $h $ring
    Fill $g ($x + $s) ($y + $s) ($w - 2 * $s) ($h - 2 * $s) $fill
}

function Draw-Glyph($g, [int]$x, [int]$y, [int]$s, [string[]]$rows, [double]$a) {
    for ($r = 0; $r -lt $rows.Count; $r++) {
        $row = $rows[$r]
        for ($c = 0; $c -lt $row.Length; $c++) {
            if ($row[$c] -eq "#") { Fill $g ($x + $c * $s) ($y + $r * $s) $s $s $a }
        }
    }
}

function Draw-Sprite($g, $job, [int]$s) {
    $x = [int]$job.rect[0] * $s; $y = [int]$job.rect[1] * $s
    $w = [int]$job.rect[2] * $s; $h = [int]$job.rect[3] * $s
    switch ($job.kind) {
        "frame"    { Draw-Box $g $x $y $w $h $s 0.30 0.00 }
        "titlebar" {
            $a = if ($job.state -eq "inactive") { 0.06 } else { 0.10 }
            Fill $g $x $y $w $h $a
        }
        default {
            $st = $states[$job.state]
            if (-not $st) { throw "no state '$($job.state)' for $($job.kind)" }
            $rows = $glyphs[$job.kind]
            if (-not $rows) { throw "no glyph for element '$($job.kind)'" }
            Draw-Box $g $x $y $w $h $s $st.ring $st.fill
            Draw-Glyph $g $x $y $s $rows $st.glyph
        }
    }
}

foreach ($sheetProp in $m.sheets.PSObject.Properties) {
    $sheetName = $sheetProp.Name
    $files = $sheetProp.Value
    $mine = @($jobs.Values | Where-Object { $_.sheet -eq $sheetName })
    if ($mine.Count -eq 0) { continue }
    # The sheet is as big as its sprites need, in 1x logical pixels.
    $W = 0; $H = 0
    foreach ($j in $mine) {
        $W = [Math]::Max($W, [int]$j.rect[0] + [int]$j.rect[2])
        $H = [Math]::Max($H, [int]$j.rect[1] + [int]$j.rect[3])
    }
    $scales = if ($files -is [string]) { @(@{ s = [int]$m.authoredScale; file = $files }) } else {
        @($files.PSObject.Properties | ForEach-Object { @{ s = [int]$_.Name; file = $_.Value } })
    }
    foreach ($sc in $scales) {
        $s = $sc.s
        $bmp = New-Object System.Drawing.Bitmap ($W * $s), ($H * $s), ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
        $g = [System.Drawing.Graphics]::FromImage($bmp)
        try {
            $g.CompositingMode = [System.Drawing.Drawing2D.CompositingMode]::SourceCopy
            $g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::Half
            $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::None
            $g.Clear([System.Drawing.Color]::Transparent)
            foreach ($j in $mine) { Draw-Sprite $g $j $s }
        } finally { $g.Dispose() }
        $path = Join-Path $Out $sc.file
        $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
        $bmp.Dispose()
        "$path  $($W * $s) x $($H * $s)  ($($mine.Count) sprites at ${s}x)"
    }
}
