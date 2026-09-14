[CmdletBinding()]
param(
    [int]$Port = 17777
)

. (Join-Path $PSScriptRoot 'common.ps1')
$process = Get-GatewayProcess
$httpReady = $false
try {
    $response = Invoke-WebRequest -UseBasicParsing -Uri "http://127.0.0.1:$Port/healthz" -TimeoutSec 2
    $httpReady = $response.StatusCode -eq 200
} catch {}

[pscustomobject]@{
    ready = [bool]($process -and $httpReady)
    process = [bool]$process
    pid = if ($process) { $process.Id } else { $null }
    health = $httpReady
} | ConvertTo-Json -Compress
