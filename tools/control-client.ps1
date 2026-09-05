<#
    A throwaway client for the hp-control pipe, so v0.3 can actually be tested.

    NOT the example client control-api.md asks for — that ships at v1.0 with the
    frozen protocol, in Python, alongside the docs. This is a harness.

    The protocol is UNDOCUMENTED AND UNSTABLE until v1.0. Do not build anything
    real against it yet; that instability is the whole point of shipping the
    channel early.

    Usage:
      tools\control-client.ps1                 # handshake + status
      tools\control-client.ps1 toggle
      tools\control-client.ps1 seek 42.5
      tools\control-client.ps1 volume 0.4
      tools\control-client.ps1 listen          # watch unsolicited events
#>
param([string]$Cmd = "status", [double]$Arg)

$ErrorActionPreference = "Stop"
$pipe = New-Object System.IO.Pipes.NamedPipeClientStream(".", "hurricane-party", [System.IO.Pipes.PipeDirection]::InOut)
try { $pipe.Connect(3000) } catch { Write-Error "Couldn't reach hurricane-party. Is it running?"; exit 1 }

$r = New-Object System.IO.StreamReader($pipe)
$w = New-Object System.IO.StreamWriter($pipe); $w.AutoFlush = $true

function Send($obj) {
    $w.WriteLine(($obj | ConvertTo-Json -Compress))
    $r.ReadLine()
}

# The handshake is required first; the server rejects a version it doesn't speak
# rather than guessing.
$hello = Send @{ id = 0; cmd = "hello"; client = "ps-harness"; protocol_version = 1 }
Write-Host "<- $hello" -ForegroundColor DarkGray

if ($Cmd -eq "listen") {
    Write-Host "watching for events (ctrl-c to stop)" -ForegroundColor Cyan
    # A blocking ReadLine never returns control to PowerShell, so Ctrl-C did
    # nothing until the next event arrived. Read asynchronously and wait in
    # short slices; the engine checks for Ctrl-C between them.
    while ($true) {
        $task = $r.ReadLineAsync()
        while (-not $task.Wait(200)) { }
        $line = $task.Result
        if ($null -eq $line) { break }
        Write-Host "<- $line" -ForegroundColor Green
    }
    exit 0
}

$req = @{ id = 1; cmd = $Cmd }
if ($PSBoundParameters.ContainsKey('Arg')) {
    if ($Cmd -eq "seek")   { $req.pos_s = $Arg }
    if ($Cmd -eq "volume") { $req.level = $Arg }
}
Write-Host "-> $($req | ConvertTo-Json -Compress)" -ForegroundColor DarkGray
Write-Host "<- $(Send $req)" -ForegroundColor Green
