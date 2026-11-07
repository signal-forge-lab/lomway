[CmdletBinding()]
param([int]$Port = 17685)
$ErrorActionPreference = "Stop"
$root = (Resolve-Path (Join-Path $PSScriptRoot "../..")).Path
$runtime = Join-Path $root "runtime"
New-Item -ItemType Directory -Force -Path $runtime | Out-Null
$pidFile = Join-Path $runtime "browser-harness-mcp-bridge.pid"

function Get-BridgeListener {
    $listener = Get-NetTCPConnection -LocalAddress "127.0.0.1" -LocalPort $Port -ErrorAction SilentlyContinue | Where-Object { $_.RemotePort -eq 0 -and $_.OwningProcess -gt 0 } | Select-Object -First 1
    if (-not $listener) { return $null }
    $process = Get-CimInstance Win32_Process -Filter "ProcessId=$($listener.OwningProcess)"
    if (-not $process -or $process.CommandLine -notmatch "mcp-proxy" -or $process.CommandLine -notmatch "browser-harness-mcp") {
        throw "TCP port $Port is used by a different process; refusing to replace it."
    }
    return $listener.OwningProcess
}

$existing = Get-BridgeListener
if ($existing) {
    Set-Content -Path $pidFile -Value $existing -NoNewline
    Write-Output "ALREADY_RUNNING pid=$existing port=$Port"
    exit 0
}

$proxy = (Get-Command mcp-proxy.exe -ErrorAction Stop).Source
$harness = (Get-Command browser-harness-mcp.exe -ErrorAction Stop).Source
$process = Start-Process -FilePath $proxy -ArgumentList @("--host", "127.0.0.1", "--port", "$Port", "--", $harness) -WorkingDirectory $root -WindowStyle Hidden -RedirectStandardOutput (Join-Path $runtime "browser-harness-mcp-bridge.stdout.log") -RedirectStandardError (Join-Path $runtime "browser-harness-mcp-bridge.stderr.log") -PassThru
for ($i = 0; $i -lt 40; $i++) {
    Start-Sleep -Milliseconds 500
    $owner = Get-BridgeListener
    if ($owner) {
        Set-Content -Path $pidFile -Value $owner -NoNewline
        Write-Output "STARTED pid=$owner port=$Port"
        exit 0
    }
    if ($process.HasExited) { throw "MCP bridge exited with $($process.ExitCode). Check runtime logs." }
}
throw "Browser Harness MCP bridge did not listen on $Port within 20 seconds."
