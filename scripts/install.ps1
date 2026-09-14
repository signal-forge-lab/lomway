[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
Push-Location $repo
try {
    $versionText = (& rustc --version | Out-String).Trim()
    if ($LASTEXITCODE -ne 0 -or $versionText -notmatch '^rustc\s+(\d+)\.(\d+)\.') {
        throw 'Rust toolchain was not found.'
    }
    $major = [int]$Matches[1]
    $minor = [int]$Matches[2]
    if ($major -lt 1 -or ($major -eq 1 -and $minor -lt 90)) {
        throw "Rust 1.90+ is required; found: $versionText"
    }

    & cargo build --release --locked
    if ($LASTEXITCODE -ne 0) { throw 'cargo build --release --locked failed.' }
    Write-Output 'INSTALL_OK'
} finally {
    Pop-Location
}
