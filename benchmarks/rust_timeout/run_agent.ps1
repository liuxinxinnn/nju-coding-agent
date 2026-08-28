[CmdletBinding()]
param()

& (Join-Path $PSScriptRoot "..\run_case.ps1") `
    -BenchmarkRoot $PSScriptRoot `
    -TestKind "rust" `
    -ProtectedFiles @("tests\timeout_tests.rs") `
    -RequiredChangedFiles @("src\lib.rs")
exit $LASTEXITCODE
