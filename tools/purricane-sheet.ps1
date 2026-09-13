<#
    purricane-sheet.ps1 - draw the Purricane skin's sheet from its manifest (D132).

      powershell -NoProfile -ExecutionPolicy Bypass -File tools\purricane-sheet.ps1

    Purricane is `art: final`: the sheet is the picture, drawn in the
    designer's colours (design/screens/PurricaneMain, PurricanePlaylist) as
    design/tokens.json writes them - the six roles and the `art` block beside
    them. Nothing here names a colour; it names a token.

    Like tools\chrome-sheet.ps1, the rectangles come from the manifest, which
    is the one place geometry lives. This script knows what each named element
    looks like in each state, finds every rectangle the manifest cuts from the
    sheet, and draws each once, at 1x and 2x. The words on the pills are not
    here: they are the buttons' labels, drawn live in the theme's face.

    A pill is its rectangle less a pixel all round, and that pixel is where
    its glow goes, so neighbouring pills never draw over each other.

    Windows PowerShell 5.1, System.Drawing only. Re-run after editing the
    manifest's rectangles or a look below; commit the PNGs it writes.
#>
param(
    [string]$Manifest = "skins\purricane\manifest.json",
    [string]$Out = "skins\purricane"
)
$ErrorActionPreference = "Stop"
Set-Location (Split-Path -Parent $PSScriptRoot)
Add-Type -AssemblyName System.Drawing

$m = Get-Content -Raw -Encoding UTF8 $Manifest | ConvertFrom-Json
$tokens = Get-Content -Raw -Encoding UTF8 "design\tokens.json" | ConvertFrom-Json
$six = $tokens.themes.purricane.colors
$art = $tokens.themes.purricane.art

$sugar = $six.ground
$milk = $six.surface
$ink = $six.text
$floss = $six.accent
$peri = $six.alert
$butter = $six.warn
$blush = $art.blush
$blushHover = $art.blushHover
$mint = $art.mint
$plum = $art.plum
$rose = $art.rose

# ---- colour ----

function Rgb([string]$hex) {
    $n = [Convert]::ToInt32($hex.Substring(1, 6), 16)
    return @((($n -shr 16) -band 255), (($n -shr 8) -band 255), ($n -band 255))
}

function Col([string]$hex, [double]$alpha = 1.0) {
    $c = Rgb $hex
    $a = [int][Math]::Round(255.0 * $alpha)
    if ($a -lt 0) { $a = 0 }
    if ($a -gt 255) { $a = 255 }
    return [System.Drawing.Color]::FromArgb($a, $c[0], $c[1], $c[2])
}

# A colour `t` of the way from one to another, as a hex again.
function Mix([string]$from, [string]$to, [double]$t) {
    $x = Rgb $from
    $y = Rgb $to
    $o = for ($i = 0; $i -lt 3; $i++) { [int][Math]::Round($x[$i] + ($y[$i] - $x[$i]) * $t) }
    return "#{0:X2}{1:X2}{2:X2}" -f $o[0], $o[1], $o[2]
}

# ---- shapes, in logical pixels; the sheet's scale is on the transform ----

function F([double]$v) { return [single]$v }

function RoundRect([double]$x, [double]$y, [double]$w, [double]$hh, [double]$r) {
    $p = New-Object System.Drawing.Drawing2D.GraphicsPath
    $half = $w
    if ($hh -lt $half) { $half = $hh }
    $half = $half / 2.0
    if ($r -gt $half) { $r = $half }
    if ($r -le 0.01) {
        $p.AddRectangle((New-Object System.Drawing.RectangleF (F $x), (F $y), (F $w), (F $hh)))
        return $p
    }
    $d = 2.0 * $r
    $p.AddArc((F $x), (F $y), (F $d), (F $d), (F 180), (F 90))
    $p.AddArc((F ($x + $w - $d)), (F $y), (F $d), (F $d), (F 270), (F 90))
    $p.AddArc((F ($x + $w - $d)), (F ($y + $hh - $d)), (F $d), (F $d), (F 0), (F 90))
    $p.AddArc((F $x), (F ($y + $hh - $d)), (F $d), (F $d), (F 90), (F 90))
    $p.CloseFigure()
    return $p
}

function FillPath($g, $path, [string]$hex, [double]$alpha = 1.0) {
    $b = New-Object System.Drawing.SolidBrush (Col $hex $alpha)
    $g.FillPath($b, $path)
    $b.Dispose()
}

# A one-pixel ring just inside a rounded rectangle.
function Ring($g, [double]$x, [double]$y, [double]$w, [double]$hh, [double]$r, [string]$hex, [double]$alpha) {
    $p = RoundRect ($x + 0.5) ($y + 0.5) ($w - 1) ($hh - 1) ($r - 0.5)
    $pen = New-Object System.Drawing.Pen (Col $hex $alpha), (F 1)
    $g.DrawPath($pen, $p)
    $pen.Dispose()
    $p.Dispose()
}

# A soft halo round a rounded rectangle, `spread` pixels out: rings of the
# colour, faintest outermost, stacked so it deepens toward the edge.
function Glow($g, [double]$x, [double]$y, [double]$w, [double]$hh, [double]$r, [string]$hex, [double]$alpha, [double]$spread) {
    $steps = [int][Math]::Ceiling($spread * 4)
    for ($i = $steps; $i -ge 1; $i--) {
        $d = $spread * $i / $steps
        $k = 1.0 - ($i / ($steps + 1.0))
        $p = RoundRect ($x - $d) ($y - $d) ($w + 2 * $d) ($hh + 2 * $d) ($r + $d)
        FillPath $g $p $hex ($alpha * $k * $k * 2.4 / $steps)
        $p.Dispose()
    }
}

function Poly($g, [double[]]$xy, [string]$hex, [double]$alpha = 1.0) {
    $pts = New-Object 'System.Drawing.PointF[]' ($xy.Length / 2)
    for ($i = 0; $i -lt $pts.Length; $i++) {
        $pts[$i] = New-Object System.Drawing.PointF (F $xy[2 * $i]), (F $xy[2 * $i + 1])
    }
    $b = New-Object System.Drawing.SolidBrush (Col $hex $alpha)
    $g.FillPolygon($b, $pts)
    $b.Dispose()
}

function Box($g, [double]$x, [double]$y, [double]$w, [double]$hh, [string]$hex, [double]$alpha = 1.0) {
    $b = New-Object System.Drawing.SolidBrush (Col $hex $alpha)
    $g.FillRectangle($b, (F $x), (F $y), (F $w), (F $hh))
    $b.Dispose()
}

function Line($g, [double]$x1, [double]$y1, [double]$x2, [double]$y2, [string]$hex, [double]$width = 1.0) {
    $pen = New-Object System.Drawing.Pen (Col $hex), (F $width)
    $pen.StartCap = [System.Drawing.Drawing2D.LineCap]::Round
    $pen.EndCap = [System.Drawing.Drawing2D.LineCap]::Round
    $g.DrawLine($pen, (F $x1), (F $y1), (F $x2), (F $y2))
    $pen.Dispose()
}

function Gradient($g, [double]$x, [double]$y, [double]$w, [double]$hh, [string]$from, [string]$to, [bool]$across) {
    $rect = New-Object System.Drawing.RectangleF (F $x), (F $y), (F $w), (F $hh)
    $mode = if ($across) { [System.Drawing.Drawing2D.LinearGradientMode]::Horizontal } else { [System.Drawing.Drawing2D.LinearGradientMode]::Vertical }
    # A hair larger than the box, so GDI+'s wrap never paints the far colour
    # back into the first row.
    $grow = New-Object System.Drawing.RectangleF (F ($x - 0.5)), (F ($y - 0.5)), (F ($w + 1)), (F ($hh + 1))
    $b = New-Object System.Drawing.Drawing2D.LinearGradientBrush $grow, (Col $from), (Col $to), $mode
    $g.FillRectangle($b, $rect)
    $b.Dispose()
}

# ---- what each element looks like ----

# A pill: fill, ring and glow by state. `on` states are lit.
$pills = @{
    # CALM and the EQ's switch: milk and a mint ring, mint when on.
    "mint"   = @{ fill = $milk; ring = $mint; ringA = 0.9; hover = $mint; hoverA = 0.22; lit = $mint; glow = $mint }
    # 6 SEG, 8 SEG, EQ and PL: milk and a periwinkle ring, periwinkle when on.
    "peri"   = @{ fill = $milk; ring = $peri; ringA = 0.7; hover = $peri; hoverA = 0.2; lit = $peri; glow = $peri }
    # The playlist's bar: milk and a floss ring, mint when a switch is on.
    "floss"  = @{ fill = $milk; ring = $floss; ringA = 0.8; hover = $blush; hoverA = 1.0; lit = $mint; glow = $mint }
    # The preset menu's button: periwinkle, washed while the menu is open.
    "preset" = @{ fill = $milk; ring = $peri; ringA = 0.7; hover = $peri; hoverA = 0.15; lit = $null; glow = $null }
    # The transport: blush and a floss ring, floss while it is the state.
    "transport" = @{ fill = $blush; ring = $floss; ringA = 0.8; hover = $blushHover; hoverA = 1.0; lit = $floss; glow = $floss }
}

function Draw-Pill($g, [double]$w, [double]$hh, $look, [string]$state) {
    $x = 1.0; $y = 1.0; $pw = $w - 2; $ph = $hh - 2; $r = $ph / 2.0
    $on = $state.StartsWith("on.")
    $s = $state -replace '^on\.', ''
    if ($on -and $look.lit) {
        $a = if ($s -eq "h") { 1.0 } else { 0.8 }
        Glow $g $x $y $pw $ph $r $look.glow $a 1.0
        $fill = if ($s -eq "a") { Mix $look.lit $plum 0.15 } else { $look.lit }
        $p = RoundRect $x $y $pw $ph $r; FillPath $g $p $fill; $p.Dispose()
        return
    }
    $p = RoundRect $x $y $pw $ph $r
    FillPath $g $p $look.fill
    if ($on) {
        # Lit without a lit colour (the preset menu open): a firmer wash.
        FillPath $g $p $look.hover ($look.hoverA * 1.6)
    } elseif ($s -eq "h") {
        FillPath $g $p $look.hover $look.hoverA
    } elseif ($s -eq "a") {
        FillPath $g $p $look.ring 0.35
    }
    $p.Dispose()
    Ring $g $x $y $pw $ph $r $look.ring $(if ($s -eq "n" -and -not $on) { $look.ringA } else { 1.0 })
}

# The transport's marks, centred in an 18 x 17 box, in plum or, lit, sugar.
function Draw-Mark($g, [string]$name, [string]$hex) {
    switch ($name) {
        "play"  { Poly $g @(7.2, 5.6, 7.2, 11.4, 12.0, 8.5) $hex }
        "pause" { Box $g 6.6 5.8 2 5.4 $hex; Box $g 9.4 5.8 2 5.4 $hex }
        "stop"  { Box $g 6.5 6.0 5 5 $hex }
        "prev"  { Poly $g @(9.0, 6.0, 9.0, 11.0, 5.6, 8.5) $hex; Poly $g @(12.4, 6.0, 12.4, 11.0, 9.0, 8.5) $hex }
        "next"  { Poly $g @(5.6, 6.0, 5.6, 11.0, 9.0, 8.5) $hex; Poly $g @(9.0, 6.0, 9.0, 11.0, 12.4, 8.5) $hex }
    }
}

# The title bar's round buttons: a 10 px circle in a 12 x 13 box, its glow in
# the rest.
$circles = @{
    "minimize" = @{ fill = $butter; mark = $ink }
    "close"    = @{ fill = $floss; mark = $sugar }
    "shade"    = @{ fill = $peri; mark = $sugar }
    "zoom"     = @{ fill = $mint; mark = $ink }
}

function Draw-Circle($g, [string]$name, [string]$state) {
    $look = $circles[$name]
    $on = $state.StartsWith("on.")
    $s = $state -replace '^on\.', ''
    $a = if ($s -eq "h") { 1.0 } else { 0.7 }
    Glow $g 1 1.5 10 10 5 $look.fill $a 1.0
    $fill = if ($s -eq "a") { Mix $look.fill $plum 0.18 } else { $look.fill }
    $p = RoundRect 1 1.5 10 10 5; FillPath $g $p $fill; $p.Dispose()
    $c = $look.mark
    switch ($name) {
        "minimize" { Line $g 4.2 6.5 7.8 6.5 $c }
        "close"    { Line $g 4.4 4.9 7.6 8.1 $c; Line $g 7.6 4.9 4.4 8.1 $c }
        # The strip's arrow points the way the window will go (D60).
        "shade"    { if ($on) { Poly $g @(3.8, 7.6, 8.2, 7.6, 6.0, 5.0) $c } else { Poly $g @(3.8, 5.4, 8.2, 5.4, 6.0, 8.0) $c } }
        # Double size: an outlined square, filled while doubled.
        "zoom"     {
            if ($on) { Box $g 4 4.5 4 4 $c } else {
                $pen = New-Object System.Drawing.Pen (Col $c), (F 1)
                $g.DrawRectangle($pen, (F 4.5), (F 5.0), (F 3), (F 3))
                $pen.Dispose()
            }
        }
    }
}

# A white well: rounded as far as its box allows up to 8, ringed.
$wells = @{
    "stripWell"   = @{ ring = $peri; a = 0.75 }
    "listWell"    = @{ ring = $peri; a = 0.6 }
    "eqCurveWell" = @{ ring = $peri; a = 0.6 }
    "seekWell"    = @{ ring = $floss; a = 0.7 }
    "volWell"     = @{ ring = $mint; a = 0.8 }
}

$dots = @{ "main" = $floss; "equalizer" = $butter; "playlist" = $mint }

function Draw-Job($g, $job) {
    $w = [double]$job.w
    $hh = [double]$job.h
    $name = $job.name
    $state = $job.state
    switch -regex ($name) {
        '^frame$' {
            Box $g 0 0 $w $hh $sugar
            Box $g 0 0 $w 1 $floss; Box $g 0 ($hh - 1) $w 1 $floss
            Box $g 0 0 1 $hh $floss; Box $g ($w - 1) 0 1 $hh $floss
        }
        '^titlebar$' {
            # Blush into sugar, and under a full title bar a floss line.
            if ($state -eq "inactive") { Box $g 0 0 $w $hh $sugar } else { Gradient $g 0 0 $w $hh $blush $sugar $false }
            if ($hh -ge 16) { Box $g 0 ($hh - 1) $w 1 $floss $(if ($state -eq "inactive") { 0.4 } else { 0.8 }) }
        }
        '^bottomBar$' { Box $g 0 0 $w 1 $floss 0.6 }
        '^dot$' {
            $c = $dots[$job.window]
            Glow $g 3 3.5 4 4 2 $c 0.9 2.0
            $p = RoundRect 3 3.5 4 4 2; FillPath $g $p $c; $p.Dispose()
        }
        '^visRing$' {
            # Round the badge: a glow outside the circle and a floss ring at
            # its edge, and nothing inside, where the kaleidoscope is.
            $cx = 21.0; $cy = 21.0; $r = 17.0
            $steps = 16
            for ($i = $steps; $i -ge 1; $i--) {
                $d = 4.0 * $i / $steps
                $k = 1.0 - ($i / ($steps + 1.0))
                $p = New-Object System.Drawing.Drawing2D.GraphicsPath
                $p.AddEllipse((F ($cx - $r - $d)), (F ($cy - $r - $d)), (F (2 * ($r + $d))), (F (2 * ($r + $d))))
                $p.AddEllipse((F ($cx - $r)), (F ($cy - $r)), (F (2 * $r)), (F (2 * $r)))
                FillPath $g $p $floss (0.55 * $k * $k * 2.4 / $steps)
                $p.Dispose()
            }
            $pen = New-Object System.Drawing.Pen (Col $floss 0.9), (F 1)
            $g.DrawEllipse($pen, (F ($cx - $r + 0.5)), (F ($cy - $r + 0.5)), (F (2 * $r - 1)), (F (2 * $r - 1)))
            $pen.Dispose()
        }
        'Well$' {
            $look = $wells[$name]
            $r = [Math]::Floor([double]([Math]::Min([int]$w, [int]$hh)) / 2.0)
            if ($r -gt 8) { $r = 8.0 }
            $p = RoundRect 0 0 $w $hh $r; FillPath $g $p $milk; $p.Dispose()
            Ring $g 0 0 $w $hh $r $look.ring $look.a
        }
        '^(minimize|close|shade|zoom)$' { Draw-Circle $g $name $state }
        '^(calm|eqOnButton)$' { Draw-Pill $g $w $hh $pills["mint"] $state }
        '^(segments6|segments8|eq|pl)$' { Draw-Pill $g $w $hh $pills["peri"] $state }
        '^eqPresetButton$' { Draw-Pill $g $w $hh $pills["preset"] $state }
        '^(urlButton|removeButton|libraryButton|selectButton|shuffleButton|repeatButton)$' { Draw-Pill $g $w $hh $pills["floss"] $state }
        '^addButton$' {
            # ADD is the mint one, lit all the time, as the designer drew it.
            Draw-Pill $g $w $hh $pills["floss"] $("on." + $state)
        }
        '^eqClipLamp$' {
            if ($state -eq "on.n") {
                Glow $g 1 1 ($w - 2) ($hh - 2) (($hh - 2) / 2) $rose 0.8 1.0
                $p = RoundRect 1 1 ($w - 2) ($hh - 2) (($hh - 2) / 2); FillPath $g $p $rose; $p.Dispose()
            } else {
                $p = RoundRect 1 1 ($w - 2) ($hh - 2) (($hh - 2) / 2); FillPath $g $p $milk; $p.Dispose()
                Ring $g 1 1 ($w - 2) ($hh - 2) (($hh - 2) / 2) $floss 0.5
            }
        }
        '^(prev|play|pause|stop|next)$' {
            Draw-Pill $g $w $hh $pills["transport"] $state
            $lit = $state.StartsWith("on.")
            Draw-Mark $g $name $(if ($lit) { $sugar } else { $plum })
        }
        '^seek$' {
            if ($state -eq "fill") { Gradient $g 0 0 $w $hh $butter $floss $true }
            if ($state -eq "thumb") {
                Glow $g 2 2 6 12 3 $mint 0.95 2.0
                $p = RoundRect 2 2 6 12 3; FillPath $g $p $mint; $p.Dispose()
            }
        }
        '^volume$' { if ($state -eq "fill") { Box $g 0 0 $w $hh $mint } }
        '^eq(Pre|Band\d+)$' {
            switch ($state) {
                "track" {
                    $p = RoundRect 5.5 1 6 ($hh - 2) 3; FillPath $g $p $milk; $p.Dispose()
                    Ring $g 5.5 1 6 ($hh - 2) 3 $peri 0.6
                }
                "fill" { Box $g 6.5 0 4 $hh $floss }
                "thumb" {
                    Glow $g 2 2 10 6 3 $mint 0.95 2.0
                    $p = RoundRect 2 2 10 6 3; FillPath $g $p $mint; $p.Dispose()
                }
            }
        }
        default { throw "purricane-sheet: nothing knows how to draw '$name' ($state)" }
    }
}

# ---- collect every sprite the manifest draws, keyed by its rectangle ----

$jobs = [ordered]@{}

# Which elements draw alike, so may share a rectangle: the EQ's eleven
# sliders, the playlist's pills, 6 SEG and 8 SEG, EQ and PL.
function Look-Of([string]$window, [string]$name) {
    switch -regex ($name) {
        '^eq(Pre|Band\d+)$' { return "eqslider" }
        '^(segments6|segments8|eq|pl)$' { return "peri" }
        '^(urlButton|removeButton|libraryButton|selectButton|shuffleButton|repeatButton)$' { return "floss" }
        '^dot$' { return "dot-$window" }
        default { return $name }
    }
}

function Add-Job([string]$window, [string]$name, [string]$state, $ref) {
    if ($null -eq $ref) { return }
    if ($ref.sheet -ne "chrome") { return }
    $r = $ref.rect
    $key = "{0},{1},{2},{3}" -f $r[0], $r[1], $r[2], $r[3]
    $look = Look-Of $window $name
    if ($jobs.Contains($key)) {
        $was = $jobs[$key]
        if ($was.look -ne $look -or $was.state -ne $state) {
            throw "purricane-sheet: $key is both $($was.name) ($($was.state)) and $name ($state)"
        }
        return
    }
    $jobs[$key] = @{ window = $window; name = $name; look = $look; state = $state; x = [int]$r[0]; y = [int]$r[1]; w = [int]$r[2]; h = [int]$r[3] }
}

function Add-Element([string]$window, [string]$name, $e) {
    switch ($e.type) {
        { $_ -in "nineslice", "image" } {
            Add-Job $window $name "n" $e.sprite
            Add-Job $window $name "inactive" $e.inactive
        }
        { $_ -in "button", "toggle" } {
            Add-Job $window $name "n" $e.sprite
            Add-Job $window $name "h" $e.hover
            Add-Job $window $name "a" $e.active
            if ($e.on) {
                Add-Job $window $name "on.n" $e.on.sprite
                Add-Job $window $name "on.h" $e.on.hover
                Add-Job $window $name "on.a" $e.on.active
            }
        }
        "slider" {
            Add-Job $window $name "track" $e.track
            Add-Job $window $name "fill" $e.fill
            Add-Job $window $name "thumb" $e.thumb
        }
    }
}

foreach ($wp in $m.windows.PSObject.Properties) {
    foreach ($set in @($wp.Value.elements, $wp.Value.shade.elements)) {
        foreach ($ep in $set.PSObject.Properties) { Add-Element $wp.Name $ep.Name $ep.Value }
    }
}

$sheetW = 0; $sheetH = 0
foreach ($j in $jobs.Values) {
    if ($j.x + $j.w -gt $sheetW) { $sheetW = $j.x + $j.w }
    if ($j.y + $j.h -gt $sheetH) { $sheetH = $j.y + $j.h }
}

# ---- draw, at every scale the manifest lists ----

foreach ($sp in $m.sheets.chrome.PSObject.Properties) {
    if ($sp.Name -notin "1", "2") { continue }
    $s = [int]$sp.Name
    $bmp = New-Object System.Drawing.Bitmap ($sheetW * $s), ($sheetH * $s), ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
    $g.CompositingQuality = [System.Drawing.Drawing2D.CompositingQuality]::HighQuality
    $g.Clear([System.Drawing.Color]::Transparent)
    foreach ($j in $jobs.Values) {
        $g.ResetTransform()
        $g.ResetClip()
        $g.ScaleTransform((F $s), (F $s))
        $g.TranslateTransform((F $j.x), (F $j.y))
        $g.SetClip((New-Object System.Drawing.RectangleF (F 0), (F 0), (F $j.w), (F $j.h)))
        Draw-Job $g $j
    }
    $g.Dispose()
    $path = Join-Path $Out $sp.Value
    $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    "{0}  {1} x {2}, {3} sprites" -f $path, ($sheetW * $s), ($sheetH * $s), $jobs.Count
}
