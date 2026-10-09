[CmdletBinding()]
param([switch]$VerifyOnly)

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
$clients = @($state.records.Client.PSObject.Properties | ForEach-Object Name)
$approvedIds = @($state.records.Grant.PSObject.Properties |
    Where-Object { $_.Value.clientId -in $clients -and $_.Value.exp -gt $now } |
    ForEach-Object { [string]$_.Value.clientId } | Sort-Object -Unique)
$digests = @($approvedIds | ForEach-Object {
    $bytes = [Text.Encoding]::UTF8.GetBytes($_)
    'sha256:' + [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($bytes)).ToLowerInvariant()
} | Sort-Object -Unique)
if (Test-Path -LiteralPath $policyPath -PathType Leaf) {
    $existing = Get-Content -LiteralPath $policyPath -Raw | ConvertFrom-Json -Depth 20
    if ($existing.version -ne 1) { throw 'Existing policy has invalid version' }
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
