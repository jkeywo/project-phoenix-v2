[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Receipt,
    [Parameter(Mandatory)][string]$ContentRoot,
    [Parameter(Mandatory)][string]$Hardware,
    [Parameter(Mandatory)][string]$Output,
    [ValidateRange(1, 10)][int]$Repetitions = 3
)
$ErrorActionPreference = 'Stop'
$receiptData = Get-Content -LiteralPath $Receipt -Raw | ConvertFrom-Json
if ($receiptData.runtime -ne 'headless') { throw 'Headless build receipt required.' }
$binary = (Resolve-Path -LiteralPath $receiptData.executable).Path
$ContentRoot = (Resolve-Path -LiteralPath $ContentRoot).Path
$root = [IO.Path]::GetFullPath($Output)
if (Test-Path -LiteralPath $root) { throw 'Output directory already exists.' }
$verified = & node (Join-Path $PSScriptRoot 'profile-provenance.mjs') verify $ContentRoot $Receipt $binary
if ($LASTEXITCODE -ne 0 -or -not ($verified | ConvertFrom-Json).verified) { throw 'Build receipt verification failed.' }
New-Item -ItemType Directory -Path $root | Out-Null
function Quote-NativeArgument([string]$Value) {
    if ($Value -notmatch '[\s"]') { return $Value }
    return '"' + [regex]::Replace([regex]::Replace($Value, '(\\*)"', '$1$1\"'), '(\\+)$', '$1$1') + '"'
}
for ($repetition = 0; $repetition -lt $Repetitions; $repetition++) {
    $worlds = if ($repetition % 2) { @('falling_skyway', 'combat_test') } else { @('combat_test', 'falling_skyway') }
    foreach ($world in $worlds) {
        if (Get-Process -Name cargo,rustc,cl,link,lld-link,trunk -ErrorAction SilentlyContinue) { throw 'Build process running; wait without stopping unrelated work.' }
        $runRoot = Join-Path $root "$world-$repetition"
        New-Item -ItemType Directory -Path $runRoot, (Join-Path $runRoot 'appdata') | Out-Null
        New-Item -ItemType Junction -Path (Join-Path $runRoot 'assets') -Target (Join-Path $ContentRoot 'assets') | Out-Null
        $arguments = @('--world', "assets/worlds/$world.toml", '--ship', 'assets/entities/alliance_destroyer.toml',
            '--seed', '42', '--sim-seconds', '60', '--log', 'off', '--perf-scenario', "review-$world",
            '--perf-capture', (Join-Path $runRoot 'capture.json'), '--report', (Join-Path $runRoot 'report.json'))
        $manifest = [ordered]@{
            sourceRevision=$receiptData.sourceRevision; binarySha256=$receiptData.binarySha256; contentSha256=$receiptData.contentSha256
            sourceClean=$true; buildReceiptVerified=$true; runtime='headless-native'; profile=$receiptData.profile; features=$receiptData.features
            hardware=(Get-Content -LiteralPath $Hardware -Raw | ConvertFrom-Json); world=$world; repetition=$repetition
            arguments=$arguments; seed=42; excludedUpdates=300; isolatedState=$true; harnessPid=$PID; maxBackgroundCpuCores=1
            startedUtc=[DateTime]::UtcNow.ToString('o'); exitCode=$null
        }
        $previous = @{}
        $environment = @{ APPDATA=(Join-Path $runRoot 'appdata'); BEVY_ASSET_ROOT=$runRoot; GITHUB_SHA=$receiptData.sourceRevision }
        $samples = [Collections.Generic.List[object]]::new()
        $child = $null
        try {
            foreach ($key in $environment.Keys) {
                $previous[$key] = [Environment]::GetEnvironmentVariable($key, 'Process')
                [Environment]::SetEnvironmentVariable($key, $environment[$key], 'Process')
            }
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
            $verifiedAfter = & node (Join-Path $PSScriptRoot 'profile-provenance.mjs') verify $ContentRoot $Receipt $binary
            $manifest.provenanceUnchanged = $LASTEXITCODE -eq 0 -and ($verifiedAfter | ConvertFrom-Json).verified
            ConvertTo-Json -InputObject @($samples.ToArray()) -Depth 8 | Set-Content -LiteralPath (Join-Path $runRoot 'process-samples.json') -Encoding UTF8
            $manifest | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath (Join-Path $runRoot 'manifest.json') -Encoding UTF8
        }
        if ($manifest.exitCode -ne 0) { throw "Headless run failed: $runRoot" }
    }
}
& node (Join-Path $PSScriptRoot 'profile-headless-analysis.mjs') $root
if ($LASTEXITCODE -ne 0) { throw 'Headless captures are diagnostic only; inspect summary.json.' }
