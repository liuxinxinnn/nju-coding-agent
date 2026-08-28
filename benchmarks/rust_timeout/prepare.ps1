[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
$benchmarkRoot = $PSScriptRoot
$fixture = Join-Path $benchmarkRoot "fixture"
$workspace = Join-Path $benchmarkRoot "workspace"

if (-not (Test-Path -LiteralPath $fixture -PathType Container)) {
    throw "Benchmark fixture not found: $fixture"
}

if (Test-Path -LiteralPath $workspace) {
    Remove-Item -LiteralPath $workspace -Recurse -Force
}
New-Item -ItemType Directory -Path $workspace -Force | Out-Null
Get-ChildItem -LiteralPath $fixture -Force | ForEach-Object {
    Copy-Item -LiteralPath $_.FullName -Destination $workspace -Recurse -Force
}

Write-Output "Benchmark workspace prepared: $workspace"
Write-Output "Expected baseline: 5 tests run, 2 failures caused by suffix parsing order."
