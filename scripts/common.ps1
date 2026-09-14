$ErrorActionPreference = 'Stop'

$Script:RepoRoot = Split-Path -Parent $PSScriptRoot
$Script:RuntimeDir = Join-Path $Script:RepoRoot 'runtime'
$Script:LogDir = Join-Path $Script:RepoRoot 'logs'
$Script:PidFile = Join-Path $Script:RuntimeDir 'gateway.pid'
$Script:Exe = Join-Path $Script:RepoRoot 'target\release\lomway.exe'

function Get-GatewayPid {
    if (-not (Test-Path -LiteralPath $Script:PidFile -PathType Leaf)) { return $null }
    $raw = (Get-Content -LiteralPath $Script:PidFile -Raw).Trim()
    if ($raw -notmatch '^\d+$') { return $null }
    return [int]$raw
}

function Get-GatewayProcess {
    $pidValue = Get-GatewayPid
    if (-not $pidValue) { return $null }
    $process = Get-Process -Id $pidValue -ErrorAction SilentlyContinue
    if (-not $process) { return $null }
    try {
        $actualPath = $process.Path
    } catch {
        return $null
    }
    if (-not $actualPath) { return $null }
    if ([IO.Path]::GetFullPath($actualPath) -ne [IO.Path]::GetFullPath($Script:Exe)) { return $null }
    return $process
}

function Assert-GatewayBinary {
    if (-not (Test-Path -LiteralPath $Script:Exe -PathType Leaf)) {
        throw "Gateway binary is missing. Run scripts\install.ps1 first: $Script:Exe"
    }
}
