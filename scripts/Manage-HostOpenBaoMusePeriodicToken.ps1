# Host-local OpenBao Muse periodic token issuance. Only status flags leave the
# host. Token and accessor are atomically committed as encrypted SOPS values.
[CmdletBinding()]
param([Parameter(Mandatory)][ValidateSet('Check', 'Test', 'Issue')][string]$Mode)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$store = Join-Path $env:USERPROFILE '.config\sops\secrets\global.sops.json'
$base = 'http://127.0.0.1:8200/v1'
$tokenKey = 'OPENBAO_MUSE_PERIODIC_TOKEN'
$accessorKey = 'OPENBAO_MUSE_PERIODIC_TOKEN_ACCESSOR'
$policyName = 'read-accounts-muse'
$expectedPolicy = @'
path "kv/data/accounts/mozilla/personal" {
  capabilities = ["read"]
}

path "kv/data/accounts/google/personal" {
  capabilities = ["read"]
}
'@
$phase = 'preflight'
$lock = $null
$stage = $null
$backup = $null
$committed = $false
$token = $null
$accessor = $null
$root = $null
$rootHeaders = $null
$testHeaders = $null
$cleanupFailed = $false
$failed = $false

function Read-Scalar([string]$name, [string]$path) {
    $selector = '["' + $name + '"]'
    $text = (& sops --decrypt --extract $selector $path 2>$null) -join ''
    if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($text)) {
        throw 'SOPS extraction failed'
    }
    try { return [string]($text | ConvertFrom-Json -ErrorAction Stop) }
    catch { return [string]$text.Trim('"') }
}

function Assert-Capabilities([hashtable]$headers) {
    $allowed = @('kv/data/accounts/mozilla/personal', 'kv/data/accounts/google/personal')
    $denied = @('kv/data/accounts/github/personal', 'kv/data/accounts/mozilla/other', 'kv/data/api/ai/openai')
    $paths = @($allowed) + @($denied)
    $body = @{ paths = $paths } | ConvertTo-Json -Compress
    $response = Invoke-RestMethod -Uri "$base/sys/capabilities-self" -Headers $headers -Method Post -ContentType 'application/json' -Body $body
    $data = if ($response.data) { $response.data } else { $response }
    foreach ($path in $allowed) {
        $caps = @($data.$path)
        if ($caps.Count -ne 1 -or $caps[0] -cne 'read') { throw 'Expected read-only capability missing' }
    }
    foreach ($path in $denied) {
        $caps = @($data.$path)
        if ($caps.Count -ne 1 -or $caps[0] -cne 'deny') { throw 'Unexpected access to unrelated path' }
    }
}

try {
    if (-not (Test-Path -LiteralPath $store -PathType Leaf)) {
        throw 'SOPS store missing'
    }
    $dir = Split-Path -Parent $store
    $lockPath = Join-Path $dir '.sops-write.lock'
    if ($Mode -eq 'Issue') {
        # Shared with Sopshelf and Invoke-HostSops Generate.
        for ($i = 0; $i -lt 50; $i++) {
            try {
                $lock = [IO.FileStream]::new($lockPath, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
                break
            } catch [IO.IOException] {
                if (-not (Test-Path -LiteralPath $lockPath -PathType Leaf)) { throw }
                Start-Sleep -Milliseconds 100
            }
        }
        if (-not $lock) { throw 'Shared SOPS lock unavailable' }
    }

    $phase = 'existing-key-check'
    # Read encrypted key names only. Never overwrite or rotate silently.
    $doc = Get-Content -LiteralPath $store -Raw | ConvertFrom-Json -ErrorAction Stop
    $hasToken = $null -ne $doc.PSObject.Properties[$tokenKey]
    $hasAccessor = $null -ne $doc.PSObject.Properties[$accessorKey]
    $doc = $null
    if ($hasToken -ne $hasAccessor) { throw 'Partial existing Muse token pair' }
    if ($hasToken) {
        Write-Output 'MUSE_TOKEN_PAIR=ALREADY_STORED'
        Write-Output 'MUSE_TOKEN_ISSUED=NO'
        exit 0
    }

    $phase = 'policy-check'
    $root = Read-Scalar 'OPENBAO_INITIAL_ROOT_TOKEN' $store
    $rootHeaders = @{ 'X-Vault-Token' = $root }
    $policy = Invoke-RestMethod -Uri "$base/sys/policies/acl/$policyName" -Headers $rootHeaders -Method Get
    $actualPolicy = [string]$policy.data.policy
    if (($actualPolicy -replace '\s+', '') -cne ($expectedPolicy -replace '\s+', '')) {
        throw 'Muse policy differs from two read-only paths'
    }
    Write-Output 'MUSE_POLICY_SCOPE=PASS'
    if ($Mode -eq 'Check') {
        Write-Output 'MUSE_TOKEN_ISSUE_READY=YES'
        Write-Output 'MUSE_TOKEN_ISSUED=NO'
        exit 0
    }

    if ($Mode -eq 'Issue') {
        $phase = 'encrypted-stage'
        $hash = (Get-FileHash -LiteralPath $store -Algorithm SHA256).Hash
        $stage = Join-Path $dir ('.host-sops-muse-' + [guid]::NewGuid().ToString('N') + '.json')
        $backup = $store + '.backup-' + [DateTime]::UtcNow.ToString('yyyyMMddTHHmmssfffZ') + '-' + [guid]::NewGuid().ToString('N') + '.sops.json'
        Copy-Item -LiteralPath $store -Destination $stage -ErrorAction Stop
        $canonicalAcl = Get-Acl -LiteralPath $store
        Set-Acl -LiteralPath $stage -AclObject $canonicalAcl -ErrorAction Stop
        $canonicalSddl = $canonicalAcl.Sddl -replace 'D:(P?)AI(?=\()', 'D:$1'
        $stageSddl = (Get-Acl -LiteralPath $stage).Sddl -replace 'D:(P?)AI(?=\()', 'D:$1'
        if ($canonicalSddl -cne $stageSddl) { throw 'Encrypted stage ACL mismatch' }
    }

    $phase = 'issue'
    # create-orphan provides a parent-independent service token. No explicit
    # maximum TTL: the 24h period resets on every successful renew-self.
    $periodSeconds = if ($Mode -eq 'Test') { 300 } else { 86400 }
    $issueBody = @{
        policies = @($policyName)
        period = $(if ($Mode -eq 'Test') { '5m' } else { '24h' })
        type = 'service'
        renewable = $true
        display_name = 'muse'
    } | ConvertTo-Json -Compress
    $issued = Invoke-RestMethod -Uri "$base/auth/token/create-orphan" -Method Post -Headers $rootHeaders -ContentType 'application/json' -Body $issueBody
    $token = [string]$issued.auth.client_token
    $accessor = [string]$issued.auth.accessor
    if (-not $token -or -not $accessor) { throw 'Token response missing fields' }
    $testHeaders = @{ 'X-Vault-Token' = $token }

    $phase = 'verify-token'
    $lookup = Invoke-RestMethod -Uri "$base/auth/token/lookup-self" -Method Get -Headers $testHeaders
    $info = $lookup.data
    $unexpectedPolicies = @($info.policies | Where-Object { $_ -cnotin @('default', $policyName) })
    if ($info.renewable -ne $true -or [int]$info.period -ne $periodSeconds -or
        [int]$info.explicit_max_ttl -ne 0 -or [string]$info.type -cne 'service' -or
        [string]$info.accessor -cne $accessor -or
        @($info.policies) -cnotcontains $policyName -or
        $unexpectedPolicies.Count -ne 0 -or
        ($null -ne $info.PSObject.Properties['orphan'] -and $info.orphan -ne $true)) {
        throw 'Issued token properties invalid'
    }
    Assert-Capabilities $testHeaders
    Write-Output 'MUSE_TOKEN_PROPERTIES=PASS'
    Write-Output 'MUSE_TOKEN_SCOPE=PASS'

    $phase = 'renew-test'
    $renewed = Invoke-RestMethod -Uri "$base/auth/token/renew-self" -Method Post -Headers $testHeaders -ContentType 'application/json' -Body '{}'
    if ($renewed.auth.renewable -ne $true -or [int]$renewed.auth.lease_duration -lt ($periodSeconds - 60)) {
        throw 'Periodic token renewal did not restore the period'
    }
    Write-Output 'MUSE_RENEW_SELF=PASS'

    if ($Mode -eq 'Test') {
        Write-Output 'MUSE_TEMP_TOKEN_SMOKE_TEST=PASS'
    } else {
    $phase = 'encrypted-write'
    $entries = @(
        [pscustomobject]@{ name = $tokenKey; value = $token }
        [pscustomobject]@{ name = $accessorKey; value = $accessor }
    )
    foreach ($entry in $entries) {
        $selector = '["' + $entry.name + '"]'
        $encoded = ConvertTo-Json -InputObject ([string]$entry.value) -Compress
        $encoded | & sops set --value-stdin $stage $selector *> $null
        $encoded = $null
        if ($LASTEXITCODE -ne 0) { throw 'Encrypted stage write failed' }
    }
    if ((Read-Scalar $tokenKey $stage) -cne $token -or
        (Read-Scalar $accessorKey $stage) -cne $accessor) {
        throw 'Encrypted stage verification failed'
    }
    if ((Get-FileHash -LiteralPath $store -Algorithm SHA256).Hash -cne $hash) {
        throw 'Concurrent SOPS update detected'
    }
    [IO.File]::Replace($stage, $store, $backup, $true)
    $committed = $true
    Write-Output 'MUSE_SOPS_ATOMIC_PAIR_SAVE=PASS'
    Write-Output 'MUSE_TOKEN_ISSUED=YES'
    }
}
catch {
    $failed = $true
    # Exception and REST response bodies may contain credentials. Never echo.
    Write-Output ('MUSE_FAILED_PHASE=' + $phase)
}
finally {
    if ($token -and -not $committed) {
        try {
            if ($accessor -and $rootHeaders) {
                Invoke-RestMethod -Uri "$base/auth/token/revoke-accessor" -Method Post -Headers $rootHeaders -ContentType 'application/json' -Body (@{accessor = $accessor} | ConvertTo-Json -Compress) | Out-Null
            } else {
                Invoke-RestMethod -Uri "$base/auth/token/revoke-self" -Method Post -Headers @{ 'X-Vault-Token' = $token } | Out-Null
            }
            Write-Output 'MUSE_UNSAVED_TOKEN_REVOKED=PASS'
        } catch {
            $cleanupFailed = $true
            Write-Output 'MUSE_UNSAVED_TOKEN_REVOKED=FAIL'
        }
    }
    $token = $null
    $accessor = $null
    $entries = $null
    $root = $null
    $rootHeaders = $null
    $testHeaders = $null
    $issueBody = $null
    $issued = $null
    $lookup = $null
    $renewed = $null
    if ($stage) { Remove-Item -LiteralPath $stage -Force -ErrorAction SilentlyContinue }
    if (-not $committed -and $backup) {
        Remove-Item -LiteralPath $backup -Force -ErrorAction SilentlyContinue
    }
    if ($lock) {
        $lock.Dispose()
        Remove-Item -LiteralPath $lockPath -Force -ErrorAction SilentlyContinue
    }
}
if ($failed -or $cleanupFailed) { Write-Output 'MUSE_ISSUER=FAIL'; exit 1 }
Write-Output 'MUSE_ISSUER=PASS'
exit 0
