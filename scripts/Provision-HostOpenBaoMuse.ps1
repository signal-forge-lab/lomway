# Fixed-scope Muse AppRole provisioning and capability verification.
# SOPS recovery token and temporary AppRole credentials stay in host memory.
[CmdletBinding()]
param()
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$base = 'http://127.0.0.1:8200/v1'
$store = Join-Path $env:USERPROFILE '.config\sops\secrets\global.sops.json'
$policyName = 'read-accounts-muse'
$roleName = 'muse'
$policyText = @'
path "kv/data/accounts/mozilla/personal" {
  capabilities = ["read"]
}

path "kv/data/accounts/google/personal" {
  capabilities = ["read"]
}
'@
$phase = 'preflight'
$rootToken = $null
$tempToken = $null
$tempSecretIdAccessor = $null
$testHeaders = $null
$adminHeaders = $null
$failed = $false
$cleanupFailed = $false
$outcomes = [System.Collections.Generic.List[string]]::new()
try {
    $selector = '["OPENBAO_INITIAL_ROOT_TOKEN"]'
    $extracted = (& sops --decrypt --extract $selector $store 2>$null) -join ''
    if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($extracted)) {
        throw 'Recovery token unavailable'
    }
    try { $rootToken = [string]($extracted | ConvertFrom-Json -ErrorAction Stop) }
    catch { $rootToken = [string]$extracted.Trim('"') }
    $extracted = $null
    if ([string]::IsNullOrWhiteSpace($rootToken)) { throw 'Invalid recovery token' }
    $adminHeaders = @{ 'X-Vault-Token' = $rootToken }

    # Never broaden or overwrite existing settings with different permissions.
    $phase = 'policy'
    $policyUri = "$base/sys/policies/acl/$policyName"
    $currentPolicy = Invoke-WebRequest -Uri $policyUri -Headers $adminHeaders -SkipHttpErrorCheck
    if ([int]$currentPolicy.StatusCode -eq 404) {
        Invoke-RestMethod -Uri $policyUri -Method Post -Headers $adminHeaders -ContentType 'application/json' -Body (@{policy = $policyText} | ConvertTo-Json -Compress) | Out-Null
        $outcomes.Add('MUSE_POLICY_CREATED=PASS')
    } elseif ([int]$currentPolicy.StatusCode -eq 200) {
        $serverText = [string](($currentPolicy.Content | ConvertFrom-Json).data.policy)
        if (($serverText -replace '\s+', '') -cne ($policyText -replace '\s+', '')) {
            throw 'Existing Muse policy differs from requested scope'
        }
        $outcomes.Add('MUSE_POLICY_REUSED=PASS')
    } else { throw 'Cannot inspect Muse policy' }

    $phase = 'role'
    $roleUri = "$base/auth/approle/role/$roleName"
    $roleReply = Invoke-WebRequest -Uri $roleUri -Headers $adminHeaders -SkipHttpErrorCheck
    if ([int]$roleReply.StatusCode -eq 404) {
        $config = @{token_policies = @($policyName); bind_secret_id = $true} | ConvertTo-Json -Compress
        Invoke-RestMethod -Uri $roleUri -Method Post -Headers $adminHeaders -ContentType 'application/json' -Body $config | Out-Null
        $outcomes.Add('MUSE_ROLE_CREATED=PASS')
    } elseif ([int]$roleReply.StatusCode -eq 200) {
        $outcomes.Add('MUSE_ROLE_REUSED=PASS')
    } else { throw 'Cannot inspect Muse role' }
    $role = Invoke-RestMethod -Uri $roleUri -Headers $adminHeaders
    if ($role.data.bind_secret_id -ne $true -or @($role.data.token_policies).Count -ne 1 -or @($role.data.token_policies) -cnotcontains $policyName) {
        throw 'Muse role policy or SecretID requirement mismatched'
    }
    $outcomes.Add('MUSE_ROLE_CONFIGURATION=PASS')

    $phase = 'test-credentials'
    $roleIdReply = Invoke-RestMethod -Uri "$roleUri/role-id" -Headers $adminHeaders
    $roleId = [string]$roleIdReply.data.role_id
    $issued = Invoke-RestMethod -Uri "$roleUri/secret-id" -Method Post -Headers $adminHeaders -ContentType 'application/json' -Body '{}'
    $tempSecretId = [string]$issued.data.secret_id
    $tempSecretIdAccessor = [string]$issued.data.secret_id_accessor
    if (-not $roleId -or -not $tempSecretId -or -not $tempSecretIdAccessor) { throw 'Temporary AppRole credentials missing' }

    $phase = 'login'
    $login = Invoke-RestMethod -Uri "$base/auth/approle/login" -Method Post -ContentType 'application/json' -Body (@{role_id = $roleId; secret_id = $tempSecretId} | ConvertTo-Json -Compress)
    $tempSecretId = $null
    $roleId = $null
    $tempToken = [string]$login.auth.client_token
    if (-not $tempToken -or @($login.auth.policies) -cnotcontains $policyName) { throw 'Muse AppRole login or assigned policy invalid' }
    $testHeaders = @{ 'X-Vault-Token' = $tempToken }
    $outcomes.Add('MUSE_APPROLE_LOGIN=PASS')

    $phase = 'capabilities'
    $permitted = @('kv/data/accounts/mozilla/personal', 'kv/data/accounts/google/personal')
    $blocked = @('kv/data/accounts/github/personal', 'kv/data/accounts/mozilla/other', 'kv/data/api/ai/openai')
    $capsReply = Invoke-RestMethod -Uri "$base/sys/capabilities-self" -Method Post -Headers $testHeaders -ContentType 'application/json' -Body (@{paths = @($permitted) + @($blocked)} | ConvertTo-Json -Compress)
    $capsData = if ($capsReply.data) { $capsReply.data } else { $capsReply }
    foreach ($path in $permitted) {
        $caps = @($capsData.$path)
        if ($caps.Count -ne 1 -or $caps[0] -cne 'read') { throw 'Unexpected allowed path capabilities' }
    }
    foreach ($path in $blocked) {
        $caps = @($capsData.$path)
        if ($caps.Count -ne 1 -or $caps[0] -cne 'deny') { throw 'Unexpected access to another secret path' }
    }
    $outcomes.Add('MUSE_TWO_PATHS_READ_ONLY=PASS')
    $outcomes.Add('MUSE_OTHER_PATHS_DENIED=PASS')
}
catch {
    $failed = $true
    Write-Output ('MUSE_FAILED_PHASE=' + $phase)
}
finally {
    if ($tempToken) {
        try {
            Invoke-RestMethod -Uri "$base/auth/token/revoke-self" -Method Post -Headers $testHeaders | Out-Null
            $outcomes.Add('MUSE_TEST_TOKEN_REVOKED=PASS')
        } catch { $cleanupFailed = $true }
    }
    if ($tempSecretIdAccessor) {
        try {
            Invoke-RestMethod -Uri "$roleUri/secret-id-accessor/destroy" -Method Post -Headers $adminHeaders -ContentType 'application/json' -Body (@{secret_id_accessor = $tempSecretIdAccessor} | ConvertTo-Json -Compress) | Out-Null
            $outcomes.Add('MUSE_TEST_SECRET_ID_REVOKED=PASS')
        } catch { $cleanupFailed = $true }
    }
    $rootToken = $null
    $tempToken = $null
    $tempSecretId = $null
    $tempSecretIdAccessor = $null
    $testHeaders = $null
    $adminHeaders = $null
}
foreach ($result in $outcomes) { Write-Output $result }
if ($cleanupFailed) { Write-Output 'MUSE_TEST_CLEANUP=FAIL' }
if ($failed -or $cleanupFailed) { Write-Output 'MUSE_PROVISION=FAIL'; exit 1 }
Write-Output 'MUSE_PROVISION=PASS'
exit 0
