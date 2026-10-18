[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidatePattern('^LOMWAY_BACKEND_[A-Z0-9_]+$')]
    [string]$Name,
    [switch]$Rotate
)

# Generate a backend bearer token and store it ONLY in the canonical SOPS
# store, under the top-level key $Name. The token is never printed, never put
# on a command line, and never written to a repository file. The original
# encrypted document stays unchanged if anything fails.
#
# The same key feeds both sides: the gateway reads it as $Name (see
# bearer_token_env) and the backend launcher passes it to the backend as that
# backend's own token variable.
$ErrorActionPreference = 'Stop'
$store = [Environment]::GetEnvironmentVariable('LOCAL_MCP_SOPS_STORE', 'Process')
if ([string]::IsNullOrWhiteSpace($store)) {
    $store = Join-Path $env:USERPROFILE '.config\sops\secrets\global.sops.json'
}
if (-not (Test-Path -LiteralPath $store -PathType Leaf)) { throw 'Canonical SOPS secrets file is unavailable.' }
$sops = (Get-Command sops -ErrorAction Stop).Source
$key = '["' + $Name + '"]'

$existing = (& $sops --decrypt --extract $key $store 2>$null) -join ''
$present = ($LASTEXITCODE -eq 0 -and $existing.Trim('"').Length -ge 32)
$existing = $null
if ($present -and -not $Rotate) {
    Write-Output ("{0}: already provisioned in SOPS (use -Rotate to replace it)." -f $Name)
    return
}

$originalHash = (Get-FileHash -LiteralPath $store -Algorithm SHA256).Hash
$staging = Join-Path ([IO.Path]::GetDirectoryName($store)) ('.lomway-token-' + [guid]::NewGuid().ToString('N') + '.json')
$backup = $staging + '.backup'
try {
    Copy-Item -LiteralPath $store -Destination $staging -ErrorAction Stop
    $token = [Convert]::ToHexString([Security.Cryptography.RandomNumberGenerator]::GetBytes(32)).ToLowerInvariant()
    $jsonValue = ConvertTo-Json -InputObject $token -Compress
    $jsonValue | & $sops set --value-stdin $staging $key | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'SOPS set failed.' }
    $verify = (& $sops --decrypt --extract $key $staging 2>$null) -join ''
    if ($LASTEXITCODE -ne 0 -or $verify.Trim('"') -ne $token) { throw 'Staged SOPS token verification failed.' }
    $verify = $null
    $jsonValue = $null
    $token = $null
    if ((Get-FileHash -LiteralPath $store -Algorithm SHA256).Hash -ne $originalHash) {
        throw 'SOPS document changed concurrently; refusing replacement.'
    }
    [IO.File]::Replace($staging, $store, $backup, $true)
    Write-Output ("{0}: encrypted in SOPS (64 random hex characters)." -f $Name)
} finally {
    $token = $null
    $verify = $null
    $jsonValue = $null
    Remove-Item -LiteralPath $staging -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $backup -Force -ErrorAction SilentlyContinue
}
