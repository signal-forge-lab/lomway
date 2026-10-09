[CmdletBinding()]
param()

# Controlled, OAuth-only cutover. Workbridge management must not share this
# process, and the old build must be staged before any restart is attempted.
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$runtime = Join-Path $repo 'runtime'
$rollbackJs = Join-Path $runtime 'oauth-rollback\server.pre-csp.js'
$activeJs = Join-Path $repo 'sidecar\dist\src\server.js'
$result = Join-Path $runtime 'oauth-csp-deployment.json'
$restartUrl = 'http://127.0.0.1:17991/api/targets/lomway/components/oauth/restart'
$started = $false

function Record([string]$phase, [string]$detail) {
    @{ phase=$phase; detail=$detail; at=(Get-Date).ToString('o') } |
        ConvertTo-Json -Compress | Set-Content -LiteralPath $result -Encoding UTF8
}

function Http([string]$uri) {
    try {
        $response = Invoke-WebRequest -UseBasicParsing -Uri $uri -TimeoutSec 6
        return [int]$response.StatusCode
    } catch {
        if ($_.Exception.Response) { return [int]$_.Exception.Response.StatusCode }
        return 0
    }
}

function AwaitReady([int]$previousPid) {
    for ($i=0; $i -lt 80; $i++) {
        $sidecarStatus = & (Join-Path $PSScriptRoot 'oauth-sidecar.ps1') -Action status -Json | ConvertFrom-Json
        if (
            $sidecarStatus.ready -and
            $sidecarStatus.pid -ne $previousPid -and
            (Http 'http://127.0.0.1:7677/healthz') -eq 200 -and
            (Http 'http://127.0.0.1:17777/healthz') -eq 200 -and
            (Http 'https://mcp.maiteneru.com/.well-known/openid-configuration') -eq 200 -and
            (Http 'https://mcp.maiteneru.com/mcp') -eq 401
        ) { return $true }
        Start-Sleep -Milliseconds 500
    }
    return $false
}

if (-not (Test-Path -LiteralPath $rollbackJs -PathType Leaf)) {
    throw 'Old Sidecar JS backup is missing. Refusing cutover.'
}
if (-not (Test-Path -LiteralPath $activeJs -PathType Leaf)) {
    throw 'New Sidecar JavaScript is missing.'
}
if (-not ((Get-Content -LiteralPath $rollbackJs -Raw).Contains("form-action 'self';"))) {
    throw 'Rollback JS is not the expected pre-fix build.'
}
if (-not ((Get-Content -LiteralPath $activeJs -Raw).Contains('approvedRedirectOrigin'))) {
    throw 'Current Sidecar JS lacks the verified redirect-CSP fix.'
}
$workbridge = Get-NetTCPConnection -LocalPort 7680 -State Listen -ErrorAction Stop |
    Select-Object -First 1
$sidecarPort = Get-NetTCPConnection -LocalPort 7677 -State Listen -ErrorAction Stop |
    Select-Object -First 1
if ($workbridge.OwningProcess -eq $sidecarPort.OwningProcess) {
    throw 'Management process is not independent of OAuth. Refusing cutover.'
}
$old = & (Join-Path $PSScriptRoot 'oauth-sidecar.ps1') -Action status -Json | ConvertFrom-Json
if (-not $old.ready) { throw 'OAuth is not READY before cutover.' }
if ((Http 'https://mcp.maiteneru.com/.well-known/openid-configuration') -ne 200) {
    throw 'Public OAuth is unhealthy before cutover.'
}
$swibo = Invoke-RestMethod 'http://127.0.0.1:17991/api/status' -TimeoutSec 4
$target = $swibo.targets | Where-Object id -eq 'lomway' | Select-Object -First 1
if ($target.state -ne 'READY') { throw 'Swibo target is not READY.' }

Record 'preflight_ok' 'Independent management and old Sidecar build verified'
try {
    $started = $true
    Record 'restarting' 'Restart only OAuth component; Gateway and desktop local remain untouched'
    $null = Invoke-RestMethod -Method Post -Uri $restartUrl -TimeoutSec 45
    if (-not (AwaitReady $old.pid)) { throw 'New Sidecar did not pass public/local readiness' }
    Record 'success' 'Chromium-tested CSP fix is active; public OAuth and MCP healthy'
    Write-Output 'OAUTH_CSP_LIVE_DEPLOYMENT_PASS'
} catch {
    $reason = $_.Exception.Message
    if ($started) {
        Record 'rolling_back' $reason
        try {
            $current = & (Join-Path $PSScriptRoot 'oauth-sidecar.ps1') -Action status -Json | ConvertFrom-Json
            Copy-Item -LiteralPath $rollbackJs -Destination $activeJs -Force
            $null = Invoke-RestMethod -Method Post -Uri $restartUrl -TimeoutSec 45
            if (AwaitReady $current.pid) {
                Record 'rolled_back' $reason
            } else {
                Record 'rollback_unhealthy' $reason
            }
        } catch {
            Record 'rollback_failed' ($reason + '; rollback exception=' + $_.Exception.Message)
        }
    }
    throw ('OAuth Sidecar cutover failed: ' + $reason)
}
