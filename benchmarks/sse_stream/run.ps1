[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
$utf8 = New-Object System.Text.UTF8Encoding($false)
[Console]::InputEncoding = $utf8
[Console]::OutputEncoding = $utf8
$OutputEncoding = $utf8
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$targetDir = Join-Path $repoRoot "target-benchmark-agent"
$logPath = Join-Path $PSScriptRoot "last-run.log"

if ([string]::IsNullOrWhiteSpace($env:DEEPSEEK_API_KEY)) {
    throw "DEEPSEEK_API_KEY is not set. Set a newly generated key in this terminal first."
}

Push-Location $repoRoot
try {
    cargo build --offline --target-dir $targetDir --example sse_smoke
    if ($LASTEXITCODE -ne 0) {
        throw "cargo build failed with exit code $LASTEXITCODE"
    }
    $example = Join-Path $targetDir "debug\examples\sse_smoke.exe"
    $previousErrorActionPreference = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try {
        & $example 2>&1 | Tee-Object -FilePath $logPath
        $exitCode = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $previousErrorActionPreference
    }
}
finally {
    Pop-Location
}

Write-Output "SSE smoke exit code: $exitCode"
Write-Output "SSE evidence: $logPath"
exit $exitCode
