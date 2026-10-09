[CmdletBinding()]
param([switch]$Public, [switch]$AllTools)

# A disposable one-day token is issued strictly in this PowerShell process,
# never written to logs or clipboard, and revoked in a finally block.
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$cli = Join-Path $repo 'sidecar\dist\src\pat-cli.js'
$token = $null
$id = $null
$targetTool = 'praxiom_praxiom_status'
$local = if ($Public) { 'https://mcp.maiteneru.com/mcp' } else { 'http://127.0.0.1:17777/mcp' }
$headers = @{}
$session = $null

function Request-Status([string]$uri, [string]$method, [object]$body) {
    $parameters = @{
        Uri=$uri
        Method=$method
        Headers=$headers
        ContentType='application/json'
        TimeoutSec=12
        SkipHttpErrorCheck=$true
    }
    if ($null -ne $body) {
        $parameters.Body = ($body | ConvertTo-Json -Depth 12 -Compress)
    }
    return Invoke-WebRequest @parameters
}

function Find-RpcResult($response) {
    $raw = [string]$response.Content
    if ($raw -match '^\s*\{') {
        return ($raw | ConvertFrom-Json -Depth 40)
    }
    foreach ($line in ($raw -split '\r?\n')) {
        if ($line -match '^data:\s*(\{.*\})') {
            $candidate = $Matches[1] | ConvertFrom-Json -Depth 40
            if ($candidate.result) { return $candidate }
        }
    }
    throw 'MCP response did not contain a JSON-RPC result'
}

try {
    if (!(Test-Path -LiteralPath $cli)) { throw 'PAT CLI is missing' }
    if ($AllTools) {
        $issued = & node.exe $cli issue --label 'disposable-full-access-live-probe' --days 1 --all-tools
    } else {
        $issued = & node.exe $cli issue --label 'disposable-live-probe' --days 1 --tools $targetTool
    }
    if ($LASTEXITCODE -ne 0) { throw 'PAT issue failed' }
    $record = $issued | ConvertFrom-Json
    $token = [string]$record.token
    $id = [string]$record.id
    if (!$token -or !$id) { throw 'PAT issue returned no credential or ID' }
    $issued = $null
    $record = $null

    $headers = @{ Authorization = "Bearer $token"; Accept = 'application/json, text/event-stream' }

    $init = Request-Status $local 'POST' @{
        jsonrpc='2.0'; id=1; method='initialize'
        params=@{
            protocolVersion='2025-06-18'
            capabilities=@{}
            clientInfo=@{ name='lomway-pat-live-probe'; version='1.0' }
        }
    }
    if ($init.StatusCode -ne 200) { throw "PAT MCP initialize: HTTP $($init.StatusCode)" }
    $null = Find-RpcResult $init
    if ($init.Headers['mcp-session-id']) {
        $session = [string]$init.Headers['mcp-session-id']
        $headers['mcp-session-id'] = $session
    }
    Write-Output 'PAT_INITIALIZE_200'

    $initialized = Request-Status $local 'POST' @{
        jsonrpc='2.0'; method='notifications/initialized'
    }
    if ($initialized.StatusCode -notin @(200,202,204)) {
        throw "PAT notifications/initialized: HTTP $($initialized.StatusCode)"
    }
    Write-Output 'PAT_SESSION_INITIALIZED'

    $list = Request-Status $local 'POST' @{ jsonrpc='2.0'; id=2; method='tools/list'; params=@{} }
    if ($list.StatusCode -ne 200) { throw "PAT tools/list: HTTP $($list.StatusCode)" }
    $listed = Find-RpcResult $list
    $names = @($listed.result.tools | ForEach-Object { $_.name })
    if ($AllTools) {
        if ($names.Count -lt 2 -or $targetTool -notin $names -or
            'lomway_call_tool' -notin $names -or
            @($names | Where-Object { $_ -like 'proxy/*' }).Count -gt 0) {
            throw ('Full-access PAT catalog mismatch: observed count=' + $names.Count)
        }
        Write-Output ('PAT_ALL_TOOLS_LIST_OK count=' + $names.Count)
    } elseif ($names.Count -ne 1 -or $names[0] -ne $targetTool) {
        throw ('PAT tool allowlist mismatch: observed count=' + $names.Count)
    }
    Write-Output 'PAT_TOOL_LIST_FILTER_OK'

    $deniedParams = if ($AllTools) {
        @{ name='proxy/config'; arguments=@{} }
    } else {
        @{ name='lomway_call_tool'; arguments=@{ name=$targetTool; arguments=@{} } }
    }
    $denied = Request-Status $local 'POST' @{
        jsonrpc='2.0'; id=3; method='tools/call'
        params=$deniedParams
    }
    if ($denied.StatusCode -ne 403) { throw "PAT forbidden tool: HTTP $($denied.StatusCode)" }
    Write-Output 'PAT_FORBIDDEN_TOOL_403'

    $allowed = Request-Status $local 'POST' @{
        jsonrpc='2.0'; id=4; method='tools/call'
        params=@{ name=$targetTool; arguments=@{} }
    }
    if ($allowed.StatusCode -ne 200) {
        throw "PAT approved tool: HTTP $($allowed.StatusCode)"
    }
    $allowedResult = Find-RpcResult $allowed
    if ($allowedResult.error -or !$allowedResult.result -or $allowedResult.result.isError) {
        throw 'PAT approved tool returned an MCP error result'
    }
    Write-Output 'PAT_ALLOWED_TOOL_HTTP_200'
}
finally {
    if ($id) {
        $revoked = & node.exe $cli revoke --id $id
        if ($LASTEXITCODE -ne 0) { throw 'Probe PAT could not be revoked' }
        $revoked = $null
        if ($token) {
            $after = Request-Status $local 'POST' @{jsonrpc='2.0'; id=5; method='tools/list'; params=@{}}
            if ($after.StatusCode -ne 401) {
                throw "Revoked PAT request: HTTP $($after.StatusCode)"
            }
            Write-Output 'PAT_REVOKED_401'
        }
    }
    $token = $null
    $headers = @{}
}
