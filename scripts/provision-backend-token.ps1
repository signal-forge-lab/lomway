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
$lockPath = Join-Path (Split-Path -Parent $store) '.sops-write.lock'
$lockHandle = $null
try {
    for ($attempt = 0; $attempt -lt 50; $attempt++) {
        try {
            $lockHandle = [IO.FileStream]::new(
                $lockPath, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write,
                [IO.FileShare]::None
            )
            break
        } catch [IO.IOException] {
            if (-not (Test-Path -LiteralPath $lockPath -PathType Leaf)) { throw }
            Start-Sleep -Milliseconds 100
        }
    }
    if (-not $lockHandle) { throw 'SOPS write lock unavailable' }

$existing = (& $sops --decrypt --extract $key $store 2>$null) -join ''
$present = ($LASTEXITCODE -eq 0 -and $existing.Trim('"').Length -ge 32)
$existing = $null
if ($present -and -not $Rotate) {
    Write-Output ("{0}: already provisioned in SOPS (use -Rotate to replace it)." -f $Name)
    return
}

# The shared writer lock prevents conflicting updates; the per-key policy
# independently prevents unapproved issuance or rotation through this entry.
$policyPath = Join-Path $env:USERPROFILE '.config\sops\host-sops-policy.json'
if (-not (Test-Path -LiteralPath $policyPath -PathType Leaf)) {
    throw 'Missing host SOPS mutation policy'
}
$policy = Get-Content -LiteralPath $policyPath -Raw | ConvertFrom-Json -ErrorAction Stop
if ($policy.version -ne 1 -or $null -eq $policy.keys -or
    $null -eq $policy.keys.PSObject.Properties[$Name]) {
    throw 'SOPS key not approved for mutation'
}
$rule = $policy.keys.PSObject.Properties[$Name].Value
if ($rule.provider -cne 'RandomHex' -or
    $rule.allowRotate -isnot [bool] -or
    ($Rotate -and -not $rule.allowRotate)) {
    throw 'SOPS mutation policy denies operation'
}
$rule = $null
$policy = $null

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
} finally {
    if ($lockHandle) {
        $lockHandle.Dispose()
        Remove-Item -LiteralPath $lockPath -Force -ErrorAction SilentlyContinue
    }
}
