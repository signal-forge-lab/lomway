[CmdletBinding()]
param(
    [switch]$RequireIsolated
)

# Read ACL metadata only. Do NOT invoke sops decrypt, open or display a
# secret value, dump token-store records, or change any Windows permissions.
$ErrorActionPreference = 'Stop'
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = [Security.Principal.WindowsPrincipal]::new($identity)
$currentSids = [Collections.Generic.HashSet[string]]::new(
    [StringComparer]::OrdinalIgnoreCase
)
[void]$currentSids.Add($identity.User.Value)
foreach ($group in $identity.Groups) { [void]$currentSids.Add($group.Value) }
[void]$currentSids.Add('S-1-1-0') # Everyone
[void]$currentSids.Add('S-1-5-11') # Authenticated Users

$readRights = [Security.AccessControl.FileSystemRights](
    [int][Security.AccessControl.FileSystemRights]::ReadData -bor
    [int][Security.AccessControl.FileSystemRights]::ReadPermissions -bor
    [int][Security.AccessControl.FileSystemRights]::ReadAttributes
)
function Inspect-Acl([string]$Label, [string]$Path) {
    if ([string]::IsNullOrWhiteSpace($Path) -or
        -not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        return [pscustomobject]@{
            name = $Label; exists = $false; inherited = $null
            current_user_may_read = $null; check = 'MISSING'
        }
    }
    try {
        $acl = Get-Acl -LiteralPath $Path -ErrorAction Stop
        $mayRead = $false
        foreach ($rule in $acl.Access) {
            if ($rule.AccessControlType -ne
                [Security.AccessControl.AccessControlType]::Allow) { continue }
            if (-not ($rule.FileSystemRights -band $readRights)) { continue }
            $sid = $rule.IdentityReference.Translate(
                [Security.Principal.SecurityIdentifier]
            ).Value
            if ($currentSids.Contains($sid)) { $mayRead = $true }
        }
        return [pscustomobject]@{
            name = $Label; exists = $true
            inherited = -not $acl.AreAccessRulesProtected
            current_user_may_read = $mayRead
            check = $(if ($mayRead) { 'NOT_ISOLATED' } else { 'INDETERMINATE' })
        }
    } catch {
        # ACL inspection failures cannot be accepted as proof of isolation.
        return [pscustomobject]@{
            name = $Label; exists = $true; inherited = $null
            current_user_may_read = $null; check = 'INDETERMINATE'
        }
    }
}

$targets = @(
    (Inspect-Acl 'canonical_sops' (
        Join-Path $env:USERPROFILE '.config\sops\secrets\global.sops.json')),
    (Inspect-Acl 'age_identity' (
        Join-Path $env:APPDATA 'sops\age\keys.txt')),
    (Inspect-Acl 'lomway_pat_store' (
        Join-Path $env:LOCALAPPDATA 'Lomway\oauth-sidecar\runtime\pat-tokens.json'))
)
# In practice a file ACL review alone never proves a separate process identity
# has access isolation, so do not report a PASS without a service-account test.
$result = [pscustomobject]@{
    check = 'HOST_SOPS_ISOLATION_PREFLIGHT'
    verdict = 'NOT_VERIFIED'
    os_admin = $principal.IsInRole(
        [Security.Principal.WindowsBuiltInRole]::Administrator)
    mutation_policy_exists = Test-Path -LiteralPath (
        Join-Path $env:USERPROFILE '.config\sops\host-sops-policy.json')
    targets = $targets
    current_user_has_secret_access_evidence = @(
        $targets | Where-Object { $_.name -in @('canonical_sops', 'age_identity') -and
            $_.current_user_may_read }
    ).Count -gt 0
    separate_account_activation_tested = $false
}
$result | ConvertTo-Json -Depth 4 -Compress
if ($RequireIsolated) { exit 2 }
