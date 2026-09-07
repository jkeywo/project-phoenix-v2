[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Receipt,
    [Parameter(Mandatory)][string]$ContentRoot,
    [Parameter(Mandatory)][string]$ProfileDirectory,
    [Parameter(Mandatory)][string]$Hardware,
    [Parameter(Mandatory)][string]$ClientDirectory,
    [Parameter(Mandatory)][string]$Output,
    [ValidateRange(1, 10)][int]$Repetitions = 3
)
$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath($Output)
if (Test-Path -LiteralPath $root) { throw 'Matrix directory already exists.' }
$tasks = & node (Join-Path $PSScriptRoot 'profile-matrix.mjs') $Repetitions | ConvertFrom-Json
if ($LASTEXITCODE -ne 0) { throw 'Could not prepare matrix.' }
New-Item -ItemType Directory -Path $root | Out-Null
$tasks | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $root 'matrix.json') -Encoding UTF8
$index = 0
foreach ($task in $tasks) {
    $profileName = if ($task.condition -in @('renderer','chrome')) { 'zero' } else { $task.condition }
    $name = '{0:d2}-{1}-{2}-{3}-{4}' -f $index, $task.world, $task.condition, $task.repetition, $task.control
    & (Join-Path $PSScriptRoot 'profile-native.ps1') -Receipt $Receipt -ContentRoot $ContentRoot -Profile (Join-Path $ProfileDirectory "$profileName.toml") -Hardware $Hardware -ClientDirectory $ClientDirectory -World $task.world -Condition $task.condition -Output (Join-Path $root $name)
    $index++
}
