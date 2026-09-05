<#
    keyout.ps1 - turn a flat-background artwork into a transparent, square PNG
    ready for `pnpm tauri icon`.

      powershell -NoProfile -ExecutionPolicy Bypass -File tools\keyout.ps1 -In art.jpg -Out icon-1024.png

    Flood-fills the background in from the image border, so anything enclosed
    by an outline (a grey boombox on a grey background, say) survives. A pixel
    joins the background when it is within -SeedTol of the corner colour and
    within -StepTol of the neighbour it was reached from; the second bound is
    what lets a gentle gradient in and keeps a hard outline out. Edge pixels
    next to the background get partial alpha by their distance from it, which
    is a one-pixel feather. Then crops to the art with a margin, pads to a
    square, and resamples to -Size.

    Windows PowerShell 5.1, System.Drawing only. No ImageMagick needed.
#>
param(
    [Parameter(Mandatory = $true)][string]$In,
    [Parameter(Mandatory = $true)][string]$Out,
    [int]$Size = 1024,
    [int]$SeedTol = 160,
    [int]$StepTol = 14,
    [int]$Feather = 90,
    [double]$Margin = 0.04
)

Add-Type -AssemblyName System.Drawing
Add-Type -ReferencedAssemblies System.Drawing @"
using System;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;
using System.Collections.Generic;
public static class KeyOut {
    static int Dist(int a, int b) {
        return Math.Abs(((a >> 16) & 255) - ((b >> 16) & 255))
             + Math.Abs(((a >> 8) & 255) - ((b >> 8) & 255))
             + Math.Abs((a & 255) - (b & 255));
    }
    public static Bitmap Run(string path, int seedTol, int stepTol, int feather) {
        var src = new Bitmap(path);
        int w = src.Width, h = src.Height, n = w * h;
        var bmp = new Bitmap(w, h, PixelFormat.Format32bppArgb);
        using (var g = Graphics.FromImage(bmp)) g.DrawImage(src, 0, 0, w, h);
        src.Dispose();
        var rect = new Rectangle(0, 0, w, h);
        var data = bmp.LockBits(rect, ImageLockMode.ReadWrite, PixelFormat.Format32bppArgb);
        int[] px = new int[n];
        Marshal.Copy(data.Scan0, px, 0, n);
        // Grow the background in from every border pixel by neighbour
        // similarity alone: a gradient passes one gentle step at a time, an
        // outline is a cliff. `seedTol` is only a sanity bound against the
        // corner colour so a run can never wander into something saturated.
        int seed = px[0] & 0xFFFFFF;
        var bg = new bool[n];
        var q = new Queue<int>();
        for (int x = 0; x < w; x++) { q.Enqueue(x); q.Enqueue((h - 1) * w + x); }
        for (int y = 0; y < h; y++) { q.Enqueue(y * w); q.Enqueue(y * w + w - 1); }
        while (q.Count > 0) {
            int i = q.Dequeue();
            if (bg[i]) continue;
            int c = px[i] & 0xFFFFFF;
            if (Dist(c, seed) > seedTol) continue;
            bg[i] = true;
            int x = i % w, y = i / w;
            if (x > 0 && !bg[i - 1] && Dist(px[i - 1] & 0xFFFFFF, c) <= stepTol) q.Enqueue(i - 1);
            if (x < w - 1 && !bg[i + 1] && Dist(px[i + 1] & 0xFFFFFF, c) <= stepTol) q.Enqueue(i + 1);
            if (y > 0 && !bg[i - w] && Dist(px[i - w] & 0xFFFFFF, c) <= stepTol) q.Enqueue(i - w);
            if (y < h - 1 && !bg[i + w] && Dist(px[i + w] & 0xFFFFFF, c) <= stepTol) q.Enqueue(i + w);
        }
        // Feather: a pixel next to the background gets alpha by how far it is
        // from that neighbour's colour, so a soft edge fades rather than halos.
        int[] orig = (int[])px.Clone();
        for (int i = 0; i < n; i++) {
            if (bg[i]) { px[i] = 0; continue; }
            int x = i % w, y = i / w;
            int nb = -1;
            if (x > 0 && bg[i - 1]) nb = i - 1;
            else if (x < w - 1 && bg[i + 1]) nb = i + 1;
            else if (y > 0 && bg[i - w]) nb = i - w;
            else if (y < h - 1 && bg[i + w]) nb = i + w;
            if (nb >= 0) {
                int a = Math.Min(255, Dist(orig[i] & 0xFFFFFF, orig[nb] & 0xFFFFFF) * 255 / feather);
                px[i] = (a << 24) | (px[i] & 0xFFFFFF);
            }
        }
        Marshal.Copy(px, 0, data.Scan0, n);
        bmp.UnlockBits(data);
        return bmp;
    }
    public static Rectangle Bounds(Bitmap bmp) {
        int w = bmp.Width, h = bmp.Height;
        var data = bmp.LockBits(new Rectangle(0, 0, w, h), ImageLockMode.ReadOnly, PixelFormat.Format32bppArgb);
        int[] px = new int[w * h];
        Marshal.Copy(data.Scan0, px, 0, px.Length);
        bmp.UnlockBits(data);
        int x0 = w, y0 = h, x1 = -1, y1 = -1;
        for (int i = 0; i < px.Length; i++) {
            if (((px[i] >> 24) & 255) < 8) continue;
            int x = i % w, y = i / w;
            if (x < x0) x0 = x; if (x > x1) x1 = x; if (y < y0) y0 = y; if (y > y1) y1 = y;
        }
        return x1 < 0 ? new Rectangle(0, 0, w, h) : new Rectangle(x0, y0, x1 - x0 + 1, y1 - y0 + 1);
    }
}
"@

$src = [KeyOut]::Run((Resolve-Path $In).Path, $SeedTol, $StepTol, $Feather)
$b = [KeyOut]::Bounds($src)
"art bounds: x=$($b.X) y=$($b.Y) w=$($b.Width) h=$($b.Height) of $($src.Width)x$($src.Height)"

# Square canvas around the art with a margin, art centred.
$side = [Math]::Max($b.Width, $b.Height)
$side = [int][Math]::Ceiling($side * (1 + 2 * $Margin))
$cx = $b.X + $b.Width / 2.0; $cy = $b.Y + $b.Height / 2.0
$sq = New-Object System.Drawing.Bitmap $side, $side, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
$g = [System.Drawing.Graphics]::FromImage($sq)
$g.Clear([System.Drawing.Color]::Transparent)
$g.DrawImage($src, [int]($side / 2 - $cx), [int]($side / 2 - $cy))
$g.Dispose()

# Resample to the requested size with a good filter. (Not `$out`: PowerShell
# variable names are case-insensitive and that is the -Out parameter.)
$result = New-Object System.Drawing.Bitmap $Size, $Size, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
$g = [System.Drawing.Graphics]::FromImage($result)
$g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
$g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
$g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
$g.Clear([System.Drawing.Color]::Transparent)
$g.DrawImage($sq, 0, 0, $Size, $Size)
$g.Dispose()

$dir = Split-Path -Parent $Out
if ($dir -and -not (Test-Path $dir)) { New-Item -ItemType Directory -Path $dir | Out-Null }
$full = if ($dir) { Join-Path (Resolve-Path -LiteralPath $dir) (Split-Path -Leaf $Out) } else { Join-Path (Get-Location) $Out }
$result.Save($full, [System.Drawing.Imaging.ImageFormat]::Png)
$result.Dispose(); $sq.Dispose(); $src.Dispose()
"wrote $Out ($Size x $Size, transparent)"
