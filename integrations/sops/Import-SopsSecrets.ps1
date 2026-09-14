<#
.SYNOPSIS
    Optional SOPS integration: resolve secrets from the canonical SOPS store
    into the current process environment.

.DESCRIPTION
    Decrypts the canonical SOPS store once and copies the requested secret
    values into process-level environment variables. Values are never written
    to disk and never emitted to stdout/stderr by this script.

    The core gateway does not require SOPS: plain environment variables always
    work. This helper only automates the local workflow where secrets live in
    the user's external global SOPS store.

.PARAMETER Names
    Secret names to copy from the decrypted store into the process
    environment. Missing names are skipped silently; callers validate their
    own requirements after invoking this script.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string[]]$Names
)

$ErrorActionPreference = 'Stop'

$store = [Environment]::GetEnvironmentVariable('LOCAL_MCP_SOPS_STORE', 'Process')
if ([string]::IsNullOrWhiteSpace($store)) {
    $store = Join-Path $env:USERPROFILE '.config\sops\secrets\global.sops.json'
}
if (-not (Test-Path -LiteralPath $store -PathType Leaf)) {
    throw "Canonical SOPS secret file was not found: $store"
}

$sops = (Get-Command sops -ErrorAction Stop).Source
$secretMap = (& $sops decrypt $store | Out-String | ConvertFrom-Json)
if ($LASTEXITCODE -ne 0) { throw 'Failed to decrypt the canonical SOPS secret file.' }

foreach ($name in $Names) {
    if (-not $name.Trim()) { throw 'Secret names must not be empty.' }
    $value = [string]$secretMap.PSObject.Properties[$name].Value
    if ($value) {
        [Environment]::SetEnvironmentVariable($name, $value.Trim(), 'Process')
    }
}
