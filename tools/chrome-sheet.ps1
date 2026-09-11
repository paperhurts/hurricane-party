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

    The look is the CSS chrome that shipped in v0.4b, transcribed. Two sprites
    are shared by many elements and drawn at full alpha, because the element
    carries the strength as `opacity` (D93): `ring`, a one-pixel border, is the
    window frame at 0.3 and a control's edge at 0.14; `solid` is every well.

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

# A latched button (playing, paused, stopped) fills at 0.14 and carries a soft
# bloom around its glyph, baked into the alpha rather than added by a filter --
# the same reason the analyser's glow lives in its ramp art (D73).
$bloom = @(@{ r = 3; a = 0.16 }, @{ r = 2; a = 0.24 }, @{ r = 1; a = 0.38 })

# Exactly the transport's lit states. Named, not matched on ".on": a
# title-bar toggle's "on" is its other glyph (1x rather than 2x, the up arrow
# rather than the down), not a lit state, and matching the suffix baked this
# bloom into them and made the 1x button look fuzzy (#117).
$latchedKinds = @("play.on", "pause.on", "stop.on")

# Glyphs. Each string grid must be exactly the sprite's logical size: 13 x 9 on
# a title-bar button, 17 x 14 on a transport button. Row 0 and the last row,
# column 0 and the last column, are the ring.
$glyphs = @{
    # ---- title bar, 13 x 9 ----
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
    # Close: Main only, and it is the app's exit (D63).
    "close" = @(
        ".............",
        ".............",
        "....#...#....",
        ".....#.#.....",
        "......#......",
        ".....#.#.....",
        "....#...#....",
        ".............",
        "............."
    )

    # ---- transport, 17 x 14 ----
    "prev" = @(
        ".................",
        ".................",
        ".................",
        ".......#....#....",
        "......##...##....",
        ".....###..###....",
        "....####.####....",
        "....####.####....",
        ".....###..###....",
        "......##...##....",
        ".......#....#....",
        ".................",
        ".................",
        "................."
    )
    "next" = @(
        ".................",
        ".................",
        ".................",
        "....#....#.......",
        "....##...##......",
        "....###..###.....",
        "....####.####....",
        "....####.####....",
        "....###..###.....",
        "....##...##......",
        "....#....#.......",
        ".................",
        ".................",
        "................."
    )
    "play" = @(
        ".................",
        ".................",
        "......#..........",
        "......##.........",
        "......###........",
        "......####.......",
        "......#####......",
        "......#####......",
        "......####.......",
        "......###........",
        "......##.........",
        "......#..........",
        ".................",
        "................."
    )
    "pause" = @(
        ".................",
        ".................",
        ".................",
        "......##..##.....",
        "......##..##.....",
        "......##..##.....",
        "......##..##.....",
        "......##..##.....",
        "......##..##.....",
        "......##..##.....",
        "......##..##.....",
        ".................",
        ".................",
        "................."
    )
    "stop" = @(
        ".................",
        ".................",
        ".................",
        ".................",
        "......#####......",
        "......#####......",
        "......#####......",
        "......#####......",
        "......#####......",
        ".................",
        ".................",
        ".................",
        ".................",
        "................."
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

function Add-States($e, [string]$kind) {
    foreach ($s in "sprite", "hover", "active", "inactive") {
        $ref = $e.$s
        if ($ref) {
            $state = if ($s -eq "sprite") { "normal" } else { $s }
            Add-Job $ref.sheet $ref.rect $kind $state
        }
    }
}

function Add-Element([string]$name, $e) {
    switch ($e.type) {
        "nineslice" { Add-Job $e.sprite.sheet $e.sprite.rect $name "normal" }
        "image" {
            Add-Job $e.sprite.sheet $e.sprite.rect $name "normal"
            if ($e.inactive) { Add-Job $e.inactive.sheet $e.inactive.rect $name "inactive" }
        }
        { $_ -eq "button" -or $_ -eq "toggle" } {
            Add-States $e $name
            if ($e.on) { Add-States $e.on "$name.on" }
        }
        "slider" {
            foreach ($part in "track", "fill", "thumb") {
                $ref = $e.$part
                if ($ref) { Add-Job $ref.sheet $ref.rect "$name.$part" "normal" }
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

# An element's name says what to draw. Two suffixes are conventions rather than
# glyphs, because many elements share one sprite: anything named *Frame (and
# the window's own `frame`) is the ring, anything named *Well is the solid.
function Resolve-Recipe([string]$name) {
    if ($name -eq "frame" -or $name.EndsWith("Frame")) { return "ring" }
    if ($name.EndsWith("Well")) { return "solid" }
    return $name
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

# The bloom, stamped widest-and-dimmest first so a later, brighter pass wins
# under SourceCopy. Clipped to the box interior so the ring survives.
function Draw-Bloom($g, [int]$x, [int]$y, [int]$w, [int]$h, [int]$s, [string[]]$rows) {
    foreach ($ring in $bloom) {
        $r = $ring.r
        for ($gr = 0; $gr -lt $rows.Count; $gr++) {
            $row = $rows[$gr]
            for ($gc = 0; $gc -lt $row.Length; $gc++) {
                if ($row[$gc] -ne "#") { continue }
                $px = $x + ($gc - $r) * $s
                $py = $y + ($gr - $r) * $s
                $pw = (2 * $r + 1) * $s
                $ph = (2 * $r + 1) * $s
                # Clamp inside the ring.
                $lo = $x + $s; $to = $y + $s
                $hi = $x + $w - $s; $bo = $y + $h - $s
                if ($px -lt $lo) { $pw -= ($lo - $px); $px = $lo }
                if ($py -lt $to) { $ph -= ($to - $py); $py = $to }
                if ($px + $pw -gt $hi) { $pw = $hi - $px }
                if ($py + $ph -gt $bo) { $ph = $bo - $py }
                Fill $g $px $py $pw $ph $ring.a
            }
        }
    }
}

function Draw-Sprite($g, $job, [int]$s) {
    $x = [int]$job.rect[0] * $s; $y = [int]$job.rect[1] * $s
    $w = [int]$job.rect[2] * $s; $h = [int]$job.rect[3] * $s
    $recipe = Resolve-Recipe $job.kind
    switch ($recipe) {
        # A one-pixel border at full alpha. The element says how strong it is.
        "ring"  { Draw-Box $g $x $y $w $h $s 1.00 0.00 }
        # A plain fill, stretched by the renderer to whatever box wants it.
        "solid" { Fill $g $x $y $w $h 1.00 }
        "titlebar" {
            $a = if ($job.state -eq "inactive") { 0.06 } else { 0.10 }
            Fill $g $x $y $w $h $a
        }
        # The seek bar's progress: the CSS gradient, as alpha.
        "seek.fill" {
            for ($i = 0; $i -lt $w; $i++) {
                $t = if ($w -le 1) { 1.0 } else { $i / ($w - 1.0) }
                Fill $g ($x + $i) $y 1 $h (0.25 + 0.50 * $t)
            }
        }
        # The volume level: brightest along its centre line, so a bar five
        # pixels tall still reads as lit rather than as a block.
        "volume.fill" {
            for ($j = 0; $j -lt $h; $j++) {
                $t = if ($h -le 1) { 0.0 } else { [Math]::Abs($j / ($h - 1.0) - 0.5) * 2.0 }
                # Six arguments, always: PowerShell binds positionally and
                # silently, so a missing height takes the alpha's place and
                # the sprite comes out empty rather than wrong.
                Fill $g $x ($y + $j) $w 1 (1.0 - 0.45 * $t)
            }
        }
        # The seek thumb: a hard core with its halo baked around it, never a
        # filter (D73). Three logical pixels wide, and the falloff is the rest.
        "seek.thumb" {
            $cx = ($w - 1) / 2.0
            $core = 1.5 * $s
            for ($i = 0; $i -lt $w; $i++) {
                $d = [Math]::Abs($i - $cx)
                $a = if ($d -le $core) { 1.0 } else { 0.55 * [Math]::Exp(-([double]($d - $core)) / (1.4 * $s)) }
                for ($j = 0; $j -lt $h; $j++) {
                    # Soften the two ends so the thumb does not read as a bar.
                    $ty = [Math]::Abs($j / ($h - 1.0) - 0.5) * 2.0
                    $edge = if ($ty -gt 0.82) { 0.45 } else { 1.0 }
                    Fill $g ($x + $i) ($y + $j) 1 1 ($a * $edge)
                }
            }
        }
        default {
            $st = $states[$job.state]
            if (-not $st) { throw "no state '$($job.state)' for $($job.kind)" }
            $base = $recipe -replace '\.on$', ''
            $rows = $glyphs[$recipe]
            if (-not $rows) { $rows = $glyphs[$base] }
            if (-not $rows) { throw "no glyph for element '$($job.kind)'" }
            if ($rows.Count -ne [int]$job.rect[3] -or $rows[0].Length -ne [int]$job.rect[2]) {
                throw "glyph for '$($job.kind)' is $($rows[0].Length) x $($rows.Count), sprite is $($job.rect[2]) x $($job.rect[3])"
            }
            $latched = $latchedKinds -contains $recipe
            # `if` is a statement, not an argument: PS 5.1 rejects it inline.
            $boxFill = $st.fill
            if ($latched) { $boxFill = 0.14 }
            Draw-Box $g $x $y $w $h $s $st.ring $boxFill
            if ($latched) { Draw-Bloom $g $x $y $w $h $s $rows }
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
