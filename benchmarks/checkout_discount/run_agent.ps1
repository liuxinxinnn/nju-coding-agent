[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
$benchmarkRoot = $PSScriptRoot
$repoRoot = (Resolve-Path (Join-Path $benchmarkRoot "..\..")).Path
$workspace = Join-Path $benchmarkRoot "workspace"
$logPath = Join-Path $benchmarkRoot "last-run.log"
$fixtureTest = Join-Path $benchmarkRoot "fixture\tests\test_checkout.py"
$workspaceTest = Join-Path $workspace "tests\test_checkout.py"

& (Join-Path $benchmarkRoot "prepare.ps1")

if ([string]::IsNullOrWhiteSpace($env:DEEPSEEK_API_KEY)) {
    throw "DEEPSEEK_API_KEY is not set. Set a newly generated key in this terminal first."
}

Push-Location $repoRoot
try {
    cargo build --offline
    if ($LASTEXITCODE -ne 0) {
        throw "cargo build failed with exit code $LASTEXITCODE"
    }
}
finally {
    Pop-Location
}

$agent = Join-Path $repoRoot "target\debug\nju-coding-agent.exe"
$task = Get-Content -LiteralPath (Join-Path $workspace "TASK.md") -Raw -Encoding UTF8
$timer = [System.Diagnostics.Stopwatch]::StartNew()

$previousErrorActionPreference = $ErrorActionPreference
$ErrorActionPreference = "Continue"
try {
    & $agent --yes --workspace $workspace $task 2>&1 | Tee-Object -FilePath $logPath
    $agentExitCode = $LASTEXITCODE
}
finally {
    $ErrorActionPreference = $previousErrorActionPreference
}
$timer.Stop()

$trace = Get-Content -LiteralPath $logPath -ErrorAction SilentlyContinue
$stepCount = @($trace | Select-String -Pattern '^\[step \d+\] thinking').Count
$toolCallCount = @($trace | Select-String -Pattern '^\[step \d+\] [a-z_]+ ').Count
$testsUnchanged = (Get-FileHash -LiteralPath $fixtureTest -Algorithm SHA256).Hash -eq
    (Get-FileHash -LiteralPath $workspaceTest -Algorithm SHA256).Hash

Push-Location $workspace
try {
    python -m unittest discover -s tests -v
    $testExitCode = $LASTEXITCODE
}
finally {
    Pop-Location
}

Write-Output "Agent exit code: $agentExitCode"
Write-Output "Test exit code: $testExitCode"
Write-Output "Agent steps: $stepCount"
Write-Output "Tool calls: $toolCallCount"
Write-Output ("Elapsed seconds: {0:N2}" -f $timer.Elapsed.TotalSeconds)
Write-Output "Tests unchanged: $testsUnchanged"
Write-Output "Agent trace: $logPath"

if ($agentExitCode -ne 0 -or $testExitCode -ne 0 -or -not $testsUnchanged) {
    exit 1
}
