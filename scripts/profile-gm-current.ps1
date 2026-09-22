param([Parameter(Mandatory)][string]$Output, [ValidateSet('idle','running','observed','controlled','raster')][string]$Stage = 'idle', [switch]$Diagnostic, [ValidateSet('measure','release')][string]$Profile = 'measure')
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$outDir = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Output)
if (Test-Path -LiteralPath $outDir) { throw 'Capture output already exists' }
if (Get-Process -Name cargo,rustc,cl,link,lld-link,trunk,pasm,uv -ErrorAction SilentlyContinue) { throw 'Wait for compilation and validation to finish' }
New-Item -ItemType Directory -Path $outDir, (Join-Path $outDir 'appdata') | Out-Null
$binary = Join-Path $root "target/$Profile/examples/profile_gm.exe"
$manifest = [ordered]@{
    sourceRevision=(& git rev-parse HEAD); sourceClean=$false; configuration="$Profile / ultralight / current uncommitted tree"; diagnostic=[bool]$Diagnostic
    binarySha256=(Get-FileHash $binary -Algorithm SHA256).Hash
    scenario='combat_test'; stage=$Stage; seed=42; resolution=@(1920,1080); warmupSeconds=40; measureSeconds=60
    cpu=@(Get-CimInstance Win32_Processor | Select-Object Name,NumberOfLogicalProcessors)
    gpu=@(Get-CimInstance Win32_VideoController | Select-Object Name,DriverVersion,CurrentHorizontalResolution,CurrentVerticalResolution,CurrentRefreshRate)
    powerMode=(& powercfg /getactivescheme)
    modifiedFiles=@(& git status --porcelain)
    sourceFiles=@(Get-ChildItem gui,src,examples -Recurse -File | Where-Object Extension -In '.rs','.js','.css' | ForEach-Object { @{path=$_.FullName; sha256=(Get-FileHash $_.FullName -Algorithm SHA256).Hash} })
}
$manifest | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $outDir 'manifest.json')
$names = @('PHOENIX_GM_DIAGNOSTIC','PHOENIX_GM_PROFILE_STAGE','PHOENIX_FRAME_CAPTURE','PHOENIX_FRAME_CAPTURE_SECONDS','PHOENIX_FRAME_EXPERIMENTS','BEVY_ASSET_ROOT','APPDATA','LOCALAPPDATA','RUST_LOG')
$saved = @{}
foreach ($name in $names) { $saved[$name] = [Environment]::GetEnvironmentVariable($name, 'Process') }
try {
    $env:PHOENIX_FRAME_CAPTURE = Join-Path $outDir 'frames.json'
    $env:PHOENIX_FRAME_CAPTURE_SECONDS = '135'
    $env:PHOENIX_GM_PROFILE_STAGE = $Stage
    $env:PHOENIX_GM_DIAGNOSTIC = if ($Diagnostic) { '1' } else { '0' }
    $env:PHOENIX_FRAME_EXPERIMENTS = ''
    $env:BEVY_ASSET_ROOT = $root
    $env:APPDATA = Join-Path $outDir 'appdata'
    $env:LOCALAPPDATA = Join-Path $outDir 'appdata'
    # Acceptance captures must not perform per-frame log-file writes.
    $env:RUST_LOG = 'warn,bevy_render::renderer=info'
    $child = Start-Process -FilePath $binary -WorkingDirectory $root -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $outDir 'stdout.log') -RedirectStandardError (Join-Path $outDir 'stderr.log')
    Write-Output "Capture PID $($child.Id), output $outDir"
    $previousMarker = ''
    while (-not $child.WaitForExit(10000)) {
        if (Test-Path (Join-Path $outDir 'stages.jsonl')) {
            $marker = Get-Content (Join-Path $outDir 'stages.jsonl') -Tail 1
            if ($marker -ne $previousMarker) { Write-Output $marker; $previousMarker = $marker }
        }
    }
    Write-Output "Exit code: $($child.ExitCode)"
} finally {
    foreach ($name in $names) { [Environment]::SetEnvironmentVariable($name, $saved[$name], 'Process') }
}
