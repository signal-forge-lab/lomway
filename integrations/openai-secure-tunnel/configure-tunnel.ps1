# Optional integration: OpenAI Secure MCP Tunnel (see integrations/openai-secure-tunnel/README.md).
# One-time creation/reuse of the Secure Tunnel alias. The core gateway builds,
# tests, and serves without this script and without any OpenAI credential.
# The client operations run exactly once each; re-run manually on failure.

[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$WorkspaceId,
    [string]$Alias = 'lomway',
    [string]$ClientPath = ''
)

$ErrorActionPreference = 'Stop'

$client = $ClientPath
if ([string]::IsNullOrWhiteSpace($client)) {
    $client = [Environment]::GetEnvironmentVariable('LOCAL_MCP_TUNNEL_CLIENT', 'Process')
}
if ([string]::IsNullOrWhiteSpace($client)) {
    $clientCommand = Get-Command 'tunnel-client.exe' -CommandType Application -ErrorAction SilentlyContinue
    if (-not $clientCommand) {
        $clientCommand = Get-Command 'tunnel-client' -CommandType Application -ErrorAction SilentlyContinue
    }
    if ($clientCommand) { $client = $clientCommand.Source }
}
$profileDir = Join-Path $env:APPDATA 'lomway\secure-tunnel\profiles'
$serverUrl = 'http://127.0.0.1:17777/mcp'

if ([string]::IsNullOrWhiteSpace($client) -or -not (Test-Path -LiteralPath $client -PathType Leaf)) {
    throw 'tunnel-client was not found. Pass -ClientPath, set LOCAL_MCP_TUNNEL_CLIENT, or add tunnel-client to PATH.'
}
if (-not $WorkspaceId.Trim()) { throw 'WorkspaceId must not be empty.' }

try {
    $health = Invoke-WebRequest -UseBasicParsing -Uri 'http://127.0.0.1:17777/healthz' -TimeoutSec 2
    if ($health.StatusCode -ne 200) { throw 'Gateway health check did not return HTTP 200.' }
} catch {
    throw 'Lomway must be running and healthy before configuring its Secure Tunnel.'
}

$oldAdmin = [Environment]::GetEnvironmentVariable('OPENAI_ADMIN_KEY', 'Process')
$oldRuntime = [Environment]::GetEnvironmentVariable('CONTROL_PLANE_API_KEY', 'Process')
try {
    if (-not $env:OPENAI_ADMIN_KEY -or -not $env:CONTROL_PLANE_API_KEY) {
        # Optional SOPS integration (integrations/sops): resolve the keys from
        # the canonical store into the process environment. Skipped entirely
        # when both keys are already present in the environment.
        $sopsHelper = Join-Path $PSScriptRoot '..\sops\Import-SopsSecrets.ps1'
        if (-not (Test-Path -LiteralPath $sopsHelper -PathType Leaf)) {
            throw 'SOPS integration helper was not found. Set OPENAI_ADMIN_KEY and CONTROL_PLANE_API_KEY in the environment or install integrations/sops.'
        }
        & $sopsHelper -Names @('OPENAI_ADMIN_KEY', 'CONTROL_PLANE_API_KEY')
    }
    $adminKey = [Environment]::GetEnvironmentVariable('OPENAI_ADMIN_KEY', 'Process')
    $runtimeKey = [Environment]::GetEnvironmentVariable('CONTROL_PLANE_API_KEY', 'Process')
    if (-not $adminKey -or -not $runtimeKey) {
        throw 'OPENAI_ADMIN_KEY and CONTROL_PLANE_API_KEY must exist in the current process or the canonical SOPS store.'
    }

    $createOutput = & $client runtimes create `
        --alias $Alias `
        --name 'lomway' `
        --description 'Secure MCP tunnel for Lomway' `
        --workspace-id $WorkspaceId `
        --admin-key 'env:OPENAI_ADMIN_KEY' `
        --json
    if ($LASTEXITCODE -ne 0) { throw "Failed to create or reuse Secure Tunnel alias '$Alias'." }

    $text = $createOutput -join [Environment]::NewLine
    $match = [regex]::Match($text, 'tunnel_[0-9a-f]{32}')
    if (-not $match.Success) { throw 'Secure Tunnel was created/reused but no tunnel id was returned.' }
    $tunnelId = $match.Value

    New-Item -ItemType Directory -Force -Path $profileDir | Out-Null
    & $client runtimes connect `
        --alias $Alias `
        --profile $Alias `
        --profile-dir $profileDir `
        --mcp-server-url $serverUrl `
        --runtime-api-key 'env:CONTROL_PLANE_API_KEY' `
        --tunnel-client-bin $client `
        --tunnel-id $tunnelId `
        --json
    if ($LASTEXITCODE -ne 0) { throw 'Secure Tunnel runtime connect failed.' }

    & $client runtimes status $Alias --json
    if ($LASTEXITCODE -ne 0) { throw 'Secure Tunnel runtime status failed after connect.' }
} finally {
    [Environment]::SetEnvironmentVariable('OPENAI_ADMIN_KEY', $oldAdmin, 'Process')
    [Environment]::SetEnvironmentVariable('CONTROL_PLANE_API_KEY', $oldRuntime, 'Process')
    $adminKey = $null
    $runtimeKey = $null
}
