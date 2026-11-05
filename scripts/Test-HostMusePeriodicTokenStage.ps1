# No live token issuance and no changes to the canonical SOPS file.
# Verify that two JSON scalar values can be staged in an encrypted copy.
[CmdletBinding()]
param()
$ErrorActionPreference = 'Stop'
$store = Join-Path $env:USERPROFILE '.config\sops\secrets\global.sops.json'
$stage = $null
try {
    $hash = (Get-FileHash -LiteralPath $store -Algorithm SHA256).Hash
    $stage = Join-Path (Split-Path -Parent $store) ('.host-sops-muse-test-' + [guid]::NewGuid().ToString('N') + '.json')
    Copy-Item -LiteralPath $store -Destination $stage -ErrorAction Stop
    $acl = Get-Acl -LiteralPath $store
    Set-Acl -LiteralPath $stage -AclObject $acl
    $expectedSddl = $acl.Sddl -replace 'D:(P?)AI(?=\()', 'D:$1'
    $actualSddl = (Get-Acl -LiteralPath $stage).Sddl -replace 'D:(P?)AI(?=\()', 'D:$1'
    if ($expectedSddl -cne $actualSddl) { throw 'ACL mismatch' }

    $cases = @(
        [pscustomobject]@{ name = 'OPENBAO_MUSE_PERIODIC_TOKEN'; value = 'fixture-token-' + [guid]::NewGuid().ToString('N') }
        [pscustomobject]@{ name = 'OPENBAO_MUSE_PERIODIC_TOKEN_ACCESSOR'; value = 'fixture-accessor-' + [guid]::NewGuid().ToString('N') }
    )
    foreach ($entry in $cases) {
        $selector = '["' + $entry.name + '"]'
        $json = ConvertTo-Json -InputObject ([string]$entry.value) -Compress
        $json | & sops set --value-stdin $stage $selector *> $null
        if ($LASTEXITCODE -ne 0) { throw 'SOPS stage write failed' }
        $read = (& sops --decrypt --extract $selector $stage 2>$null) -join ''
        if ($LASTEXITCODE -ne 0) { throw 'SOPS stage extraction failed' }
        try { $read = [string]($read | ConvertFrom-Json -ErrorAction Stop) }
        catch { $read = [string]$read.Trim('"') }
        if ($read -cne $entry.value) { throw 'SOPS stage readback differs' }
    }
    if ((Get-FileHash -LiteralPath $store -Algorithm SHA256).Hash -cne $hash) {
        throw 'Canonical SOPS store changed during stage test'
    }
    Write-Output 'MUSE_TWO_KEY_ENCRYPTED_STAGE=PASS'
    Write-Output 'MUSE_CANONICAL_SOPS_UNCHANGED=PASS'
    exit 0
} catch {
    Write-Output 'MUSE_TWO_KEY_ENCRYPTED_STAGE=FAIL'
    exit 1
} finally {
    $cases = $null
    $read = $null
    if ($stage) { Remove-Item -LiteralPath $stage -Force -ErrorAction SilentlyContinue }
}
