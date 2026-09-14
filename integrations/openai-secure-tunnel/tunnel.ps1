# Optional integration: OpenAI Secure MCP Tunnel (see integrations/openai-secure-tunnel/README.md).
# The core gateway builds, tests, and serves without this script and without
# any OpenAI credential. Each command attempts the tunnel-client operation
# exactly once; re-run manually if it fails.

[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateSet('ensure', 'status', 'stop')]
    [string]$Action,
    [string]$Alias = 'lomway',
    [string]$ProfileDir = '',
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
$profileDir = if ([string]::IsNullOrWhiteSpace($ProfileDir)) {
    Join-Path $env:APPDATA 'lomway\secure-tunnel\profiles'
} else {
    $ProfileDir
}
$profilePath = Join-Path $profileDir "$Alias.yaml"
$legacyProfilePath = Join-Path $env:APPDATA 'local-mcp-gateway\secure-tunnel\profiles\local-mcp-gateway.yaml'
$serverUrl = 'http://127.0.0.1:17777/mcp'

if ([string]::IsNullOrWhiteSpace($client) -or -not (Test-Path -LiteralPath $client -PathType Leaf)) {
    throw 'tunnel-client was not found. Pass -ClientPath, set LOCAL_MCP_TUNNEL_CLIENT, or add tunnel-client to PATH.'
}

if ($Action -eq 'status') {
    & $client runtimes status $Alias --json
    exit $LASTEXITCODE
}

if ($Action -eq 'stop') {
    & $client runtimes stop $Alias --json
    exit $LASTEXITCODE
}

$statusOutput = & $client runtimes status $Alias --json 2>$null
$statusExit = $LASTEXITCODE
$status = $null
if ($statusExit -eq 0 -and $statusOutput) {
    try { $status = (($statusOutput -join [Environment]::NewLine) | ConvertFrom-Json) } catch {}
}
if ($status -and $status.ready -eq $true) {
    $statusOutput
    exit 0
}

$tunnelId = $null
if ($status -and $status.tunnel_id -match '^tunnel_[0-9a-f]{32}$') {
    $tunnelId = [string]$status.tunnel_id
}
if (-not $tunnelId -and (Test-Path -LiteralPath $profilePath -PathType Leaf)) {
    $profileText = Get-Content -LiteralPath $profilePath -Raw
    $match = [regex]::Match($profileText, 'tunnel_[0-9a-f]{32}')
    if ($match.Success) { $tunnelId = $match.Value }
}
if (-not $tunnelId -and $Alias -eq 'lomway' -and (Test-Path -LiteralPath $legacyProfilePath -PathType Leaf)) {
    $legacyProfileText = Get-Content -LiteralPath $legacyProfilePath -Raw
    $legacyMatch = [regex]::Match($legacyProfileText, 'tunnel_[0-9a-f]{32}')
    if ($legacyMatch.Success) { $tunnelId = $legacyMatch.Value }
}
if (-not $tunnelId) {
    throw "Secure Tunnel alias '$Alias' is not configured. Create it once before using -Action ensure."
}

if (-not $env:CONTROL_PLANE_API_KEY) {
    # Optional SOPS integration (integrations/sops): resolve the key from the
    # canonical store into the process environment. Skipped entirely when the
    # key is already present in the environment.
    $sopsHelper = Join-Path $PSScriptRoot '..\sops\Import-SopsSecrets.ps1'
    if (Test-Path -LiteralPath $sopsHelper -PathType Leaf) {
        & $sopsHelper -Names @('CONTROL_PLANE_API_KEY')
    }
}
if (-not $env:CONTROL_PLANE_API_KEY) {
    throw 'CONTROL_PLANE_API_KEY is required: set it in the environment or install the SOPS integration (integrations/sops).'
}

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
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$final = & $client runtimes status $Alias --json
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
$parsed = (($final -join [Environment]::NewLine) | ConvertFrom-Json)
if ($parsed.ready -ne $true) { throw 'Secure Tunnel runtime did not become ready after reconnect.' }
$final
