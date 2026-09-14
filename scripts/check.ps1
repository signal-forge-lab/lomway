[CmdletBinding()]
param(
    [string]$Config = 'config\proxy.local.toml'
)

. (Join-Path $PSScriptRoot 'common.ps1')
$configPath = if ([IO.Path]::IsPathRooted($Config)) { $Config } else { Join-Path $Script:RepoRoot $Config }
if (-not (Test-Path -LiteralPath $configPath -PathType Leaf)) {
    throw "Local gateway config not found: $configPath"
}

Push-Location $Script:RepoRoot
try {
    & cargo run --locked -- check --config $configPath
    if ($LASTEXITCODE -ne 0) { throw 'Gateway config check failed.' }
} finally {
    Pop-Location
}
