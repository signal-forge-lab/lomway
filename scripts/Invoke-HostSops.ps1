[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [ValidateSet('Status', 'Generate')]
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
$stage = $null
$backup = $null
$token = $null
$oldToken = $null
$raw = $null
$newPatId = $null
$oldPatId = $null
$committed = $false

try {
    if (-not (Test-Path -LiteralPath $sopsFile -PathType Leaf)) {
        throw 'Missing canonical SOPS file'
    }

    # Key names in SOPS JSON are not encrypted. Distinguish an absent key from
    # an extraction/decryption failure without decrypting unrelated values.
    $encryptedDoc = Get-Content -LiteralPath $sopsFile -Raw | ConvertFrom-Json
    $exists = ($null -ne $encryptedDoc.PSObject.Properties[$Key])
    $encryptedDoc = $null
    if ($exists) {
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
            [pscustomobject]@{
                key = $Key; provider = $Provider; present = $exists
                active_pat_matches = $matches.Count
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
    $backup = $stage + '.bak'
    $originalHash = (Get-FileHash -LiteralPath $sopsFile -Algorithm SHA256).Hash
    Copy-Item -LiteralPath $sopsFile -Destination $stage -ErrorAction Stop
    $originalAcl = (Get-Acl -LiteralPath $sopsFile).Sddl
    if ((Get-Acl -LiteralPath $stage).Sddl -ne $originalAcl) {
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
        & node.exe $patCli revoke --id $oldPatId *> $null
        if ($LASTEXITCODE -ne 0) { throw 'Old PAT revocation failed; replacement remains active' }
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
    [Console]::Error.WriteLine("HOST_SOPS_FAILED type=$failureType line=$failureLine; no secret data returned")
    exit 1
} finally {
    $token = $null; $oldToken = $null; $raw = $null
    if ($stage) { Remove-Item -LiteralPath $stage -Force -ErrorAction SilentlyContinue }
    if ($backup) { Remove-Item -LiteralPath $backup -Force -ErrorAction SilentlyContinue }
}
