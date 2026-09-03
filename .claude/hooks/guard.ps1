<#
    guard.ps1 - a Claude Code hook. Mechanical enforcement of three rules that
    used to live on the honour system (CLAUDE.md, non-negotiables and conventions):

      1. docs/decisions.md is append-only. Edit is fine; a whole-file Write is
         how a rewrite happens by accident.                       [pre,  Write]
      2. Never hardcode a colour under src/. design/tokens.json is the only
         home for a hex.                                           [post, Edit|Write]
      3. Platform-specific code lives behind the trait in src-tauri/src/platform/,
         never as #[cfg(windows)] scattered through the rest.      [post, Edit|Write]

    Only the text being introduced is checked (Edit's new_string, Write's
    content), so a pre-existing line never trips it on an unrelated edit.
    Exit 2 with the reason on stderr: PreToolUse blocks the call; PostToolUse
    has already landed the edit and hands the reason back to Claude to fix in
    the same turn.

    A line that must carry a literal colour says why with a marker and is skipped:
        outline: #FF00FF; /* tokens-exempt: debug overlay, never shipped */
    (The video letterbox is not an example: D65 made it a theme token.)

    Windows PowerShell 5.1. Invoked from .claude/settings.json with the project
    root as the working directory.
#>
param(
    [Parameter(Mandatory = $true)][ValidateSet("pre", "post")][string]$Phase
)

$raw = [Console]::In.ReadToEnd()
if ([string]::IsNullOrWhiteSpace($raw)) { exit 0 }
try { $hook = $raw | ConvertFrom-Json } catch { exit 0 }

$tool = [string]$hook.tool_name
$path = [string]$hook.tool_input.file_path
if (-not $path) { exit 0 }
$rel = $path -replace '\\', '/'
$nl = [Environment]::NewLine

function Fail($msg) {
    [Console]::Error.WriteLine($msg)
    exit 2
}

if ($Phase -eq "pre") {
    if ($tool -eq "Write" -and $rel -match '(^|/)docs/decisions\.md$') {
        Fail 'guard: docs/decisions.md is append-only. Use Edit to add or strike a row (or /decide). A whole-file Write is refused.'
    }
    exit 0
}

# post: only the text that is new
$new = $null
if ($tool -eq "Edit")  { $new = [string]$hook.tool_input.new_string }
if ($tool -eq "Write") { $new = [string]$hook.tool_input.content }
if (-not $new) { exit 0 }
$lines = $new -split "`r?`n"

# Rule 2: colours in the frontend.
if ($rel -match '(^|/)src/.*\.(svelte|ts|css|html)$') {
    $hits = @()
    foreach ($l in $lines) {
        if ($l -match 'tokens-exempt') { continue }
        # 6- or 8-digit hex anywhere; 3- or 4-digit only in CSS value position,
        # so an issue reference like "#123" in a comment does not trip it.
        if ($l -match '#[0-9a-fA-F]{6}([0-9a-fA-F]{2})?\b' -or $l -match ':\s*#[0-9a-fA-F]{3,4}\b') {
            $hits += ('    ' + $l.Trim())
        }
    }
    if ($hits.Count -gt 0) {
        Fail ('guard: hardcoded colour in ' + $rel + '. Never hardcode a hex; import design/tokens.json (CLAUDE.md). If it truly is not a theme colour, put a tokens-exempt: <why> comment on the line.' + $nl + ($hits -join $nl))
    }
}

# Rule 3: cfg(windows) outside the platform module.
if ($rel -match '(^|/)src-tauri/src/.*\.rs$' -and $rel -notmatch '/platform/') {
    $bad = @($lines | Where-Object { $_ -match 'cfg\s*\(\s*(not\s*\()?\s*(windows|target_os\s*=\s*"windows")' })
    if ($bad.Count -gt 0) {
        Fail ('guard: #[cfg(windows)] in ' + $rel + '. Platform-specific calls go behind the trait in src-tauri/src/platform/ (CLAUDE.md conventions).' + $nl + (($bad | ForEach-Object { '    ' + $_.Trim() }) -join $nl))
    }
}

exit 0
