[CmdletBinding()]
param([ValidateSet('start','stop','status')][string]$Action = 'status')

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$dir = Join-Path $repo 'runtime/browser-use-mcp'
$pidFile = Join-Path $dir 'proxy.pid'
$proxyExe = Join-Path $HOME '.local/bin/mcp-proxy.exe'
$browserExe = Join-Path $HOME '.local/bin/browser-use.exe'
$port = 17837

function Get-OwnedProcess {
    if (-not (Test-Path -LiteralPath $pidFile -PathType Leaf)) { return $null }
    $rawId = (Get-Content -LiteralPath $pidFile -Raw).Trim()
    if ($rawId -notmatch '^\d+$') { throw 'Unexpected Browser Use PID file content' }
    $process = Get-CimInstance Win32_Process -Filter "ProcessId=$rawId" -ErrorAction SilentlyContinue
    if (-not $process) { return $null }
    if (-not ([string]::Equals($process.ExecutablePath, $proxyExe, [StringComparison]::OrdinalIgnoreCase) -and
            $process.CommandLine -match '(?<!\d)17837(?!\d)' -and
            $process.CommandLine -match 'browser-use\.exe' -and
            $process.CommandLine -match '--mcp')) {
        throw 'Browser Use PID belongs to an unexpected process; refusing to manage it'
    }
    return $process
}

function Test-Ready {
    try {
        $response = Invoke-WebRequest -UseBasicParsing -Uri "http://127.0.0.1:$port/status" -TimeoutSec 2
        return $response.StatusCode -eq 200
    } catch { return $false }
}

New-Item -ItemType Directory -Path $dir -Force | Out-Null
$owned = Get-OwnedProcess
switch ($Action) {
    'status' {
        @{ ready=([bool]$owned -and (Test-Ready)); pid=if ($owned) { $owned.ProcessId } else { $null } } |
            ConvertTo-Json -Compress
        break
    }
    'start' {
        if ($owned) {
            if (Test-Ready) { Write-Output "ALREADY_RUNNING pid=$($owned.ProcessId)"; break }
            throw 'Browser Use proxy is running but not ready; inspect its logs before restart'
        }
        if ((Get-NetTCPConnection -LocalPort $port -ErrorAction SilentlyContinue |
            Where-Object { $_.LocalAddress -eq '127.0.0.1' -and $_.RemotePort -eq 0 -and $_.OwningProcess -gt 0 })) {
            throw "Port $port belongs to another process"
        }
        foreach ($exe in @($proxyExe, $browserExe)) {
            if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) { throw "Executable not found: $exe" }
        }
        $p = Start-Process -FilePath $proxyExe -ArgumentList @(
            '--host=127.0.0.1', "--port=$port", '--', ('"{0}"' -f $browserExe), '--mcp'
        ) -WorkingDirectory $dir -WindowStyle Hidden -RedirectStandardOutput (Join-Path $dir 'stdout.log') -RedirectStandardError (Join-Path $dir 'stderr.log') -Environment @{
            'ANONYMIZED_TELEMETRY' = 'false'
            'BROWSER_USE_LOGGING_LEVEL' = 'critical'
            'BROWSER_USE_SETUP_LOGGING' = 'false'
        } -PassThru
        Set-Content -LiteralPath $pidFile -Value $p.Id -NoNewline
        for ($i = 0; $i -lt 120; $i++) {
            if ($p.HasExited) { throw "Browser Use proxy exited: $($p.ExitCode)" }
            if (Test-Ready) { Write-Output "STARTED pid=$($p.Id)"; break }
            Start-Sleep -Milliseconds 250
        }
        if (-not (Test-Ready)) { throw "Browser Use proxy failed to become ready on $port" }
        break
    }
    'stop' {
        if ($owned) {
            & taskkill.exe /PID $owned.ProcessId /T /F | Out-Null
            if ($LASTEXITCODE -ne 0) { throw "Failed to stop Browser Use PID=$($owned.ProcessId)" }
        }
        Remove-Item -LiteralPath $pidFile -Force -ErrorAction SilentlyContinue
        Write-Output 'STOPPED'
        break
    }
}
