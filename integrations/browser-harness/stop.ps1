[CmdletBinding()]
param([int]$Port = 17685)
$ErrorActionPreference = "Stop"
$runtime = Join-Path (Resolve-Path (Join-Path $PSScriptRoot "../..")).Path "runtime"
$pidFile = Join-Path $runtime "browser-harness-mcp-bridge.pid"
$listener = Get-NetTCPConnection -LocalAddress "127.0.0.1" -LocalPort $Port -ErrorAction SilentlyContinue | Where-Object { $_.RemotePort -eq 0 -and $_.OwningProcess -gt 0 } | Select-Object -First 1
if (-not $listener) {
    Remove-Item -LiteralPath $pidFile -Force -ErrorAction SilentlyContinue
    Write-Output "NOT_RUNNING"
    exit 0
}
$owner = [int]$listener.OwningProcess
$process = Get-CimInstance Win32_Process -Filter "ProcessId=$owner"
if (-not $process -or $process.CommandLine -notmatch "mcp-proxy" -or $process.CommandLine -notmatch "browser-harness-mcp") {
    throw "Refusing to stop unrelated process on port $Port."
}
& taskkill.exe /T /F /PID $owner | Out-Null
if ($LASTEXITCODE -ne 0) { throw "Failed to stop bridge process $owner." }
Remove-Item -LiteralPath $pidFile -Force -ErrorAction SilentlyContinue
Write-Output "STOPPED pid=$owner"
