[CmdletBinding()]
param()

# This script is run as a detached local Windows process. The Swibo action
# intentionally disconnects the invoking Lomway MCP session; rollback cannot
# depend on the caller remaining connected.
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$staged = Join-Path $repo 'target\pat-release\release\lomway.exe'
$live = Join-Path $repo 'target\release\lomway.exe'
$runtime = Join-Path $repo 'runtime'
$backup = Join-Path $runtime 'lomway-pre-pat-gateway.exe'
$stateFile = Join-Path $runtime 'pat-deployment.json'
$endpoint = 'http://127.0.0.1:17991/api/targets/lomway'

function Record([string]$phase, [string]$detail) {
    @{ phase = $phase; detail = $detail; at = (Get-Date).ToString('o') } |
        ConvertTo-Json -Compress | Set-Content -LiteralPath $stateFile -Encoding utf8
}
function Target([string]$action) {
    $null = Invoke-RestMethod -Uri "$endpoint/$action" -Method Post -TimeoutSec 45
}
function Healthy {
    try {
        $local = Invoke-WebRequest -UseBasicParsing 'http://127.0.0.1:17777/healthz' -TimeoutSec 2
        $oauth = Invoke-WebRequest -UseBasicParsing 'http://127.0.0.1:7677/healthz' -TimeoutSec 2
        $public = Invoke-WebRequest -UseBasicParsing 'https://mcp.maiteneru.com/.well-known/openid-configuration' -TimeoutSec 5
        return ($local.StatusCode -eq 200 -and $oauth.StatusCode -eq 200 -and $public.StatusCode -eq 200)
    } catch {
        return $false
    }
}
function AwaitHealthy {
    for ($i = 0; $i -lt 50; $i++) {
        if (Healthy) { return $true }
        Start-Sleep -Milliseconds 600
    }
    return $false
}

New-Item -ItemType Directory -Path $runtime -Force | Out-Null
Record 'prepared' 'Detached deployment started'
Start-Sleep -Seconds 4
try {
    if (-not (Test-Path $staged) -or -not (Test-Path $live)) {
        throw 'Missing staged or current gateway executable'
    }
    Copy-Item -LiteralPath $live -Destination $backup -Force
    Record 'stopping' 'Backed up the current gateway'
    Target 'stop'
    Record 'replacing' 'Swibo target stopped'
    Copy-Item -LiteralPath $staged -Destination $live -Force
    if ((Get-FileHash $staged -Algorithm SHA256).Hash -ne
        (Get-FileHash $live -Algorithm SHA256).Hash) {
        throw 'Staged and deployed binaries differ'
    }
    Target 'start'
    Record 'verifying' 'New gateway start requested'
    if (-not (AwaitHealthy)) {
        throw 'New gateway/OAuth/public issuer readiness failed'
    }
    Record 'success' 'PAT-aware gateway and OAuth sidecar healthy'
} catch {
    $reason = $_.Exception.Message
    Record 'rolling_back' $reason
    try { Target 'stop' } catch {}
    try {
        if (Test-Path $backup) {
            Copy-Item -LiteralPath $backup -Destination $live -Force
        }
        Target 'start'
        if (AwaitHealthy) {
            Record 'rolled_back' $reason
        } else {
            Record 'rollback_unhealthy' $reason
        }
    } catch {
        Record 'rollback_failed' ($reason + '; ' + $_.Exception.Message)
    }
    exit 1
}
