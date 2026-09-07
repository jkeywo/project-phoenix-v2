[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$SourceBundle,
    [Parameter(Mandatory)][string]$Destination,
    [ValidateRange(1024, 65535)][int]$DiagnosticPort = 18181
)
$ErrorActionPreference = 'Stop'
$destination = [IO.Path]::GetFullPath($Destination)
if (Test-Path -LiteralPath $destination) { throw "Scratch bundle already exists: $destination" }
New-Item -ItemType Directory -Path $destination | Out-Null
# Copy documents and GUI code; reference large immutable assets in place.
# Junctions are only created in the new scratch bundle and are never modified.
foreach ($entry in Get-ChildItem -LiteralPath $SourceBundle) {
    $target = Join-Path $destination $entry.Name
    if (-not $entry.PSIsContainer) { Copy-Item -LiteralPath $entry.FullName -Destination $target }
    elseif ($entry.Name -eq 'client') {
        New-Item -ItemType Directory -Path $target | Out-Null
        foreach ($clientEntry in Get-ChildItem -LiteralPath $entry.FullName) {
            $clientTarget = Join-Path $target $clientEntry.Name
            if ($clientEntry.PSIsContainer -and $clientEntry.Name -eq 'assets') {
                New-Item -ItemType Junction -Path $clientTarget -Target $clientEntry.FullName | Out-Null
            } else { Copy-Item -LiteralPath $clientEntry.FullName -Destination $clientTarget -Recurse }
        }
    } elseif ($entry.Name -eq 'gui') { Copy-Item -LiteralPath $entry.FullName -Destination $target -Recurse }
    else { New-Item -ItemType Junction -Path $target -Target $entry.FullName | Out-Null }
}
$clientIndex = Join-Path $destination 'client\index.html'
$beforeHash = (Get-FileHash -LiteralPath $clientIndex).Hash
$html = [IO.File]::ReadAllText($clientIndex)
if (-not $html.Contains('</body>')) { throw 'Client document has no body close tag.' }
$shim = [IO.File]::ReadAllText((Join-Path $PSScriptRoot 'profile-ready-shim.js')).Replace('127.0.0.1:18181', "127.0.0.1:$DiagnosticPort")
$html = $html.Replace('</body>', "<script>`n$shim`n</script>`n</body>")
[IO.File]::WriteAllText($clientIndex, $html, [Text.UTF8Encoding]::new($false))
[pscustomobject]@{ source=$SourceBundle; destination=$destination; sourceIndexSha256=$beforeHash; instrumentedIndexSha256=(Get-FileHash -LiteralPath $clientIndex).Hash; readiness='SetReady then acknowledged SetAfk; normal commands restore Backfill and retain console focus'; diagnosticPort=$DiagnosticPort; immutableAssetJunctions=@('assets','client/assets','fonts','shaders','viewscreen') } | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $destination 'review-instrumentation.json') -Encoding UTF8
Write-Output "Prepared review-only native bundle: $destination"
