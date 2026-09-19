param([string]$OutputDirectory = "$PSScriptRoot/evidence")
$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force -Path $OutputDirectory | Out-Null
$headers = @{ 'User-Agent' = 'rust-airplay-feasibility-research/0.1' }
$packages = 'gstreamer', 'mdns-sd', 'plist', 'srp', 'aes-gcm', 'symphonia', 'btleplug', 'rtsp-types'
$results = foreach ($package in $packages) {
    $response = Invoke-RestMethod -Uri "https://crates.io/api/v1/crates/$package" -Headers $headers -TimeoutSec 30
    $crate = $response.crate
    [pscustomobject]@{
        name = $crate.name
        version = $crate.max_stable_version
        description = $crate.description
        repository = $crate.repository
        documentation = $crate.documentation
        source = "https://crates.io/api/v1/crates/$package"
        retrieved_at = [DateTime]::UtcNow.ToString('o')
    }
}
$results | ConvertTo-Json -Depth 6 | Set-Content -Encoding utf8 -LiteralPath "$OutputDirectory/crates.json"
$documents = @{
    'gstreamer-rs-readme.md' = 'https://raw.githubusercontent.com/GStreamer/gstreamer-rs/main/README.md'
    'symphonia-readme.md' = 'https://raw.githubusercontent.com/pdeljanov/Symphonia/master/README.md'
    'btleplug-readme.md' = 'https://raw.githubusercontent.com/deviceplug/btleplug/master/README.md'
    'mdns-sd-readme.md' = 'https://raw.githubusercontent.com/keepsimple1/mdns-sd/main/README.md'
}
$documentResults = foreach ($entry in $documents.GetEnumerator()) {
    try {
        Invoke-WebRequest -Uri $entry.Value -Headers $headers -TimeoutSec 30 -OutFile "$OutputDirectory/$($entry.Key)"
        [pscustomobject]@{ file = $entry.Key; url = $entry.Value; retrieved_at = [DateTime]::UtcNow.ToString('o'); success = $true }
    } catch {
        [pscustomobject]@{ file = $entry.Key; url = $entry.Value; retrieved_at = [DateTime]::UtcNow.ToString('o'); success = $false; error = $_.Exception.Message }
    }
}
$documentResults | ConvertTo-Json -Depth 5 | Set-Content -Encoding utf8 -LiteralPath "$OutputDirectory/documents.json"
$results | Format-Table name,version,repository -AutoSize
$documentResults | Format-Table file,success -AutoSize
