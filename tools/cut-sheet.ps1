<#
    cut-sheet.ps1 - cut a sheet of loose poses into one PNG per pose, for
    sheet.ps1 (docs/companion-art.md).

      powershell -NoProfile -ExecutionPolicy Bypass -File tools\cut-sheet.ps1 -In design\sprites\wee-man\sheet.png -Out .sid\wee-man-poses
      powershell -NoProfile -ExecutionPolicy Bypass -File tools\cut-sheet.ps1 -In design\sprites\wee-man\sheet.png -Out .sid\wee-man-poses -Map "idle=0.0 0.1 2.1 2.3; sleep=1.0 1.1; dance=4.0 4.1 4.2 4.3; walk=3.0 3.1 3.2 3.3; startle=5.1 5.2; pet=6.0; carry=7.0 7.1"

    For an image model that returns every pose on one transparent sheet, a
    row per action, instead of one image per pose (Wee Man, D164). The poses
    were all drawn at one scale on one canvas, so each is cut tight and
    sheet.ps1's one factor keeps them the sizes they were drawn.

    A pose is its body, a connected shape of at least -Body pixels, plus the
    small marks beside it within -Reach pixels: a sleeper's z's, a heart, the
    lines around a jump. Bodies group into rows by height on the sheet, and a
    body lying mostly over another in its row (a hand drawn apart from the
    head) joins that pose. Marks that stick out past the body's sides move in
    over it, all together, so a "z Z" keeps its shape and never makes its
    pose the widest one, which would shrink every frame.

    Without -Map it writes every pose as row<r>-<n>.png and prints the rows it
    found, to see which is which. -Map names them: "<state>=<row>.<n> ..." in
    playing order, separated by ";", both numbers from 0. A state's later
    idle frames are its moments (sheet.ps1), so other rows can feed idle.

    Windows PowerShell 5.1, System.Drawing only. The sheet must be a PNG:
    Windows' own WebP decoder drops the alpha.
#>
param(
    [Parameter(Mandatory = $true)][string]$In,
    [Parameter(Mandatory = $true)][string]$Out,
    [string]$Map = "",
    [int]$Body = 2500,
    [int]$Reach = 60
)
$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing
Add-Type -ReferencedAssemblies System.Drawing @"
using System;
using System.Collections.Generic;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;

public class CutPart { public int Id, X0, Y0, X1, Y1, Area; }

public class CutPose {
    public List<int> Bodies = new List<int>();
    public List<int> Marks = new List<int>();
    public int X0, Y0, X1, Y1;
}

public static class CutSheet {
    static int W, H;
    static byte[] Px;
    static int[] Lab;
    static List<CutPart> Parts = new List<CutPart>();
    public static List<List<CutPose>> Rows = new List<List<CutPose>>();

    static bool Solid(int i) { return Px[i * 4 + 3] > 8; }

    public static void Load(string path) {
        using (var bmp = new Bitmap(path)) {
            W = bmp.Width; H = bmp.Height;
            var data = bmp.LockBits(new Rectangle(0, 0, W, H), ImageLockMode.ReadOnly, PixelFormat.Format32bppArgb);
            Px = new byte[W * H * 4];
            for (int y = 0; y < H; y++)
                Marshal.Copy(data.Scan0 + y * data.Stride, Px, y * W * 4, W * 4);
            bmp.UnlockBits(data);
        }
    }

    // Connected shapes, touching at an edge or a corner.
    public static void Label() {
        Lab = new int[W * H];
        var stack = new Stack<int>();
        for (int i = 0; i < W * H; i++) {
            if (!Solid(i) || Lab[i] != 0) continue;
            var p = new CutPart { Id = Parts.Count + 1, X0 = W, Y0 = H, X1 = 0, Y1 = 0 };
            Parts.Add(p);
            Lab[i] = p.Id;
            stack.Push(i);
            while (stack.Count > 0) {
                int q = stack.Pop(), x = q % W, y = q / W;
                p.Area++;
                p.X0 = Math.Min(p.X0, x); p.X1 = Math.Max(p.X1, x + 1);
                p.Y0 = Math.Min(p.Y0, y); p.Y1 = Math.Max(p.Y1, y + 1);
                for (int dy = -1; dy <= 1; dy++)
                    for (int dx = -1; dx <= 1; dx++) {
                        int nx = x + dx, ny = y + dy;
                        if (nx < 0 || ny < 0 || nx >= W || ny >= H) continue;
                        int n = ny * W + nx;
                        if (Solid(n) && Lab[n] == 0) { Lab[n] = p.Id; stack.Push(n); }
                    }
            }
        }
    }

    static void Grow(CutPose pose, CutPart p) {
        pose.X0 = Math.Min(pose.X0, p.X0); pose.X1 = Math.Max(pose.X1, p.X1);
        pose.Y0 = Math.Min(pose.Y0, p.Y0); pose.Y1 = Math.Max(pose.Y1, p.Y1);
    }

    static double Gap(CutPart m, CutPose p) {
        int dx = Math.Max(Math.Max(p.X0 - m.X1, m.X0 - p.X1), 0);
        int dy = Math.Max(Math.Max(p.Y0 - m.Y1, m.Y0 - p.Y1), 0);
        return Math.Sqrt(dx * dx + dy * dy);
    }

    public static void Group(int body, int reach) {
        var bodies = Parts.FindAll(p => p.Area >= body);
        if (bodies.Count == 0) throw new Exception("no shape of " + body + " px or more; is the background transparent?");
        bodies.Sort((a, b) => (a.Y0 + a.Y1).CompareTo(b.Y0 + b.Y1));
        // A new row starts when a body's middle is below the last one's by
        // more than half the shortest body.
        int shortest = int.MaxValue;
        foreach (var b in bodies) shortest = Math.Min(shortest, b.Y1 - b.Y0);
        var rows = new List<List<CutPart>>();
        double lastMid = double.NegativeInfinity;
        foreach (var b in bodies) {
            double mid = (b.Y0 + b.Y1) / 2.0;
            if (rows.Count == 0 || mid - lastMid > shortest / 2.0) rows.Add(new List<CutPart>());
            rows[rows.Count - 1].Add(b);
            lastMid = mid;
        }
        foreach (var row in rows) {
            row.Sort((a, b) => a.X0.CompareTo(b.X0));
            var poses = new List<CutPose>();
            foreach (var b in row) {
                if (poses.Count > 0) {
                    var last = poses[poses.Count - 1];
                    int over = Math.Min(last.X1, b.X1) - Math.Max(last.X0, b.X0);
                    if (over > 0.5 * Math.Min(b.X1 - b.X0, last.X1 - last.X0)) {
                        last.Bodies.Add(b.Id); Grow(last, b); continue;
                    }
                }
                var pose = new CutPose { X0 = b.X0, Y0 = b.Y0, X1 = b.X1, Y1 = b.Y1 };
                pose.Bodies.Add(b.Id);
                poses.Add(pose);
            }
            Rows.Add(poses);
        }
        var all = new List<CutPose>();
        foreach (var r in Rows) all.AddRange(r);
        foreach (var m in Parts) {
            if (m.Area >= body || m.Area < 6) continue;   // a body, or a speck
            CutPose near = null; double best = double.MaxValue;
            foreach (var p in all) { double g = Gap(m, p); if (g < best) { best = g; near = p; } }
            if (near != null && best < reach) near.Marks.Add(m.Id);
        }
        foreach (var p in all) Tuck(p);
    }

    // Move the marks that stick out past the body's sides back over it, all
    // by the same step, so they keep their places relative to each other.
    static void Tuck(CutPose pose) {
        int bx0 = int.MaxValue, bx1 = int.MinValue;
        foreach (int id in pose.Bodies) { bx0 = Math.Min(bx0, Parts[id - 1].X0); bx1 = Math.Max(bx1, Parts[id - 1].X1); }
        pose.X0 = bx0; pose.X1 = bx1;
        foreach (int id in pose.Bodies) Grow(pose, Parts[id - 1]);
        if (pose.Marks.Count == 0) return;
        int mx0 = int.MaxValue, mx1 = int.MinValue;
        foreach (int id in pose.Marks) { mx0 = Math.Min(mx0, Parts[id - 1].X0); mx1 = Math.Max(mx1, Parts[id - 1].X1); }
        int shift = mx1 > bx1 ? bx1 - mx1 : (mx0 < bx0 ? bx0 - mx0 : 0);
        if (shift != 0) {
            // Lift every mark first, then put them all down, so one never
            // lands on another that has yet to move.
            var pixels = new List<int>();
            var owners = new List<int>();
            var colours = new List<byte[]>();
            foreach (int id in pose.Marks) {
                var m = Parts[id - 1];
                for (int y = m.Y0; y < m.Y1; y++)
                    for (int x = m.X0; x < m.X1; x++) {
                        int i = y * W + x;
                        if (Lab[i] != id) continue;
                        pixels.Add(i); owners.Add(id);
                        colours.Add(new byte[] { Px[i * 4], Px[i * 4 + 1], Px[i * 4 + 2], Px[i * 4 + 3] });
                    }
                m.X0 += shift; m.X1 += shift;
            }
            foreach (int i in pixels) { Px[i * 4 + 3] = 0; Lab[i] = 0; }
            for (int k = 0; k < pixels.Count; k++) {
                int to = pixels[k] + shift;
                Array.Copy(colours[k], 0, Px, to * 4, 4);
                Lab[to] = owners[k];
            }
        }
        foreach (int id in pose.Marks) Grow(pose, Parts[id - 1]);
    }

    public static string Describe() {
        var s = new System.Text.StringBuilder();
        for (int r = 0; r < Rows.Count; r++) {
            s.Append("row " + r + ":");
            foreach (var p in Rows[r])
                s.Append("  [" + (p.X1 - p.X0) + "x" + (p.Y1 - p.Y0) + " at " + p.X0 + "," + p.Y0 + (p.Marks.Count > 0 ? ", " + p.Marks.Count + " marks" : "") + "]");
            s.AppendLine();
        }
        return s.ToString();
    }

    // One pose alone, cut to its own box.
    public static void Write(int row, int n, string path) {
        if (row < 0 || row >= Rows.Count || n < 0 || n >= Rows[row].Count)
            throw new Exception("there is no pose " + row + "." + n);
        var p = Rows[row][n];
        var keep = new HashSet<int>(p.Bodies);
        keep.UnionWith(p.Marks);
        int w = p.X1 - p.X0, h = p.Y1 - p.Y0;
        using (var bmp = new Bitmap(w, h, PixelFormat.Format32bppArgb)) {
            var data = bmp.LockBits(new Rectangle(0, 0, w, h), ImageLockMode.WriteOnly, PixelFormat.Format32bppArgb);
            var line = new byte[w * 4];
            for (int y = 0; y < h; y++) {
                Array.Clear(line, 0, line.Length);
                for (int x = 0; x < w; x++) {
                    int i = (p.Y0 + y) * W + (p.X0 + x);
                    if (keep.Contains(Lab[i])) Array.Copy(Px, i * 4, line, x * 4, 4);
                }
                Marshal.Copy(line, 0, data.Scan0 + y * data.Stride, line.Length);
            }
            bmp.UnlockBits(data);
            bmp.Save(path, ImageFormat.Png);
        }
    }
}
"@

$src = (Resolve-Path -LiteralPath $In).Path
New-Item -ItemType Directory -Force $Out | Out-Null
$dest = (Resolve-Path -LiteralPath $Out).Path

[CutSheet]::Load($src)
[CutSheet]::Label()
[CutSheet]::Group($Body, $Reach)
Write-Host ([CutSheet]::Describe())

if (-not $Map) {
    for ($r = 0; $r -lt [CutSheet]::Rows.Count; $r++) {
        for ($n = 0; $n -lt [CutSheet]::Rows[$r].Count; $n++) {
            [CutSheet]::Write($r, $n, (Join-Path $dest "row$r-$n.png"))
        }
    }
    Write-Host "wrote every pose as row<r>-<n>.png in $Out; name them with -Map"
    exit 0
}

$written = 0
foreach ($entry in ($Map -split ";")) {
    if (-not $entry.Trim()) { continue }
    $state, $picks = $entry -split "=", 2
    $state = $state.Trim()
    if (-not $picks) { throw "'$entry' should be <state>=<row>.<n> ..." }
    $i = 0
    foreach ($pick in ($picks.Trim() -split "\s+")) {
        if ($pick -notmatch "^(\d+)\.(\d+)$") { throw "'$pick' in $state should be <row>.<n>" }
        [CutSheet]::Write([int]$Matches[1], [int]$Matches[2], (Join-Path $dest "$state-$i.png"))
        $i++; $written++
    }
}
Write-Host "wrote $written poses to $Out"
