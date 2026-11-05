# Value-free audit of the exact Windows AI AppRole's issued SecretIDs.
# Invoked by the host SOPS manager; does not mutate OpenBao or SOPS.
[CmdletBinding()]
param([switch]$CleanupUnused)

$ErrorActionPreference = 'Stop'
$phase = 'preflight'
try {
    $store = Join-Path $env:USERPROFILE '.config\sops\secrets\global.sops.json'
    function Read-Secret([string]$name) {
        $selector = '["' + $name + '"]'
        $valueText = (& sops --decrypt --extract $selector $store 2>$null) -join ''
        if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($valueText)) {
            throw 'SOPS extraction failed'
        }
        try { return [string]($valueText | ConvertFrom-Json -ErrorAction Stop) }
        catch { return [string]$valueText.Trim('"') }
    }
    $phase = 'load-local-credentials'
    $root = Read-Secret 'OPENBAO_INITIAL_ROOT_TOKEN'
    $current = Read-Secret 'OPENBAO_WINDOWS_AI_RUNTIME_SECRET_ID'
    $headers = @{ 'X-Vault-Token' = $root }
    $base = 'http://127.0.0.1:8200/v1/auth/approle/role/windows-ai-runtime'

    $phase = 'lookup-current'
    $lookup = Invoke-RestMethod -Uri "$base/secret-id/lookup" -Method Post `
        -Headers $headers -ContentType 'application/json' `
        -Body (@{secret_id=$current}|ConvertTo-Json -Compress)
    $knownAccessor = [string]$lookup.data.secret_id_accessor
    if (-not $knownAccessor) { throw 'Current SecretID accessor missing' }
    $currentCreated = [datetime]$lookup.data.creation_time

    $phase = 'list-active'
    $list = Invoke-RestMethod -Uri "$base/secret-id?list=true" -Method Get -Headers $headers
    $accessors = @($list.data.keys)
    if ($accessors -cnotcontains $knownAccessor) { throw 'Current SecretID not in list' }

    $phase = 'check-unmatched'
    $otherAccessors = @($accessors | Where-Object { $_ -cne $knownAccessor })
    $allOlder = $true
    $allRecent = $true
    foreach ($accessor in $otherAccessors) {
        $record = Invoke-RestMethod -Uri "$base/secret-id-accessor/lookup" -Method Post `
            -Headers $headers -ContentType 'application/json' `
            -Body (@{secret_id_accessor=$accessor}|ConvertTo-Json -Compress)
        $otherCreated = [datetime]$record.data.creation_time
        if ($otherCreated -ge $currentCreated) {
            $allOlder = $false
        }
        $ageGap = ($currentCreated - $otherCreated).TotalMinutes
        if ($ageGap -lt 0 -or $ageGap -gt 60) { $allRecent = $false }
    }
    Write-Output 'ACTIVE_SECRET_ID_INVENTORY=PASS'
    Write-Output ('ACTIVE_SECRET_ID_COUNT=' + $accessors.Count)
    Write-Output ('CURRENT_ID_FOUND=' + ($accessors -ccontains $knownAccessor))
    Write-Output ('OTHER_ID_COUNT=' + $otherAccessors.Count)
    Write-Output ('OTHER_IDS_OLDER_THAN_CURRENT=' + $allOlder)
    Write-Output ('OTHER_IDS_WITHIN_60_MINUTES=' + $allRecent)
    if ($CleanupUnused) {
        # Only clean up the one extra ID from the immediately preceding failed
        # SOPS-save attempt. Anything else requires human reconciliation.
        if ($accessors.Count -ne 2 -or $otherAccessors.Count -ne 1 -or
            -not $allOlder -or -not $allRecent) {
            throw 'Cleanup preconditions not met'
        }
        $phase = 'revoke-unused'
        Invoke-RestMethod -Uri "$base/secret-id-accessor/destroy" -Method Post `
            -Headers $headers -ContentType 'application/json' `
            -Body (@{secret_id_accessor=$otherAccessors[0]}|ConvertTo-Json -Compress) | Out-Null
        $phase = 'verify-cleanup'
        $after = Invoke-RestMethod -Uri "$base/secret-id?list=true" -Method Get -Headers $headers
        $remaining = @($after.data.keys)
        if ($remaining.Count -ne 1 -or $remaining[0] -cne $knownAccessor) {
            throw 'Unexpected SecretID count after cleanup'
        }
        Write-Output 'OLD_SECRET_ID_REVOKED=PASS'
        Write-Output 'CURRENT_SECRET_ID_PRESERVED=PASS'
    }
    Write-Output 'SECRET_ID_AUDIT=PASS'
    exit 0
} catch {
    # Do not return credentials, accessors or response/exception bodies.
    Write-Output ('FAILED_PHASE=' + $phase)
    Write-Output 'SECRET_ID_AUDIT=FAIL'
    exit 1
} finally {
    $root = $null
    $current = $null
    $lookup = $null
    $list = $null
    $headers = $null
}
