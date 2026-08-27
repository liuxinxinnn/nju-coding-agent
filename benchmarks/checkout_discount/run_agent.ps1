[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
$utf8 = New-Object System.Text.UTF8Encoding($false)
[Console]::InputEncoding = $utf8
[Console]::OutputEncoding = $utf8
$OutputEncoding = $utf8
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
$stepCount = @($trace | Select-String -Pattern '^\[(PLAN|EXEC|VERIFY) step \d+\] thinking').Count
$toolCallCount = @($trace | Select-String -Pattern '^\[(PLAN|EXEC|VERIFY) step \d+\] [a-z_]+ ').Count
$phaseSequence = @(
    $trace | Select-String -Pattern '^\[phase:(PLAN|EXEC|VERIFY|DONE)\]$' | ForEach-Object {
        $_.Matches[0].Groups[1].Value
    }
) -join ' -> '
$revisionMatches = @($trace | Select-String -Pattern '^\[revision\] (\d+) ')
$workspaceRevision = if ($revisionMatches.Count -eq 0) {
    0
}
else {
    [int]$revisionMatches[-1].Matches[0].Groups[1].Value
}
$doneReached = @($trace | Select-String -Pattern '^\[phase:DONE\]$').Count -gt 0
$currentRevisionVerified = $workspaceRevision -eq 0 -or @(
    $trace | Select-String -Pattern "^\[verify:PASS\] revision $workspaceRevision "
).Count -gt 0
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
Write-Output "Phase sequence: $phaseSequence"
Write-Output "Workspace revision: $workspaceRevision"
Write-Output "Current revision verified: $currentRevisionVerified"
Write-Output "DONE reached: $doneReached"
Write-Output ("Elapsed seconds: {0:N2}" -f $timer.Elapsed.TotalSeconds)
Write-Output "Tests unchanged: $testsUnchanged"
Write-Output "Agent trace: $logPath"

if (
    $agentExitCode -ne 0 -or
    $testExitCode -ne 0 -or
    -not $testsUnchanged -or
    -not $doneReached -or
    -not $currentRevisionVerified
) {
    exit 1
}
