[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Receipt,
    [Parameter(Mandatory)][string]$ContentRoot,
    [Parameter(Mandatory)][string]$Hardware,
    [Parameter(Mandatory)][string]$Output,
    [ValidateSet('headless', 'native')][string]$Runtime = 'headless',
    [ValidateSet('control', 'wrapped')][string]$Mode = 'wrapped',
    [ValidateSet('combat_test', 'falling_skyway')][string]$World = 'combat_test',
    [string]$Profile
)
$ErrorActionPreference = 'Stop'
$receiptData = Get-Content -LiteralPath $Receipt -Raw | ConvertFrom-Json
if ($receiptData.runtime -ne 'attribution') { throw 'Attribution build receipt required.' }
$binary = (Resolve-Path -LiteralPath $receiptData.executable).Path
$ContentRoot = (Resolve-Path -LiteralPath $ContentRoot).Path
$runRoot = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Output)
if (Test-Path -LiteralPath $runRoot) { throw 'Run directory already exists.' }
$verifyArgs = @((Join-Path $PSScriptRoot 'profile-provenance.mjs'), 'verify', $ContentRoot, $Receipt, $binary)
$verified = & node @verifyArgs
if ($LASTEXITCODE -ne 0 -or -not ($verified | ConvertFrom-Json).verified) { throw 'Build receipt verification failed.' }
if (Get-Process -Name cargo,rustc,cl,link,lld-link,trunk -ErrorAction SilentlyContinue) { throw 'Build process running; wait without stopping unrelated work.' }
$hardwareData = Get-Content -LiteralPath $Hardware -Raw | ConvertFrom-Json
if (-not $hardwareData.gpu -or -not $hardwareData.driver -or -not $hardwareData.powerMode -or -not $hardwareData.displays) { throw 'Complete hardware JSON required.' }
New-Item -ItemType Directory -Path $runRoot, (Join-Path $runRoot 'appdata') | Out-Null
New-Item -ItemType Junction -Path (Join-Path $runRoot 'assets') -Target (Join-Path $ContentRoot 'assets') | Out-Null
$arguments = @($Runtime, $Mode, "assets/worlds/$World.toml", (Join-Path $runRoot 'artifact'))
$profileHash = $null
if ($Runtime -eq 'native') {
    $Profile = (Resolve-Path -LiteralPath $Profile).Path
    $profileHash = (Get-FileHash -LiteralPath $Profile -Algorithm SHA256).Hash
    Copy-Item -LiteralPath $Profile -Destination (Join-Path $runRoot 'profile.toml')
    if ((Get-FileHash -LiteralPath (Join-Path $runRoot 'profile.toml') -Algorithm SHA256).Hash -ne $profileHash) { throw 'Profile changed while copying.' }
    $arguments += (Join-Path $runRoot 'profile.toml')
}
$manifest = [ordered]@{
    sourceRevision=$receiptData.sourceRevision; sourceClean=$true; buildReceiptVerified=$true
    binarySha256=$receiptData.binarySha256; pdbSha256=$receiptData.pdbSha256; contentSha256=$receiptData.contentSha256
    runtime=$Runtime; mode=$Mode; profile=$receiptData.profile; features=$receiptData.features; seed=42; world=$World
    hardware=$hardwareData; arguments=$arguments; profileSha256=$profileHash; isolatedState=$true
    harnessPid=$PID; maxBackgroundCpuCores=1; warmupSeconds=40; measureSeconds=30; condition='renderer'; stationIntent=@()
    startedUnixMs=[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds(); exitCode=$null
}
if ($Runtime -eq 'native') {
    $manifest.expectedWindows = @(& node (Join-Path $PSScriptRoot 'profile-windows.mjs') $Profile $Hardware | ConvertFrom-Json)
    if ($LASTEXITCODE -ne 0) { throw 'Monitor profile does not match hardware provenance.' }
}
function Quote-NativeArgument([string]$Value) {
    if ($Value -notmatch '[\s"]') { return $Value }
    return '"' + [regex]::Replace([regex]::Replace($Value, '(\\*)"', '$1$1\"'), '(\\+)$', '$1$1') + '"'
}
$environment = @{ APPDATA=(Join-Path $runRoot 'appdata'); BEVY_ASSET_ROOT=$runRoot; GITHUB_SHA=$receiptData.sourceRevision; RUST_LOG='warn,bevy_render::renderer=info' }
$previous = @{}
$samples = [Collections.Generic.List[object]]::new()
$child = $null
try {
    foreach ($key in $environment.Keys) {
        $previous[$key] = [Environment]::GetEnvironmentVariable($key, 'Process')
        [Environment]::SetEnvironmentVariable($key, $environment[$key], 'Process')
    }
    $manifest.startedUnixMs = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
    $watch = [Diagnostics.Stopwatch]::StartNew()
    $child = Start-Process -FilePath $binary -ArgumentList (($arguments | ForEach-Object { Quote-NativeArgument $_ }) -join ' ') -WorkingDirectory $runRoot -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $runRoot 'stdout.log') -RedirectStandardError (Join-Path $runRoot 'stderr.log')
    $manifest.pid = $child.Id
    while (-not $child.HasExited -and $watch.Elapsed.TotalSeconds -lt 300) {
        $all = @(Get-Process -ErrorAction SilentlyContinue | ForEach-Object {
            try { [pscustomobject]@{ id=$_.Id; name=$_.ProcessName; cpuSeconds=$_.CPU; startedUtc=$_.StartTime.ToUniversalTime().ToString('o') } } catch { }
        })
        $samples.Add([pscustomobject]@{
            elapsedSeconds=$watch.Elapsed.TotalSeconds; privateBytes=$child.PrivateMemorySize64
            competingCompilers=@($all | Where-Object { $_.name -in @('cargo','rustc','cl','link','lld-link','trunk') }); processes=$all
        })
        Start-Sleep -Milliseconds 500
        $child.Refresh()
    }
} finally {
    foreach ($key in $previous.Keys) { [Environment]::SetEnvironmentVariable($key, $previous[$key], 'Process') }
    if ($child) {
        if (-not $child.HasExited) { $child.Kill(); $child.WaitForExit(); $manifest.timedOut=$true }
        $child.WaitForExit()
        $manifest.exitCode = $child.ExitCode
    }
    $verifiedAfter = & node @verifyArgs
    $manifest.provenanceUnchanged = $LASTEXITCODE -eq 0 -and ($verifiedAfter | ConvertFrom-Json).verified
    if ($Runtime -eq 'native') {
        $manifest.provenanceUnchanged = $manifest.provenanceUnchanged -and (Get-FileHash -LiteralPath (Join-Path $runRoot 'profile.toml') -Algorithm SHA256).Hash -eq $profileHash
        if (Test-Path -LiteralPath (Join-Path $runRoot 'artifact/frames.json')) { Copy-Item -LiteralPath (Join-Path $runRoot 'artifact/frames.json') -Destination (Join-Path $runRoot 'frames.json') }
    }
    ConvertTo-Json -InputObject @($samples.ToArray()) -Depth 8 | Set-Content -LiteralPath (Join-Path $runRoot 'process-samples.json') -Encoding UTF8
    $manifest | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath (Join-Path $runRoot 'manifest.json') -Encoding UTF8
    Copy-Item -LiteralPath $Receipt -Destination (Join-Path $runRoot 'build-receipt.json')
}
& node (Join-Path $PSScriptRoot 'profile-systems-analysis.mjs') $runRoot
if ($LASTEXITCODE -ne 0) { throw 'Attribution capture is diagnostic only; inspect summary.json.' }
