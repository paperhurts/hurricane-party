<#
    zip-points.ps1 - trim the Census Bureau's ZCTA gazetteer to the table the
    Cone radar centres a ZIP code with (#161): src-tauri\src\radar_zips.txt.

      powershell -NoProfile -ExecutionPolicy Bypass -File tools\zip-points.ps1 -In .sid\gazetteer\2025_Gaz_zcta_national.zip

    The gazetteer is downloaded by hand, once, from
    https://www2.census.gov/geo/docs/maps-data/data/gazetteer/ (the year's
    folder, the national ZCTA file). The app never fetches it (D29). It is
    pipe-separated, GEOID|...|INTPTLAT|INTPTLONG, one row per ZIP Code
    Tabulation Area: GEOID is the ZIP code and INTPT its internal point.

    What is kept is the ZIP code and that point to four decimal places, about
    eleven metres, the most the Weather Service's alerts take. No place
    names: the file has none. A newer year is the same command on the newer
    zip, with -Year, and a line in licenses\THIRD-PARTY-NOTICES.md.

    Windows PowerShell 5.1.
#>
param(
    [Parameter(Mandatory = $true)][string]$In,
    [string]$Out = "src-tauri\src\radar_zips.txt",
    [string]$Year = "2025"
)
$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.IO.Compression.FileSystem

$zip = [System.IO.Compression.ZipFile]::OpenRead((Resolve-Path -LiteralPath $In).Path)
try {
    $entry = $zip.Entries | Where-Object { $_.Name -like "*.txt" } | Select-Object -First 1
    if (-not $entry) { throw "no .txt in $In" }
    $reader = New-Object System.IO.StreamReader($entry.Open(), [System.Text.Encoding]::UTF8)
    $text = $reader.ReadToEnd()
    $reader.Dispose()
} finally {
    $zip.Dispose()
}

$lines = $text -split "`r?`n" | Where-Object { $_.Trim() }
$head = $lines[0].Split("|") | ForEach-Object { $_.Trim() }
$iZip = [Array]::IndexOf($head, "GEOID")
$iLat = [Array]::IndexOf($head, "INTPTLAT")
$iLon = [Array]::IndexOf($head, "INTPTLONG")
if ($iZip -lt 0 -or $iLat -lt 0 -or $iLon -lt 0) { throw "unexpected columns: $($lines[0])" }

$inv = [System.Globalization.CultureInfo]::InvariantCulture
$rows = New-Object System.Collections.Generic.List[string]
foreach ($line in $lines[1..($lines.Count - 1)]) {
    $f = $line.Split("|")
    $code = $f[$iZip].Trim()
    if ($code -notmatch '^\d{5}$') { throw "not a ZIP code: $line" }
    $lat = [double]::Parse($f[$iLat].Trim(), $inv)
    $lon = [double]::Parse($f[$iLon].Trim(), $inv)
    $rows.Add(("{0} {1} {2}" -f $code, $lat.ToString("F4", $inv), $lon.ToString("F4", $inv)))
}
$rows.Sort([System.StringComparer]::Ordinal)

$sb = New-Object System.Text.StringBuilder
[void]$sb.Append("# ZIP code, latitude, longitude: each ZIP Code Tabulation Area's internal point.`n")
[void]$sb.Append("# U.S. Census Bureau, $Year Gazetteer Files, ZCTA national file. Public domain.`n")
[void]$sb.Append("# Made by tools/zip-points.ps1; sorted by ZIP code.`n")
foreach ($r in $rows) { [void]$sb.Append($r).Append("`n") }
$path = Join-Path (Get-Location) $Out
[System.IO.File]::WriteAllText($path, $sb.ToString(), (New-Object System.Text.UTF8Encoding($false)))
Write-Output ("{0} ZIP codes written to {1}" -f $rows.Count, $Out)
