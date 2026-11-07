$ErrorActionPreference = 'Stop'

$projectRoot = Split-Path -Parent $PSScriptRoot
Push-Location $projectRoot
try {
    $checks = @(
        @{ Name = 'Format'; Arguments = @('fmt', '--all', '--', '--check') },
        @{ Name = 'Clippy'; Arguments = @('clippy', '--workspace', '--all-targets', '--all-features', '--', '-D', 'warnings') },
        @{ Name = 'Tests'; Arguments = @('test', '--workspace', '--all-features') },
        @{ Name = 'Coverage'; Arguments = @('llvm-cov', '--workspace', '--all-features', '--html', '--fail-under-lines', '95') }
    )

    $failed = @()
    foreach ($check in $checks) {
        Write-Host "`n=== $($check.Name) ==="
        & cargo @($check.Arguments)
        if ($LASTEXITCODE -ne 0) {
            $failed += $check.Name
        }
    }
    if ($failed.Count -gt 0) {
        Write-Error "Failed checks: $($failed -join ', ')" -ErrorAction Continue
        exit 1
    }
} finally {
    Pop-Location
}
