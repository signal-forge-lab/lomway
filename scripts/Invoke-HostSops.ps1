[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [ValidateSet('Status', 'Generate', 'VerifyOpenBao', 'AuditOpenBao', 'CleanupOpenBao')]
    [string]$Action,

    [Parameter(Mandatory)]
    [ValidatePattern('^[A-Z][A-Z0-9_]{1,127}$')]
    [string]$Key,

    [ValidateSet('RandomHex', 'LomwayPAT')]
    [string]$Provider = 'RandomHex',

    [string]$Label = '',
    [string]$AllowedTools = '',
    [ValidateRange(1, 90)]
    [int]$Days = 60,
    [switch]$Rotate
)

# Host-local SOPS manager. The MCP caller supplies only non-secret parameters.
# All secret generation, SOPS extraction, write and verification stay in this
# PowerShell process. No decrypted content, digest or token is emitted.
# Note: this protects this interface's outputs; an unrestricted shell running
# as the same Windows user can still decrypt SOPS independently.
$ErrorActionPreference = 'Stop'
$sopsFile = Join-Path $env:USERPROFILE '.config\sops\secrets\global.sops.json'
$sopsExe = (Get-Command sops -ErrorAction Stop).Source
$selector = '["' + $Key + '"]'
$patCli = Join-Path $PSScriptRoot '..\sidecar\dist\src\pat-cli.js'
$patStoreFile = Join-Path $env:LOCALAPPDATA 'Lomway\oauth-sidecar\runtime\pat-tokens.json'
$policyFile = Join-Path $env:USERPROFILE '.config\sops\host-sops-policy.json'
$lockPath = Join-Path (Split-Path -Parent $sopsFile) '.sops-write.lock'
$lockHandle = $null
$stage = $null
$backup = $null
$token = $null
$oldToken = $null
$raw = $null
$newPatId = $null
$oldPatId = $null
$committed = $false
$revocationPending = $false

try {
    if (-not (Test-Path -LiteralPath $sopsFile -PathType Leaf)) {
        throw 'Missing canonical SOPS file'
    }

    if ($Action -eq 'VerifyOpenBao') {
        # Fixed host-side consumer: no arbitrary command, destination or key.
        # The child performs SOPS extraction and OpenBao requests locally and
        # reports verdicts only. Keep the original Status/Generate paths intact.
        if ($Key -cne 'OPENBAO_WINDOWS_AI_RUNTIME_ROLE_ID' -or
            $Provider -cne 'RandomHex' -or $Rotate.IsPresent -or
            $Label -or $AllowedTools) {
            throw 'Invalid OpenBao verification parameters'
        }
        & (Join-Path $PSScriptRoot 'Test-HostOpenBaoAppRole.ps1')
        if ($LASTEXITCODE -ne 0) { throw 'OpenBao AppRole verification failed' }
        return
    }

    if ($Action -in @('AuditOpenBao', 'CleanupOpenBao')) {
        if ($Key -cne 'OPENBAO_WINDOWS_AI_RUNTIME_ROLE_ID' -or
            $Provider -cne 'RandomHex' -or $Rotate.IsPresent -or
            $Label -or $AllowedTools) {
            throw 'Invalid OpenBao audit parameters'
        }
        & (Join-Path $PSScriptRoot 'Test-HostOpenBaoSecretIdInventory.ps1') `
            -CleanupUnused:($Action -eq 'CleanupOpenBao')
        if ($LASTEXITCODE -ne 0) { throw 'OpenBao SecretID audit failed' }
        return
    }

    if ($Action -eq 'Generate') {
        # Shared fail-closed lock with the Sopshelf GUI. CreateNew is an
        # atomic filesystem operation, not a check-then-create race.
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

        # This policy is an operational allowlist, not an OS authorization
        # boundary against untrusted code executing as the same user.
        if (-not (Test-Path -LiteralPath $policyFile -PathType Leaf)) {
            throw 'Missing host SOPS mutation policy'
        }
        $policy = Get-Content -LiteralPath $policyFile -Raw | ConvertFrom-Json -ErrorAction Stop
        if ($policy.version -ne 1 -or $null -eq $policy.keys -or
            $null -eq $policy.keys.PSObject.Properties[$Key]) {
            throw 'SOPS key not approved for mutation'
        }
        $rule = $policy.keys.PSObject.Properties[$Key].Value
        if ($rule.provider -cne $Provider -or
            $rule.allowRotate -isnot [bool] -or
            ($Rotate -and -not $rule.allowRotate)) {
            throw 'SOPS mutation policy denies operation'
        }
        if ($Provider -eq 'LomwayPAT') {
            $approved = @($rule.allowedTools)
            $requested = @($AllowedTools -split ',' | Where-Object { $_ })
            if ($rule.label -cne $Label -or
                $rule.maxDays -isnot [long] -or $rule.maxDays -lt 1 -or
                $rule.maxDays -gt 90 -or $Days -gt $rule.maxDays -or
                $approved.Count -eq 0 -or
                $approved.Count -ne $requested.Count -or
                @($requested | Where-Object { $approved -cnotcontains $_ }).Count -ne 0) {
                throw 'PAT issuance denied by key policy'
            }
        }
        $rule = $null
        $policy = $null
    }

    # Key names in SOPS JSON are not encrypted. Distinguish an absent key from
    # an extraction/decryption failure without decrypting unrelated values.
    $encryptedDoc = Get-Content -LiteralPath $sopsFile -Raw | ConvertFrom-Json
    $exists = ($null -ne $encryptedDoc.PSObject.Properties[$Key])
    $encryptedDoc = $null
    if ($exists -and ($Action -ne 'Status' -or $Provider -eq 'LomwayPAT')) {
        $valueText = (& $sopsExe --decrypt --extract $selector $sopsFile 2>$null) -join ''
        if ($LASTEXITCODE -ne 0) { throw 'SOPS key decryption failed' }
        try { $oldToken = [string]($valueText | ConvertFrom-Json -ErrorAction Stop) }
        catch { $oldToken = [string]$valueText.Trim('"') }
    }
    $valueText = $null

    if ($Action -eq 'Status') {
        if ($Provider -eq 'LomwayPAT' -and $exists) {
            $digest = [Convert]::ToHexString(
                [Security.Cryptography.SHA256]::HashData(
                    [Text.Encoding]::UTF8.GetBytes($oldToken)
                )
            ).ToLowerInvariant()
            $records = (Get-Content -LiteralPath $patStoreFile -Raw | ConvertFrom-Json).tokens
            $matches = @($records | Where-Object {
                $_.digest -ceq $digest -and $null -eq $_.revokedAt -and
                $_.expiresAt -gt [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
            })
            $allowed = if ($matches.Count -eq 1) { @($matches[0].allowedTools) } else { @() }
            $sameLabelActiveCount = 0
            if ($matches.Count -eq 1) {
                $matchedLabel = $matches[0].label
                $sameLabelActiveCount = @($records | Where-Object {
                    $_.label -ceq $matchedLabel -and $null -eq $_.revokedAt -and
                    $_.expiresAt -gt [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
                }).Count
            }
            [pscustomobject]@{
                key = $Key; provider = $Provider; present = $exists
                active_pat_matches = $matches.Count
                same_label_active_count = $sameLabelActiveCount
                rotation_overlap_warning = $sameLabelActiveCount -gt 1
                allowed_count = $allowed.Count
                praxiom_count = @($allowed | Where-Object { $_ -like 'praxiom_praxiom_*' }).Count
                coding_tools_count = @($allowed | Where-Object { $_ -like 'coding_tools_*' }).Count
                execute_allowed = $allowed -ccontains 'praxiom_praxiom_execute'
            } | ConvertTo-Json -Compress
        } else {
            [pscustomobject]@{key=$Key;provider=$Provider;present=$exists} | ConvertTo-Json -Compress
        }
        return
    }

    if ($exists -and -not $Rotate) { throw 'Key exists: explicit -Rotate required' }
    if ($Rotate -and -not $exists) { throw 'Rotation requires existing key' }

    switch ($Provider) {
        'RandomHex' {
            if ($AllowedTools -or $Label) { throw 'RandomHex does not accept PAT parameters' }
            $token = [Convert]::ToHexString(
                [Security.Cryptography.RandomNumberGenerator]::GetBytes(32)
            ).ToLowerInvariant()
        }
        'LomwayPAT' {
            if (-not (Test-Path -LiteralPath $patCli -PathType Leaf)) { throw 'Lomway PAT CLI missing' }
            if ($Label -notmatch '^[A-Za-z0-9_.:-]{1,80}$') { throw 'Valid PAT label required' }
            $tools = @($AllowedTools -split ',' | Where-Object { $_ })
            if ($tools.Count -lt 1 -or $tools.Count -gt 100 -or
                @($tools | Select-Object -Unique).Count -ne $tools.Count -or
                @($tools | Where-Object { $_ -cnotmatch '^[A-Za-z0-9_.:-]{1,128}$' -or
                    $_ -eq '*' -or $_ -eq 'proxy/config' -or $_ -eq 'proxy/add_backend' }).Count) {
                throw 'Invalid explicit PAT allowlist'
            }
            if ($exists) {
                $digest = [Convert]::ToHexString(
                    [Security.Cryptography.SHA256]::HashData(
                        [Text.Encoding]::UTF8.GetBytes($oldToken)
                    )
                ).ToLowerInvariant()
                $records = (Get-Content -LiteralPath $patStoreFile -Raw | ConvertFrom-Json).tokens
                $oldMatches = @($records | Where-Object {
                    $_.digest -ceq $digest -and $_.label -ceq $Label -and $null -eq $_.revokedAt
                })
                if ($oldMatches.Count -ne 1) { throw 'Old PAT identity mismatch' }
                $oldPatId = [string]$oldMatches[0].id
            }
            # CLI plaintext JSON is captured only in local process memory.
            $raw = & node.exe $patCli issue --label $Label --days $Days --tools ($tools -join ',')
            if ($LASTEXITCODE -ne 0) { throw 'PAT issue failed' }
            $issued = ($raw | ConvertFrom-Json -ErrorAction Stop)
            $token = [string]$issued.token
            $newPatId = [string]$issued.id
            if (-not $token -or -not $newPatId) { throw 'Invalid PAT producer response' }
            $raw = $null
            $issued = $null
        }
    }

    # Stage only encrypted files next to the canonical SOPS file. No plaintext
    # temporary file, CLI argument, clipboard write or stdout of secrets.
    $parent = Split-Path -Parent $sopsFile
    $stage = Join-Path $parent ('.host-sops-' + [guid]::NewGuid().ToString('N') + '.json')
    # File.Replace creates an encrypted, ACL-preserving backup beside the
    # canonical store. Unlike the transient stage it survives successful
    # mutations for recovery from accidental rotation or partial PAT failure.
    $backup = $sopsFile + '.backup-' +
        [DateTime]::UtcNow.ToString('yyyyMMddTHHmmssfffZ') + '-' +
        [guid]::NewGuid().ToString('N') + '.sops.json'
    $originalHash = (Get-FileHash -LiteralPath $sopsFile -Algorithm SHA256).Hash
    Copy-Item -LiteralPath $sopsFile -Destination $stage -ErrorAction Stop
    # A previous File.Replace may have preserved explicit owner-only ACLs
    # that differ from the directory defaults of a new temporary file.
    # Clone the canonical ACL rather than weakening it or relying on
    # Copy-Item inheritance. Fail closed when ACL replication is denied.
    $canonicalAcl = Get-Acl -LiteralPath $sopsFile
    Set-Acl -LiteralPath $stage -AclObject $canonicalAcl -ErrorAction Stop
    # Set-Acl may add the SDDL DACL auto-inherited control flag (AI).
    # That flag doesn't grant permissions; keep every ACE, owner/group and
    # DACL-protection flag identical, ignoring only this metadata bit.
    $originalAcl = $canonicalAcl.Sddl -replace 'D:(P?)AI(?=\()', 'D:$1'
    $stageAcl = (Get-Acl -LiteralPath $stage).Sddl -replace 'D:(P?)AI(?=\()', 'D:$1'
    if ($stageAcl -cne $originalAcl) {
        throw 'Staging ACL differs from the canonical SOPS file'
    }
    $encoded = ConvertTo-Json -InputObject ([string]$token) -Compress
    $encoded | & $sopsExe set --value-stdin $stage $selector *> $null
    if ($LASTEXITCODE -ne 0) { throw 'SOPS staging update failed' }
    $encoded = $null
    $verifyText = (& $sopsExe --decrypt --extract $selector $stage 2>$null) -join ''
    $verifiedValue = $null
    try { $verifiedValue = [string]($verifyText | ConvertFrom-Json -ErrorAction Stop) }
    catch { $verifiedValue = [string]$verifyText.Trim('"') }
    $valid = ($LASTEXITCODE -eq 0 -and $verifiedValue -ceq $token)
    $verifiedValue = $null
    $verifyText = $null
    if (-not $valid) { throw 'SOPS staged readback failed' }
    if ((Get-FileHash -LiteralPath $sopsFile -Algorithm SHA256).Hash -ne $originalHash) {
        throw 'Concurrent SOPS update detected'
    }
    [IO.File]::Replace($stage, $sopsFile, $backup, $true)
    $committed = $true

    if ($oldPatId) {
        $revoked = $false
        for ($attempt = 0; $attempt -lt 3; $attempt++) {
            & node.exe $patCli revoke --id $oldPatId *> $null
            if ($LASTEXITCODE -eq 0) {
                $revoked = $true
                break
            }
            Start-Sleep -Milliseconds 150
        }
        if (-not $revoked) {
            $revocationPending = $true
            throw 'Old PAT revocation failed; manual reconciliation required'
        }
    }
    [pscustomobject]@{
        key=$Key; provider=$Provider; updated=$true
        rotated=$Rotate.IsPresent; allowed_count=$(if ($Provider -eq 'LomwayPAT') {$tools.Count} else {0})
    } | ConvertTo-Json -Compress
} catch {
    $failureType = $_.Exception.GetType().Name
    $failureLine = $_.InvocationInfo.ScriptLineNumber
    if (-not $committed -and $newPatId) {
        & node.exe $patCli revoke --id $newPatId *> $null
    }
    # Do not expose exception/CLI contents to external tools.
    $failureCode = if ($revocationPending) { 'PAT_REVOCATION_PENDING' } else { 'FAILED' }
    [Console]::Error.WriteLine("HOST_SOPS_$failureCode type=$failureType line=$failureLine; no secret data returned")
    exit 1
} finally {
    $token = $null; $oldToken = $null; $raw = $null
    if ($stage) { Remove-Item -LiteralPath $stage -Force -ErrorAction SilentlyContinue }
    if (-not $committed -and $backup) {
        Remove-Item -LiteralPath $backup -Force -ErrorAction SilentlyContinue
    }
    if ($lockHandle) {
        $lockHandle.Dispose()
        Remove-Item -LiteralPath $lockPath -Force -ErrorAction SilentlyContinue
    }
}
