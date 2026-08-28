[CmdletBinding()]
param()

& (Join-Path $PSScriptRoot "..\run_case.ps1") `
    -BenchmarkRoot $PSScriptRoot `
    -TestKind "python-unittest" `
    -ProtectedFiles @("tests\test_order.py") `
    -RequiredChangedFiles @("order\models.py", "order\service.py")
exit $LASTEXITCODE
