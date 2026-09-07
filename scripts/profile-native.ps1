[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Receipt,
    [Parameter(Mandatory)][string]$ContentRoot,
    [Parameter(Mandatory)][string]$Profile,
    [Parameter(Mandatory)][string]$Hardware,
    [Parameter(Mandatory)][string]$Output,
    [string]$ClientDirectory,
    [ValidateSet('combat_test', 'falling_skyway')][string]$World = 'combat_test',
    [ValidateSet('renderer', 'chrome', 'one', 'two')][string]$Condition = 'renderer',
    [ValidateRange(40, 3600)][int]$WarmupSeconds = 40,
    [ValidateRange(30, 3600)][int]$MeasureSeconds = 30,
    [ValidateRange(1024, 65535)][int]$Port = 18180,
    [string]$Experiment = '',
    [ValidateRange(0, 256)][double]$MaxBackgroundCpuCores = 1,
    [switch]$DryRun
)
$ErrorActionPreference = 'Stop'
$receiptData = Get-Content -LiteralPath $Receipt -Raw | ConvertFrom-Json
$binary = (Resolve-Path -LiteralPath $receiptData.executable).Path
$ContentRoot = (Resolve-Path -LiteralPath $ContentRoot).Path
$Profile = (Resolve-Path -LiteralPath $Profile).Path
$runRoot = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Output)
if (Test-Path -LiteralPath $runRoot) { throw 'Run directory already exists.' }
if ($receiptData.runtime -ne 'native') { throw 'Native build receipt required.' }
$verifyArgs = @((Join-Path $PSScriptRoot 'profile-provenance.mjs'), 'verify', $ContentRoot, $Receipt, $binary)
if ($Condition -ne 'renderer') {
    $ClientDirectory = (Resolve-Path -LiteralPath $ClientDirectory).Path
    $verifyArgs += $ClientDirectory
    $instrumentation = Get-Content -LiteralPath (Join-Path $ClientDirectory 'review-instrumentation.json') -Raw | ConvertFrom-Json
}
$verified = & node @verifyArgs
if ($LASTEXITCODE -ne 0 -or -not ($verified | ConvertFrom-Json).verified) { throw 'Build receipt verification failed.' }
$hardwareData = Get-Content -LiteralPath $Hardware -Raw | ConvertFrom-Json
if (-not $hardwareData.gpu -or -not $hardwareData.driver -or -not $hardwareData.powerMode -or -not $hardwareData.displays) {
    throw 'Hardware JSON must record gpu, driver, powerMode, and displays (physical width/height/DPI/monitor identity).'
}
$privateContent = Join-Path $runRoot 'content'
$arguments = @('--content-dir', $privateContent, '--world', "assets/worlds/$World.toml",
    '--ship', 'assets/entities/alliance_destroyer.toml', '--seed', '42', '--solo',
    '--profile', (Join-Path $runRoot 'profile.toml'), '--addr', "127.0.0.1:$Port", '--save-dir', (Join-Path $runRoot 'saves'),
    '--frame-stats', '--log', 'info')
if ($Condition -ne 'renderer') { $arguments += @('--client-dir', $ClientDirectory) }
$manifest = [ordered]@{
    sourceRevision=$receiptData.sourceRevision; sourceClean=$true; buildReceiptVerified=$true
    binarySha256=$receiptData.binarySha256; pdbSha256=$receiptData.pdbSha256
    contentSha256=$receiptData.contentSha256; bundleSha256=$receiptData.bundleSha256
    buildProfile=$receiptData.profile; features=$receiptData.features; seed=42
    world=$World; condition=$Condition; experiment=$Experiment; arguments=$arguments
    profileSha256=(Get-FileHash -LiteralPath $Profile -Algorithm SHA256).Hash
    hardware=$hardwareData; warmupSeconds=$WarmupSeconds; measureSeconds=$MeasureSeconds
    isolatedState=$true; completedObservation=$false; frameCaptureComplete=$false; exitCode=$null
    harnessPid=$PID; maxBackgroundCpuCores=$MaxBackgroundCpuCores
    stationIntent=@(switch ($Condition) { 'one' { 'helm' } 'two' { 'helm'; 'tactical' } })
}
if ($DryRun) { $manifest | ConvertTo-Json -Depth 10; return }
if (Get-Process -Name cargo,rustc,cl,link,lld-link,trunk -ErrorAction SilentlyContinue) {
    throw 'A compiler/build process is running. Wait for it; do not stop unrelated work.'
}
New-Item -ItemType Directory -Path $runRoot, $privateContent, (Join-Path $runRoot 'saves'), (Join-Path $runRoot 'appdata') | Out-Null
# pin_content_root changes the child CWD. Give SDK logs their own directory
# while keeping the hashed authored assets immutable and shared.
New-Item -ItemType Junction -Path (Join-Path $privateContent 'assets') -Target (Join-Path $ContentRoot 'assets') | Out-Null
New-Item -ItemType Junction -Path (Join-Path $privateContent 'resources') -Target (Join-Path (Split-Path -Parent $binary) 'sdk-resources') | Out-Null
Copy-Item -LiteralPath $Profile -Destination (Join-Path $runRoot 'profile.toml')
if ((Get-FileHash -LiteralPath (Join-Path $runRoot 'profile.toml') -Algorithm SHA256).Hash -ne $manifest.profileSha256) {
    throw 'Monitor profile changed during preparation.'
}
$manifest.expectedWindows = @(& node (Join-Path $PSScriptRoot 'profile-windows.mjs') $Profile $Hardware | ConvertFrom-Json)
if ($LASTEXITCODE -ne 0) { throw 'Monitor profile does not match hardware provenance.' }
Copy-Item -LiteralPath $Receipt -Destination (Join-Path $runRoot 'build-receipt.json')
function Quote-NativeArgument([string]$Value) {
    if ($Value -notmatch '[\s"]') { return $Value }
    return '"' + [regex]::Replace([regex]::Replace($Value, '(\\*)"', '$1$1\"'), '(\\+)$', '$1$1') + '"'
}
function Process-Snapshot {
    @(Get-Process -ErrorAction SilentlyContinue | ForEach-Object {
        try { [pscustomobject]@{ id=$_.Id; name=$_.ProcessName; cpuSeconds=$_.CPU; startedUtc=$_.StartTime.ToUniversalTime().ToString('o') } } catch { }
    })
}
$samples = [Collections.Generic.List[object]]::new()
$diagnostics = [Collections.Generic.List[object]]::new()
$environment = @{
    PHOENIX_FRAME_CAPTURE=(Join-Path $runRoot 'frames.json')
    PHOENIX_FRAME_CAPTURE_SECONDS=[string]($WarmupSeconds + $MeasureSeconds)
    PHOENIX_FRAME_EXPERIMENTS=$Experiment
    GITHUB_SHA=$receiptData.sourceRevision
    BEVY_ASSET_ROOT=$privateContent
    PHOENIX_PERF_DEVICE=($hardwareData.gpu + ' / ' + $hardwareData.driver)
    APPDATA=(Join-Path $runRoot 'appdata')
    RUST_LOG='warn,lobby=info,bevy_render::renderer=info'
}
$previous = @{}
$listener = $null
$child = $null
$watch = [Diagnostics.Stopwatch]::new()
try {
    if ($Condition -ne 'renderer') {
        $listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, $instrumentation.diagnosticPort)
        $listener.Start()
    }
    foreach ($key in $environment.Keys) {
        $previous[$key] = [Environment]::GetEnvironmentVariable($key, 'Process')
        [Environment]::SetEnvironmentVariable($key, $environment[$key], 'Process')
    }
    $manifest.startedUnixMs = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
    $watch.Start()
    $child = Start-Process -FilePath $binary -ArgumentList (($arguments | ForEach-Object { Quote-NativeArgument $_ }) -join ' ') -WorkingDirectory $runRoot -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $runRoot 'stdout.log') -RedirectStandardError (Join-Path $runRoot 'stderr.log')
    $manifest.pid = $child.Id
    while (-not $child.HasExited -and $watch.Elapsed.TotalSeconds -lt ($WarmupSeconds + $MeasureSeconds + 120)) {
        $all = @(Process-Snapshot)
        $samples.Add([pscustomobject]@{
            elapsedSeconds=$watch.Elapsed.TotalSeconds; cpuSeconds=$child.TotalProcessorTime.TotalSeconds
            privateBytes=$child.PrivateMemorySize64; workingSetBytes=$child.WorkingSet64
            competingCompilers=@($all | Where-Object { $_.name -in @('cargo','rustc','cl','link','lld-link','trunk') })
            processes=$all
        })
        while ($listener -and $listener.Pending()) {
            $client = $listener.AcceptTcpClient()
            try {
                $client.ReceiveTimeout = 500
                $stream = $client.GetStream()
                $buffer = New-Object byte[] 16384
                $read = $stream.Read($buffer, 0, $buffer.Length)
                $first = ([Text.Encoding]::UTF8.GetString($buffer, 0, $read) -split "`r`n")[0]
                if ($first -match '^GET /\?state=([^ ]+) HTTP/') {
                    $diagnostics.Add([pscustomobject]@{ elapsedSeconds=$watch.Elapsed.TotalSeconds; data=([Uri]::UnescapeDataString($Matches[1]) | ConvertFrom-Json) })
                }
                $response = [Text.Encoding]::ASCII.GetBytes("HTTP/1.1 204 No Content`r`nAccess-Control-Allow-Origin: *`r`nContent-Length: 0`r`nConnection: close`r`n`r`n")
                $stream.Write($response, 0, $response.Length)
            } catch { $diagnostics.Add([pscustomobject]@{ error=$_.Exception.Message }) }
            finally { $client.Dispose() }
        }
        Start-Sleep -Milliseconds 1000
        $child.Refresh()
    }
} finally {
    foreach ($key in $previous.Keys) { [Environment]::SetEnvironmentVariable($key, $previous[$key], 'Process') }
    if ($listener) { $listener.Stop() }
    if ($child) {
        $child.Refresh()
        if (-not $child.HasExited) {
            $manifest.shutdown = 'Own process exceeded bound; graceful close requested'
            $null = $child.CloseMainWindow()
            $null = $child.WaitForExit(10000)
            if (-not $child.HasExited) { $child.Kill(); $child.WaitForExit(); $manifest.shutdown = 'Own process exceeded bound and was terminated' }
        } else { $manifest.shutdown = 'Host exited' }
        $child.WaitForExit()
        $manifest.exitCode = $child.ExitCode
    }
    $manifest.finishedUtc = [DateTime]::UtcNow.ToString('o')
    $verifiedAfter = & node @verifyArgs
    $manifest.provenanceUnchanged = $LASTEXITCODE -eq 0 -and ($verifiedAfter | ConvertFrom-Json).verified -and (Get-FileHash -LiteralPath (Join-Path $runRoot 'profile.toml') -Algorithm SHA256).Hash -eq $manifest.profileSha256
    ConvertTo-Json -InputObject @($samples.ToArray()) -Depth 8 | Set-Content -LiteralPath (Join-Path $runRoot 'process-samples.json') -Encoding UTF8
    ConvertTo-Json -InputObject @($diagnostics.ToArray()) -Depth 8 | Set-Content -LiteralPath (Join-Path $runRoot 'pane-diagnostics.json') -Encoding UTF8
    $manifest | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath (Join-Path $runRoot 'manifest.json') -Encoding UTF8
}
& node (Join-Path $PSScriptRoot 'profile-analysis.mjs') $runRoot
if ($LASTEXITCODE -ne 0) { throw "Capture is diagnostic only; inspect $runRoot/summary.json" }
