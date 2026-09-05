<#
    viz-client.ps1 - a harness for the viz channel, and the ruler for #7.

    NOT the example client control-api.md asks for; that ships at v1.0 with
    the frozen protocol, in Python. This subscribes over the control pipe,
    opens the per-subscriber pipe it is given, reads binary frames, and
    reports how far behind the analyser's read each one arrived.

    The protocol is UNDOCUMENTED AND UNSTABLE until v1.0. Do not build
    anything real against it yet.

    Usage:
      tools\viz-client.ps1                          # 32 bands, 30 Hz, u8, 10 s, then a summary
      tools\viz-client.ps1 -Bands 19 -Rate 60 -Show # live bars while it runs
      tools\viz-client.ps1 -Depth f32 -Seconds 30
      tools\viz-client.ps1 -StallSeconds 2          # stop reading for 2 s at the halfway
                                                    # point: how many stale frames come back?

    Latency is measured against this machine's wall clock, which the frame's
    timestamp also uses (control-api.md), so a number here is speaker-side
    truth only after the audio output latency is subtracted; the summary says
    which is which. Windows PowerShell 5.1, .NET Framework: DateTime.UtcNow
    is coarse there, so the clock is GetSystemTimePreciseAsFileTime.
#>
param(
    [int]$Bands = 32,
    [ValidateSet(15, 30, 60)][int]$Rate = 30,
    [ValidateSet("u8", "f32")][string]$Depth = "u8",
    [int]$Seconds = 10,
    [switch]$Show,
    [double]$StallSeconds = 0
)

$ErrorActionPreference = "Stop"

Add-Type -Namespace HP -Name Clock -MemberDefinition @'
[System.Runtime.InteropServices.DllImport("kernel32.dll")]
public static extern void GetSystemTimePreciseAsFileTime(out long ft);
public static long NowUs() {
    long ft; GetSystemTimePreciseAsFileTime(out ft);
    return (ft - 116444736000000000L) / 10;
}
'@

# ---- control channel: hello, subscribe --------------------------------------

$ctl = New-Object System.IO.Pipes.NamedPipeClientStream(".", "hurricane-party", [System.IO.Pipes.PipeDirection]::InOut)
try { $ctl.Connect(3000) } catch { Write-Error "Couldn't reach hurricane-party. Is it running?"; exit 1 }
$r = New-Object System.IO.StreamReader($ctl)
$w = New-Object System.IO.StreamWriter($ctl); $w.AutoFlush = $true

function Send($obj) {
    $line = $obj | ConvertTo-Json -Compress
    Write-Host "-> $line" -ForegroundColor DarkGray
    $w.WriteLine($line)
    $reply = $r.ReadLine()
    Write-Host "<- $reply" -ForegroundColor DarkGray
    $reply | ConvertFrom-Json
}

$hello = Send @{ id = 0; cmd = "hello"; client = "ps-viz-harness"; protocol_version = 1 }
if (-not $hello.ok) { Write-Error "hello refused: $($hello.error)"; exit 1 }
if ($hello.result.capabilities -notcontains "viz") { Write-Error "this build does not advertise viz"; exit 1 }

$sub = Send @{ id = 1; cmd = "subscribe_viz"; bands = $Bands; rate_hz = $Rate; depth = $Depth }
if (-not $sub.ok) { Write-Error "subscribe_viz refused: $($sub.error)"; exit 1 }
$stream = [string]$sub.result.stream
$pipeName = $stream.Substring($stream.LastIndexOf('\') + 1)

# The subscription belongs to the viz pipe, not to this connection: close it
# and the frames keep coming. That is the lifetime model, proved every run.
$ctl.Dispose()
Write-Host "control connection closed; opening $stream" -ForegroundColor Cyan

# ---- viz channel: read frames -----------------------------------------------

$viz = New-Object System.IO.Pipes.NamedPipeClientStream(".", $pipeName, [System.IO.Pipes.PipeDirection]::In)
try { $viz.Connect(3000) } catch { Write-Error "Couldn't open the viz pipe $stream"; exit 1 }

$HEADER = 18
$bpb = if ($Depth -eq "u8") { 1 } else { 4 }
$frameLen = $HEADER + $Bands * $bpb
$buf = New-Object byte[] (64 * 1024)
$have = 0
$ramp = " .:-=+*#%@"

$lat = New-Object System.Collections.Generic.List[double]
$gaps = New-Object System.Collections.Generic.List[double]
$frames = 0; $beats = 0; $badMagic = 0; $lastTs = 0; $shown = 0
$staleAfterStall = 0; $stalled = $false
$periodMs = 1000.0 / $Rate
$t0 = [HP.Clock]::NowUs()
$endUs = $t0 + [long]($Seconds * 1000000)
$stallAtUs = $t0 + [long]($Seconds * 500000)

while ([HP.Clock]::NowUs() -lt $endUs) {
    if ($StallSeconds -gt 0 -and -not $stalled -and [HP.Clock]::NowUs() -ge $stallAtUs) {
        Write-Host ("stalling for {0} s (not reading)..." -f $StallSeconds) -ForegroundColor Yellow
        Start-Sleep -Milliseconds ([int]($StallSeconds * 1000))
        $stalled = $true
        $resumedAt = [HP.Clock]::NowUs()
    }
    $n = $viz.Read($buf, $have, $buf.Length - $have)
    if ($n -le 0) { Write-Host "pipe closed by the server" -ForegroundColor Red; break }
    $have += $n
    $off = 0
    while ($have - $off -ge $HEADER) {
        if (-not ($buf[$off] -eq 0x48 -and $buf[$off + 1] -eq 0x50 -and $buf[$off + 2] -eq 0x56 -and $buf[$off + 3] -eq 0x31)) {
            # Resync on the magic, as the doc says a client can.
            $badMagic++; $off++; continue
        }
        $nb = $buf[$off + 12]; $dep = $buf[$off + 13]
        $len = $HEADER + $nb * $(if ($dep -eq 0) { 1 } else { 4 })
        if ($have - $off -lt $len) { break }
        $ts = [BitConverter]::ToUInt64($buf, $off + 4)
        $now = [HP.Clock]::NowUs()
        $l = ($now - [double]$ts) / 1000.0
        $lat.Add($l)
        if ($lastTs -gt 0) { $gaps.Add(([double]$ts - $lastTs) / 1000.0) }
        $lastTs = $ts
        $frames++
        if (($buf[$off + 14] -band 1) -ne 0) { $beats++ }
        if ($stalled -and $now - $resumedAt -lt 500000 -and $l -gt 100) { $staleAfterStall++ }
        if ($Show -and ($frames % [Math]::Max(1, [int]($Rate / 15))) -eq 0) {
            $line = New-Object System.Text.StringBuilder
            for ($i = 0; $i -lt $nb; $i++) {
                $v = if ($dep -eq 0) { $buf[$off + $HEADER + $i] / 255.0 } else { [BitConverter]::ToSingle($buf, $off + $HEADER + $i * 4) }
                [void]$line.Append($ramp[[Math]::Min(9, [int]($v * 9.999))])
            }
            $beat = if (($buf[$off + 14] -band 1) -ne 0) { "*" } else { " " }
            Write-Host ("{0} |{1}| peak {2,3} rms {3,3} {4,6:0.0} ms" -f $beat, $line.ToString(), $buf[$off + 16], $buf[$off + 17], $l)
            $shown++
        }
        $off += $len
    }
    if ($off -gt 0) {
        [Array]::Copy($buf, $off, $buf, 0, $have - $off)
        $have -= $off
    }
}
$viz.Dispose()

# ---- the numbers ------------------------------------------------------------

function Pct($list, $p) {
    if ($list.Count -eq 0) { return [double]::NaN }
    $s = $list | Sort-Object
    $s[[Math]::Min($s.Count - 1, [int][Math]::Round(($s.Count - 1) * $p))]
}

$ran = ([HP.Clock]::NowUs() - $t0) / 1000000.0
$drops = @($gaps | Where-Object { $_ -gt 1.5 * $periodMs }).Count
Write-Host ""
Write-Host ("{0} frames in {1:0.0} s = {2:0.0} Hz (asked {3}); {4} band(s), depth {5}; {6} beat(s)" -f $frames, $ran, ($frames / $ran), $Rate, $Bands, $Depth, $beats) -ForegroundColor Cyan
Write-Host ("latency, analyser read -> this process:  p50 {0,6:0.00} ms   p95 {1,6:0.00} ms   max {2,6:0.00} ms" -f (Pct $lat 0.5), (Pct $lat 0.95), (Pct $lat 1.0)) -ForegroundColor Green
Write-Host ("cadence, between frames:                 p50 {0,6:0.0} ms   p95 {1,6:0.0} ms   max {2,6:0.0} ms   gaps over 1.5x period: {3}" -f (Pct $gaps 0.5), (Pct $gaps 0.95), (Pct $gaps 1.0), $drops)
if ($badMagic -gt 0) { Write-Host "resynced past $badMagic byte(s) that were not a frame" -ForegroundColor Yellow }
if ($stalled) { Write-Host ("after a {0} s stall, {1} stale frame(s) (older than 100 ms) were delivered before live ones" -f $StallSeconds, $staleAfterStall) -ForegroundColor Yellow }
Write-Host "the speaker plays what the analyser read after the audio output latency; the app prints that with HP_VIZ_TRACE=1" -ForegroundColor DarkGray
