[CmdletBinding()]
param([switch]$VerifyOnly, [switch]$ReconcileBeforeCutover)

# One-time compatibility snapshot. New OAuth/DCR clients are NEVER added
# automatically. Existing live grant holders remain functional after the
# Gateway begins enforcing per-client tool permissions.
$ErrorActionPreference = 'Stop'
$runtime = Join-Path $env:LOCALAPPDATA 'Lomway\oauth-sidecar\runtime'
$statePath = Join-Path $runtime 'state.json'
$policyPath = Join-Path $runtime 'client-tool-policy.json'
if (!(Test-Path -LiteralPath $statePath -PathType Leaf)) {
    throw 'OAuth state missing; refusing to provision legacy client policy'
}
$state = Get-Content -LiteralPath $statePath -Raw | ConvertFrom-Json -Depth 100
$now = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
$approvedIds = @($state.records.Grant.PSObject.Properties |
    # CIMD clients can have live grants without persistent Client records.
    Where-Object { $_.Value.clientId -is [string] -and $_.Value.exp -gt $now } |
    ForEach-Object { [string]$_.Value.clientId } | Sort-Object -Unique)
$digests = @($approvedIds | ForEach-Object {
    $bytes = [Text.Encoding]::UTF8.GetBytes($_)
    'sha256:' + [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($bytes)).ToLowerInvariant()
} | Sort-Object -Unique)
if (Test-Path -LiteralPath $policyPath -PathType Leaf) {
    $existing = Get-Content -LiteralPath $policyPath -Raw | ConvertFrom-Json -Depth 20
    if ($existing.version -ne 1) { throw 'Existing policy has invalid version' }
    if ($ReconcileBeforeCutover) {
        $deployed = Join-Path (Split-Path $PSScriptRoot -Parent) 'runtime\oauth-hardening-deployment.json'
        if (Test-Path $deployed) {
            throw 'OAuth hardening cutover was already attempted. Refusing legacy privilege expansion.'
        }
        $combined = @(@($existing.unrestrictedLegacyClientHashes) + $digests | Sort-Object -Unique)
        $existing.unrestrictedLegacyClientHashes = $combined
        $temp = $policyPath + '.' + [Guid]::NewGuid().ToString('N') + '.tmp'
        try {
            [IO.File]::WriteAllText(
                $temp, ($existing | ConvertTo-Json -Depth 12), [Text.UTF8Encoding]::new($false)
            )
            [IO.File]::Move($temp, $policyPath, $true)
        } finally {
            Remove-Item -LiteralPath $temp -Force -ErrorAction SilentlyContinue
        }
        Write-Output ('RECONCILED_BEFORE_CUTOVER protected_clients='+$combined.Count)
        exit 0
    }
    Write-Output ('EXISTING_POLICY retained_legacy_clients=' + @($existing.unrestrictedLegacyClientHashes).Count)
    exit 0
}
if ($VerifyOnly) {
    Write-Output ('PREFLIGHT_OK existing_approved_clients=' + $digests.Count)
    exit 0
}
if (!$digests.Count) {
    throw 'No active approved clients; manual review required before cutover'
}
$policy = @{
    version = 1
    unrestrictedLegacyClientHashes = $digests
    clientToolAllowlists = @{}
} | ConvertTo-Json -Depth 10
$temporary = $policyPath + '.' + [Guid]::NewGuid().ToString('N') + '.tmp'
try {
    [IO.File]::WriteAllText($temporary, $policy, [Text.UTF8Encoding]::new($false))
    [IO.File]::Move($temporary, $policyPath)
} finally {
    Remove-Item -LiteralPath $temporary -Force -ErrorAction SilentlyContinue
}
# The directory's owner-only Windows ACL is managed by the existing
# private-oauth-state-acl script; do not weaken it or print client IDs.
Write-Output ('PROVISIONED_LEGACY_POLICY existing_approved_clients=' + $digests.Count)
