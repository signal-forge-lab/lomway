[CmdletBinding()]
param(
    [int]$Port = 17777,
    [switch]$RequireOAuth
)

. (Join-Path $PSScriptRoot 'common.ps1')
$process = Get-GatewayProcess
$httpReady = $false
try {
    $response = Invoke-WebRequest -UseBasicParsing -Uri "http://127.0.0.1:$Port/healthz" -TimeoutSec 2
    $httpReady = $response.StatusCode -eq 200
} catch {}

$oauthReady = $null
if ($RequireOAuth) {
    try {
        $oauthStatus = & (Join-Path $PSScriptRoot 'oauth-sidecar.ps1') -Action status -Json | ConvertFrom-Json
        $oauthReady = [bool]$oauthStatus.ready
    } catch {
        $oauthReady = $false
    }
}

[pscustomobject]@{
    ready = [bool]($process -and $httpReady -and (-not $RequireOAuth -or $oauthReady))
    process = [bool]$process
    pid = if ($process) { $process.Id } else { $null }
    health = $httpReady
    oauth = $oauthReady
} | ConvertTo-Json -Compress
