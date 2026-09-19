param([string]$SourceDirectory = "$PSScriptRoot/../upstream/UxPlay")
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path -LiteralPath $SourceDirectory).Path
$items = Get-ChildItem -LiteralPath $root -File -Recurse | Where-Object { $_.Extension -in '.c', '.h', '.cpp', '.py' }
$inventory = foreach ($item in $items) {
    $relative = $item.FullName.Substring($root.Length + 1).Replace('\', '/')
    $group = if ($relative.StartsWith('lib/llhttp/')) { 'llhttp' }
        elseif ($relative.StartsWith('lib/playfair/')) { 'playfair' }
        elseif ($relative.StartsWith('lib/mdnsd/') -or $relative.StartsWith('lib/dns_sd/')) { 'discovery_backends' }
        elseif ($relative.StartsWith('renderers/')) { 'renderers' }
        elseif ($relative.StartsWith('Bluetooth_LE_beacon/')) { 'bluetooth_python' }
        elseif ($relative -eq 'uxplay.cpp') { 'application' }
        else { 'protocol_and_utilities' }
    [pscustomobject]@{ file = $relative; group = $group; physical_lines = [System.IO.File]::ReadAllLines($item.FullName).Length; bytes = $item.Length }
}
New-Item -ItemType Directory -Force -Path "$PSScriptRoot/evidence" | Out-Null
$inventory | Sort-Object file | Export-Csv -NoTypeInformation -Encoding utf8 -LiteralPath "$PSScriptRoot/evidence/source-inventory.csv"
$inventory | Group-Object group | ForEach-Object {
    [pscustomobject]@{ group = $_.Name; files = $_.Count; physical_lines = ($_.Group | Measure-Object physical_lines -Sum).Sum; bytes = ($_.Group | Measure-Object bytes -Sum).Sum }
} | Format-Table -AutoSize
$commit = git -c "safe.directory=$($root.Replace('\', '/'))" -C $root log -1 --format='%H%n%cs%n%s'
if ($LASTEXITCODE -ne 0) { throw 'Unable to read upstream commit' }
$commit | Set-Content -Encoding utf8 -LiteralPath "$PSScriptRoot/evidence/upstream-commit.txt"
