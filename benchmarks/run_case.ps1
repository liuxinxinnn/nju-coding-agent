[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$BenchmarkRoot,

    [Parameter(Mandatory = $true)]
    [ValidateSet("python-unittest", "rust")]
    [string]$TestKind,

    [Parameter(Mandatory = $true)]
    [string[]]$ProtectedFiles,

    [Parameter(Mandatory = $true)]
    [string[]]$RequiredChangedFiles
)

$ErrorActionPreference = "Stop"
$utf8 = New-Object System.Text.UTF8Encoding($false)
[Console]::InputEncoding = $utf8
[Console]::OutputEncoding = $utf8
$OutputEncoding = $utf8

$benchmarkRoot = (Resolve-Path -LiteralPath $BenchmarkRoot).Path
$repoRoot = (Resolve-Path (Join-Path $benchmarkRoot "..\..")).Path
$workspace = Join-Path $benchmarkRoot "workspace"
$logPath = Join-Path $benchmarkRoot "last-run.log"
$jsonPath = Join-Path $benchmarkRoot "last-run.json"
$agentTargetDir = Join-Path $repoRoot "target-benchmark-agent"

& (Join-Path $benchmarkRoot "prepare.ps1")

if ([string]::IsNullOrWhiteSpace($env:DEEPSEEK_API_KEY)) {
    throw "DEEPSEEK_API_KEY is not set. Set a newly generated key in this terminal first."
}

Push-Location $repoRoot
try {
    cargo build --offline --target-dir $agentTargetDir
    if ($LASTEXITCODE -ne 0) {
        throw "cargo build failed with exit code $LASTEXITCODE"
    }
}
finally {
    Pop-Location
}

$agent = Join-Path $agentTargetDir "debug\nju-coding-agent.exe"
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

$testsUnchanged = $true
foreach ($relativePath in $ProtectedFiles) {
    $fixtureFile = Join-Path (Join-Path $benchmarkRoot "fixture") $relativePath
    $workspaceFile = Join-Path $workspace $relativePath
    if (
        -not (Test-Path -LiteralPath $workspaceFile -PathType Leaf) -or
        (Get-FileHash -LiteralPath $fixtureFile -Algorithm SHA256).Hash -ne
            (Get-FileHash -LiteralPath $workspaceFile -Algorithm SHA256).Hash
    ) {
        $testsUnchanged = $false
        break
    }
}

$requiredFilesChanged = $true
foreach ($relativePath in $RequiredChangedFiles) {
    $fixtureFile = Join-Path (Join-Path $benchmarkRoot "fixture") $relativePath
    $workspaceFile = Join-Path $workspace $relativePath
    if (
        -not (Test-Path -LiteralPath $workspaceFile -PathType Leaf) -or
        (Get-FileHash -LiteralPath $fixtureFile -Algorithm SHA256).Hash -eq
            (Get-FileHash -LiteralPath $workspaceFile -Algorithm SHA256).Hash
    ) {
        $requiredFilesChanged = $false
        break
    }
}

Push-Location $workspace
try {
    switch ($TestKind) {
        "python-unittest" {
            python -m unittest discover -s tests -v
        }
        "rust" {
            cargo test --offline
        }
    }
    $testExitCode = $LASTEXITCODE
}
finally {
    Pop-Location
}

$result = [ordered]@{
    benchmark = Split-Path -Leaf $benchmarkRoot
    timestamp = [DateTimeOffset]::Now.ToString("o")
    model = if ([string]::IsNullOrWhiteSpace($env:DEEPSEEK_MODEL)) { "deepseek-v4-flash" } else { $env:DEEPSEEK_MODEL }
    agent_exit_code = $agentExitCode
    test_exit_code = $testExitCode
    agent_steps = $stepCount
    tool_calls = $toolCallCount
    phase_sequence = $phaseSequence
    workspace_revision = $workspaceRevision
    current_revision_verified = $currentRevisionVerified
    done_reached = $doneReached
    elapsed_seconds = [Math]::Round($timer.Elapsed.TotalSeconds, 2)
    tests_unchanged = $testsUnchanged
    required_files_changed = $requiredFilesChanged
}
$result | ConvertTo-Json | Set-Content -LiteralPath $jsonPath -Encoding UTF8

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
Write-Output "Required files changed: $requiredFilesChanged"
Write-Output "Agent trace: $logPath"
Write-Output "Machine-readable result: $jsonPath"

if (
    $agentExitCode -ne 0 -or
    $testExitCode -ne 0 -or
    -not $testsUnchanged -or
    -not $requiredFilesChanged -or
    -not $doneReached -or
    -not $currentRevisionVerified
) {
    exit 1
}
