[CmdletBinding()]
param([switch]$PreflightOnly)

# Historical entrypoint retained for compatibility. The original script
# performed unsafe in-place replacement of the shared Lomway executable;
# ALWAYS delegate to the isolated, versioned-binary deployment instead.
$ErrorActionPreference = 'Stop'
& (Join-Path $PSScriptRoot 'deploy-gateway-isolated.ps1') -PreflightOnly:$PreflightOnly
if ($LASTEXITCODE -ne 0) {
    throw 'Isolated Gateway deployment did not complete successfully.'
}
