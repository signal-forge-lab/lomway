[CmdletBinding()]
param(
    [string]$Url = 'http://127.0.0.1:17777/mcp',
    [switch]$CatalogOnly
)

$ErrorActionPreference = 'Stop'

function ConvertFrom-McpResponse {
    param([Parameter(Mandatory)][string]$Content)

    $trimmed = $Content.Trim()
    if ($trimmed.StartsWith('{')) {
        return $trimmed | ConvertFrom-Json -Depth 100
    }

    $dataLine = $Content -split '\r?\n' |
        Where-Object { $_ -like 'data: *' } |
        Select-Object -First 1
    if (-not $dataLine) { return $null }
    return $dataLine.Substring(6) | ConvertFrom-Json -Depth 100
}

function Invoke-McpPost {
    param(
        [Parameter(Mandatory)][hashtable]$Body,
        [hashtable]$Headers = @{}
    )

    $requestHeaders = @{ Accept = 'application/json, text/event-stream' }
    foreach ($entry in $Headers.GetEnumerator()) {
        $requestHeaders[$entry.Key] = $entry.Value
    }
    $json = $Body | ConvertTo-Json -Depth 100 -Compress
    return Invoke-WebRequest -UseBasicParsing -Method Post -Uri $Url `
        -Headers $requestHeaders -ContentType 'application/json' -Body $json -TimeoutSec 30
}

$initialize = Invoke-McpPost -Body @{
    jsonrpc = '2.0'
    id = 1
    method = 'initialize'
    params = @{
        protocolVersion = '2025-06-18'
        capabilities = @{}
        clientInfo = @{ name = 'lomway-smoke'; version = '0.1.0' }
    }
}

$sessionId = [string]$initialize.Headers['mcp-session-id']
if (-not $sessionId) { throw 'Gateway did not return an MCP session id.' }
$sessionHeaders = @{ 'Mcp-Session-Id' = $sessionId }

Invoke-McpPost -Headers $sessionHeaders -Body @{
    jsonrpc = '2.0'
    method = 'notifications/initialized'
} | Out-Null

$listResponse = Invoke-McpPost -Headers $sessionHeaders -Body @{
    jsonrpc = '2.0'
    id = 2
    method = 'tools/list'
    params = @{}
}
$list = ConvertFrom-McpResponse -Content $listResponse.Content
if (-not $list -or -not $list.result) { throw 'tools/list returned no MCP result.' }

$tools = @($list.result.tools)
$requiredPrefixes = @('workbridge_', 'memory_', 'ufo_', 'browser_', 'xmind_', 'praxiom_')
$observedPrefixes = @($requiredPrefixes + 'chrome_')
$counts = [ordered]@{}
foreach ($prefix in $observedPrefixes) {
    $counts[$prefix.TrimEnd('_')] = @($tools | Where-Object { $_.name -like "$prefix*" }).Count
}

$proxyTools = @($tools | Where-Object { $_.name -like 'proxy_*' })
$summary = [ordered]@{
    tool_count = $tools.Count
    namespace_counts = $counts
    proxy_tool_count = $proxyTools.Count
}
if ($CatalogOnly) { $summary.tools = @($tools | ForEach-Object { $_.name }) }
$summary | ConvertTo-Json -Depth 10

if ($proxyTools.Count -ne 0) { throw 'Client-visible proxy control-plane tools were found.' }
foreach ($prefix in $requiredPrefixes) {
    if (@($tools | Where-Object { $_.name -like "$prefix*" }).Count -eq 0) {
        throw "No tools were found for namespace '$prefix'."
    }
}

if ($CatalogOnly) { exit 0 }

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$calls = @(
    @{ name = 'workbridge_open_workspace'; arguments = @{ path = $repoRoot; mode = 'checkout' } },
    @{ name = 'memory_app_server_info'; arguments = @{} },
    @{ name = 'ufo_observe_get_desktop_app_info'; arguments = @{} },
    @{ name = 'browser_list_instances'; arguments = @{} },
    @{ name = 'xmind_workboard_get_status'; arguments = @{} },
    @{ name = 'praxiom_praxiom_status'; arguments = @{} },
    @{ name = 'chrome_devtools_list_pages'; arguments = @{} }
)

$callResults = [ordered]@{}
$id = 10
foreach ($call in $calls) {
    $response = Invoke-McpPost -Headers $sessionHeaders -Body @{
        jsonrpc = '2.0'
        id = $id
        method = 'tools/call'
        params = @{
            name = $call.name
            arguments = $call.arguments
        }
    }
    $decoded = ConvertFrom-McpResponse -Content $response.Content
    if (-not $decoded) { throw "No MCP response for $($call.name)." }
    if ($decoded.error) {
        throw "$($call.name) returned JSON-RPC error $($decoded.error.code): $($decoded.error.message)"
    }
    if (-not $decoded.result) { throw "$($call.name) returned no result." }
    if ($decoded.result.isError -eq $true) {
        $detail = @($decoded.result.content | ForEach-Object { $_.text } | Where-Object { $_ }) -join ' '
        if ($detail.Length -gt 600) { $detail = $detail.Substring(0, 600) + '...' }
        throw "$($call.name) returned a tool error: $detail"
    }
    $callResults[$call.name] = 'ok'
    $id++
}

[ordered]@{
    representative_calls = $callResults
} | ConvertTo-Json -Depth 5
