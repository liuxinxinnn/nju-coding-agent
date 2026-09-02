[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
$utf8 = New-Object System.Text.UTF8Encoding($false)
[Console]::InputEncoding = $utf8
[Console]::OutputEncoding = $utf8
$OutputEncoding = $utf8

$benchmarkRoot = $PSScriptRoot
$fixture = Join-Path $benchmarkRoot "fixture"
$workspace = Join-Path $benchmarkRoot "workspace"

if (-not (Test-Path -LiteralPath $workspace -PathType Container)) {
    throw "Video demo workspace not found. Run prepare.ps1 first: $workspace"
}

Write-Output "[1/2] Running an independent full test suite..."
Push-Location $workspace
try {
    python -m unittest discover -s tests -v
    $testExitCode = $LASTEXITCODE
}
finally {
    Pop-Location
}

function Test-SameFile([string]$RelativePath) {
    $fixtureFile = Join-Path $fixture $RelativePath
    $workspaceFile = Join-Path $workspace $RelativePath
    if (-not (Test-Path -LiteralPath $workspaceFile -PathType Leaf)) {
        return $false
    }
    return (Get-FileHash -LiteralPath $fixtureFile -Algorithm SHA256).Hash -eq
        (Get-FileHash -LiteralPath $workspaceFile -Algorithm SHA256).Hash
}

$testsUnchanged = Test-SameFile "tests\test_order.py"
$modelsChanged = -not (Test-SameFile "order\models.py")
$serviceChanged = -not (Test-SameFile "order\service.py")
$passed = $testExitCode -eq 0 -and $testsUnchanged -and $modelsChanged -and $serviceChanged

Write-Output ""
Write-Output "[2/2] Checking benchmark invariants..."
Write-Output "Tests exit code: $testExitCode"
Write-Output "Tests unchanged: $testsUnchanged"
Write-Output "order/models.py changed: $modelsChanged"
Write-Output "order/service.py changed: $serviceChanged"
Write-Output "VIDEO_DEMO_RESULT: $(if ($passed) { 'PASS' } else { 'FAIL' })"

if (-not $passed) {
    exit 1
}
