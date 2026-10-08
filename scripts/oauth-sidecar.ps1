[CmdletBinding()]
param(
    [ValidateSet('start', 'stop', 'status')]
    [string]$Action = 'status',
    [switch]$Json
)

$ErrorActionPreference = 'Stop'

$repoRoot = Split-Path -Parent $PSScriptRoot
$sidecarRoot = Join-Path $repoRoot 'sidecar'
$runtimeRoot = Join-Path $env:LOCALAPPDATA 'Lomway\oauth-sidecar'
$stateDir = Join-Path $runtimeRoot 'runtime'
$statePath = Join-Path $stateDir 'state.json'
$pidFile = Join-Path $runtimeRoot 'sidecar.pid'
$stdout = Join-Path $runtimeRoot 'sidecar.log'
$stderr = Join-Path $runtimeRoot 'sidecar.err.log'
$entryPoint = Join-Path $sidecarRoot 'dist\src\server.js'
$sopsFile = Join-Path $HOME '.config\sops\secrets\global.sops.json'
$healthUrl = 'http://127.0.0.1:7677/healthz'

function Get-SidecarProcess {
    if (-not (Test-Path -LiteralPath $pidFile -PathType Leaf)) {
        return $null
    }

    $rawPid = (Get-Content -LiteralPath $pidFile -Raw).Trim()
    $processId = 0
    if (-not [int]::TryParse($rawPid, [ref]$processId)) {
        return $null
    }

    $process = Get-Process -Id $processId -ErrorAction SilentlyContinue
    if (-not $process -or $process.ProcessName -ne 'node') {
        return $null
    }

    $netstat = Join-Path $env:SystemRoot 'System32\netstat.exe'
    $ownsPort = $false
    foreach ($line in & $netstat -ano -p tcp 2>$null) {
        $parts = @($line.Trim() -split '\s+')
        if ($parts.Count -lt 4 -or $parts[0] -ne 'TCP') {
            continue
        }
        if ($parts[1].EndsWith(':7677') -and $parts[-1] -eq [string]$processId) {
            $ownsPort = $true
            break
        }
    }
    if (-not $ownsPort) {
        return $null
    }
    return $process
}

function Test-SidecarReady {
    param(
        [System.Diagnostics.Process]$Process
    )
    if (-not $Process) {
        return $false
    }
    try {
        $response = Invoke-WebRequest -UseBasicParsing -Uri $healthUrl -TimeoutSec 1
        return $response.StatusCode -eq 200
    } catch {
        return $false
    }
}

function Get-StatusRecord {
    $process = Get-SidecarProcess
    [ordered]@{
        ready = [bool](Test-SidecarReady -Process $process)
        pid = if ($process) { $process.Id } else { $null }
        port = 7677
        secretSource = 'sops'
    }
}

function Write-Status {
    $status = Get-StatusRecord
    if ($Json) {
        $status | ConvertTo-Json -Compress
        return
    }
    if ($status.ready) {
        Write-Output ("READY pid={0} port={1}" -f $status.pid, $status.port)
    } else {
        Write-Output ("DOWN port={0}" -f $status.port)
    }
}

function Get-OAuthSecrets {
    if (-not (Test-Path -LiteralPath $sopsFile -PathType Leaf)) {
        throw "Canonical SOPS secret store not found: $sopsFile"
    }
    if (-not (Get-Command sops -ErrorAction SilentlyContinue)) {
        throw 'sops is required to start the Lomway OAuth sidecar.'
    }

    $raw = & sops -d --extract '["lomway"]["oauth"]' --output-type json $sopsFile
    if ($LASTEXITCODE -ne 0) {
        throw 'Failed to decrypt lomway.oauth from the canonical SOPS store.'
    }
    $secrets = $raw | ConvertFrom-Json
    if ([string]::IsNullOrWhiteSpace([string]$secrets.owner_credential)) {
        throw 'lomway.oauth.owner_credential is missing from the canonical SOPS store.'
    }
    if (@($secrets.jwks.keys).Count -lt 1) {
        throw 'lomway.oauth.jwks must contain at least one signing key.'
    }
    if (@($secrets.cookie_keys).Count -lt 2) {
        throw 'lomway.oauth.cookie_keys must contain at least two keys.'
    }
    return $secrets
}

switch ($Action) {
    'status' {
        Write-Status
        exit 0
    }
    'stop' {
        $process = Get-SidecarProcess
        if (-not $process) {
            Remove-Item -LiteralPath $pidFile -Force -ErrorAction SilentlyContinue
            Write-Output 'NOT_RUNNING'
            exit 0
        }

        Stop-Process -Id $process.Id -ErrorAction Stop
        $process.WaitForExit(5000) | Out-Null
        Remove-Item -LiteralPath $pidFile -Force -ErrorAction SilentlyContinue
        Write-Output 'STOPPED'
        exit 0
    }
    'start' {
        $process = Get-SidecarProcess
        if (Test-SidecarReady -Process $process) {
            Write-Output ("ALREADY_RUNNING pid={0}" -f $process.Id)
            exit 0
        }
        try {
            $occupied = (Invoke-WebRequest -UseBasicParsing -Uri $healthUrl -TimeoutSec 1).StatusCode -eq 200
        } catch {
            $occupied = $false
        }
        if ($occupied) {
            throw 'Port 7677 is already serving a process not owned by the Lomway OAuth sidecar runtime.'
        }
        if (-not (Test-Path -LiteralPath $entryPoint -PathType Leaf)) {
            throw 'OAuth sidecar build is missing. Run npm run build in sidecar before starting Lomway.'
        }

        $secrets = Get-OAuthSecrets
        New-Item -ItemType Directory -Force -Path $runtimeRoot, $stateDir | Out-Null
        & (Join-Path $PSScriptRoot 'private-oauth-state-acl.ps1') -RuntimeDirectory $stateDir | Out-Null

        $names = @(
            'LOMWAY_OAUTH_BIND_HOST',
            'LOMWAY_OAUTH_PORT',
            'LOMWAY_OAUTH_ISSUER',
            'LOMWAY_OAUTH_RESOURCE_URL',
            'LOMWAY_OAUTH_OWNER_CREDENTIAL',
            'LOMWAY_OAUTH_JWKS_JSON',
            'LOMWAY_OAUTH_COOKIE_KEYS_JSON',
            'LOMWAY_OAUTH_RUNTIME_DIR',
            'LOMWAY_OAUTH_STATE_PATH'
        )
        try {
            $env:LOMWAY_OAUTH_BIND_HOST = '127.0.0.1'
            $env:LOMWAY_OAUTH_PORT = '7677'
            $env:LOMWAY_OAUTH_ISSUER = 'https://mcp.maiteneru.com'
            $env:LOMWAY_OAUTH_RESOURCE_URL = 'https://mcp.maiteneru.com/mcp'
            $env:LOMWAY_OAUTH_OWNER_CREDENTIAL = [string]$secrets.owner_credential
            $env:LOMWAY_OAUTH_JWKS_JSON = $secrets.jwks | ConvertTo-Json -Compress -Depth 20
            $env:LOMWAY_OAUTH_COOKIE_KEYS_JSON = $secrets.cookie_keys | ConvertTo-Json -Compress -Depth 20
            $env:LOMWAY_OAUTH_RUNTIME_DIR = $stateDir
            $env:LOMWAY_OAUTH_STATE_PATH = $statePath

            $started = Start-Process -FilePath 'node.exe' `
                -ArgumentList 'dist/src/server.js' `
                -WorkingDirectory $sidecarRoot `
                -WindowStyle Hidden `
                -RedirectStandardOutput $stdout `
                -RedirectStandardError $stderr `
                -PassThru
            Set-Content -LiteralPath $pidFile -Value $started.Id -Encoding ascii -NoNewline
        } finally {
            foreach ($name in $names) {
                Remove-Item "Env:$name" -ErrorAction SilentlyContinue
            }
            $secrets = $null
        }

        for ($i = 0; $i -lt 60; $i++) {
            Start-Sleep -Milliseconds 250
            if ($started.HasExited) {
                Remove-Item -LiteralPath $pidFile -Force -ErrorAction SilentlyContinue
                throw "OAuth sidecar exited during startup with code $($started.ExitCode). See sidecar.err.log."
            }
            if (Test-SidecarReady -Process $started) {
                Write-Output ("STARTED pid={0}" -f $started.Id)
                exit 0
            }
        }

        throw 'OAuth sidecar did not become ready within 15 seconds. See sidecar.err.log.'
    }
}
