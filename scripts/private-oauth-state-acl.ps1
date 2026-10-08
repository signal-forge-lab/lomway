[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$RuntimeDirectory,
    [switch]$VerifyOnly
)

$ErrorActionPreference = 'Stop'
if (-not (Test-Path -LiteralPath $RuntimeDirectory -PathType Container)) {
    throw 'OAuth state directory does not exist.'
}

$owner = [System.Security.Principal.WindowsIdentity]::GetCurrent().User
$system = New-Object System.Security.Principal.SecurityIdentifier 'S-1-5-18'
$admins = New-Object System.Security.Principal.SecurityIdentifier 'S-1-5-32-544'
$allowedSids = @($owner.Value, $system.Value, $admins.Value)

function Assert-PrivateAcl {
    param([string]$Path)
    $applied = Get-Acl -LiteralPath $Path
    foreach ($entry in $applied.Access) {
        $sid = $entry.IdentityReference.Translate([System.Security.Principal.SecurityIdentifier])
        if ($sid.Value -notin $allowedSids) {
            throw "Unexpected identity retains access to OAuth state at '$Path'."
        }
    }
}

function Lock-Acl {
    param([string]$Path, [bool]$Directory)
    $acl = Get-Acl -LiteralPath $Path
    $acl.SetAccessRuleProtection($true, $false)
    foreach ($entry in @($acl.Access)) {
        $acl.RemoveAccessRuleSpecific($entry) | Out-Null
    }
    foreach ($sid in @($owner, $system, $admins)) {
        $inherit = if ($Directory) {
            [System.Security.AccessControl.InheritanceFlags]'ContainerInherit, ObjectInherit'
        } else {
            [System.Security.AccessControl.InheritanceFlags]::None
        }
        $rule = New-Object System.Security.AccessControl.FileSystemAccessRule (
            $sid,
            [System.Security.AccessControl.FileSystemRights]::FullControl,
            $inherit,
            [System.Security.AccessControl.PropagationFlags]::None,
            [System.Security.AccessControl.AccessControlType]::Allow
        )
        $acl.AddAccessRule($rule)
    }
    Set-Acl -LiteralPath $Path -AclObject $acl
    Assert-PrivateAcl -Path $Path
}

if ($VerifyOnly) {
    Assert-PrivateAcl -Path $RuntimeDirectory
    Get-ChildItem -LiteralPath $RuntimeDirectory -Force -Recurse |
        ForEach-Object { Assert-PrivateAcl -Path $_.FullName }
    Write-Output 'OAUTH_RUNTIME_ACL_VERIFIED'
} else {
    Lock-Acl -Path $RuntimeDirectory -Directory $true
    Get-ChildItem -LiteralPath $RuntimeDirectory -Force -Recurse |
        ForEach-Object { Lock-Acl -Path $_.FullName -Directory $_.PSIsContainer }
    Write-Output 'OAUTH_RUNTIME_ACL_PRIVATE'
}
