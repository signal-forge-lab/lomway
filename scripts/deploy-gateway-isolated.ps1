[CmdletBinding()]
param(
    [string]$Source = 'target\pat-release\release\lomway.exe',
    [switch]$PreflightOnly
)

# Gateway-only cutover. No attempt is made to replace the running binary used
# by lomway-desktop-local (:17778). The management plane (:7680/:7681) and
# Swibo/OAuth/tunnel processes are not stopped.
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$runtime = Join-Path $repo 'runtime'
$binDir = Join-Path $runtime 'bin'
$pointer = Join-Path $runtime 'gateway-binary-path.txt'
$statusFile = Join-Path $runtime 'pat-deployment.json'
$oldDefault = Join-Path $repo 'target\release\lomway.exe'
$sourcePath = if ([IO.Path]::IsPathRooted($Source)) { $Source } else { Join-Path $repo $Source }
$config = Join-Path $repo 'config\proxy.direct.local.toml'
$oldPointer = $null
$hadPointer = Test-Path -LiteralPath $pointer -PathType Leaf
$stopped = $false
$cutoverStarted = $false

function Record([string]$phase, [string]$detail) {
    @{ phase=$phase; detail=$detail; at=(Get-Date).ToString('o') } |
        ConvertTo-Json -Compress | Set-Content -LiteralPath $statusFile -Encoding UTF8
}

function Get-HttpStatus([string]$url) {
    try {
        $r = Invoke-WebRequest -UseBasicParsing -Uri $url -TimeoutSec 4
        return [int]$r.StatusCode
    } catch {
        if ($_.Exception.Response) { return [int]$_.Exception.Response.StatusCode }
        return 0
    }
}

function Wait-Gateway([string]$expectedExe) {
    for ($i=0; $i -lt 45; $i++) {
        $pidPath = Join-Path $runtime 'gateway.pid'
        if (Test-Path $pidPath) {
            $idValue = (Get-Content -LiteralPath $pidPath -Raw).Trim()
            if ($idValue -match '^\d+$') {
                $p = Get-Process -Id ([int]$idValue) -ErrorAction SilentlyContinue
                if ($p -and [string]::Equals(
                    [IO.Path]::GetFullPath($p.Path),
                    [IO.Path]::GetFullPath($expectedExe),
                    [StringComparison]::OrdinalIgnoreCase
                ) -and (Get-HttpStatus 'http://127.0.0.1:17777/healthz') -eq 200) {
                    return $true
                }
            }
        }
        Start-Sleep -Milliseconds 350
    }
    return $false
}

function Set-ExePointer([string]$value) {
    if ([string]::IsNullOrWhiteSpace($value)) {
        Remove-Item -LiteralPath $pointer -Force -ErrorAction SilentlyContinue
    } else {
        [IO.File]::WriteAllText($pointer, ($value + [Environment]::NewLine))
    }
}

function Invoke-GatewayStart {
    & (Join-Path $PSScriptRoot 'start.ps1') -Config 'config\proxy.direct.local.toml' | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'Gateway start.ps1 returned a failure' }
}

function Stop-ActiveGateway([string[]]$approvedExecutables) {
    & (Join-Path $PSScriptRoot 'stop.ps1') | Out-Null
    # If startup failed between spawning a process and writing its PID file,
    # a listener may still exist. Never touch the distinct desktop-local port.
    $listener = Get-NetTCPConnection -LocalPort 17777 -State Listen -ErrorAction SilentlyContinue |
        Select-Object -First 1
    if (-not $listener) { return }
    $candidate = Get-Process -Id $listener.OwningProcess -ErrorAction SilentlyContinue
    if (-not $candidate) { return }
    $path = $candidate.Path
    if (-not (@($approvedExecutables) | Where-Object {
        [string]::Equals(
            [IO.Path]::GetFullPath($_),
            [IO.Path]::GetFullPath($path),
            [StringComparison]::OrdinalIgnoreCase
        )
    })) {
        throw 'Gateway listener belongs to an unexpected executable. Refusing to stop it.'
    }
    Stop-Process -Id $candidate.Id -ErrorAction Stop
    $candidate.WaitForExit(5000) | Out-Null
}

New-Item -ItemType Directory -Path $runtime,$binDir -Force | Out-Null

try {
    if (!(Test-Path -LiteralPath $sourcePath -PathType Leaf)) { throw 'Staged release binary missing' }
    if (!(Test-Path -LiteralPath $config -PathType Leaf)) { throw 'Gateway config missing' }

    # Check that the management plane is independent, not a second Lomway
    # process that would disappear during a Gateway-only switch.
    $wb = Get-NetTCPConnection -LocalPort 7680 -State Listen -ErrorAction Stop |
        Select-Object -First 1
    $gatewayPort = Get-NetTCPConnection -LocalPort 17777 -State Listen -ErrorAction Stop |
        Select-Object -First 1
    $desktopPort = Get-NetTCPConnection -LocalPort 17778 -State Listen -ErrorAction Stop |
        Select-Object -First 1
    if ($wb.OwningProcess -in @($gatewayPort.OwningProcess,$desktopPort.OwningProcess)) {
        throw 'Workbridge is not independent of the Lomway processes'
    }
    if ((Get-HttpStatus 'http://127.0.0.1:7677/healthz') -ne 200) {
        throw 'OAuth must be healthy before cutover'
    }
    if ((Get-HttpStatus 'http://127.0.0.1:17777/healthz') -ne 200) {
        throw 'Gateway must be healthy before cutover'
    }
    if ((Get-HttpStatus 'https://mcp.maiteneru.com/.well-known/openid-configuration') -ne 200) {
        throw 'Public OAuth must be healthy before cutover'
    }
    $swibo = Invoke-RestMethod 'http://127.0.0.1:17991/api/status' -TimeoutSec 3
    $target = $swibo.targets | Where-Object id -eq 'lomway' | Select-Object -First 1
    if ($target.state -ne 'READY') { throw 'Swibo target is not READY' }

    $digest = (Get-FileHash -LiteralPath $sourcePath -Algorithm SHA256).Hash.ToLowerInvariant()
    $versioned = Join-Path $binDir ('lomway-gateway-' + $digest.Substring(0,16) + '.exe')
    if (!(Test-Path $versioned)) {
        Copy-Item -LiteralPath $sourcePath -Destination $versioned -ErrorAction Stop
    }
    if ((Get-FileHash -LiteralPath $versioned -Algorithm SHA256).Hash.ToLowerInvariant() -ne $digest) {
        throw 'Versioned executable digest mismatch'
    }
    & $versioned check --config $config | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'Versioned gateway config check failed' }

    $expectedOldExe = $oldDefault
    if ($hadPointer) {
        $oldPointer = (Get-Content -LiteralPath $pointer -Raw).Trim()
        . (Join-Path $PSScriptRoot 'common.ps1')
        $expectedOldExe = $Script:Exe
    }
    Record 'preflight_ok' ('stage_sha256=' + $digest.Substring(0,16) + '; management_independent=true')
    if ($PreflightOnly) {
        Write-Output 'PREFLIGHT_OK'
        exit 0
    }

    Record 'cutover_starting' 'Stopping ONLY Gateway process; OAuth and desktop local remain running'
    $cutoverStarted = $true
    Stop-ActiveGateway @($expectedOldExe, $versioned)
    $stopped = $true
    Set-ExePointer $versioned
    Invoke-GatewayStart
    if (!(Wait-Gateway $versioned)) { throw 'Versioned gateway failed health or process-path verification' }
    if ((Get-HttpStatus 'http://127.0.0.1:7677/healthz') -ne 200) {
        throw 'OAuth became unhealthy'
    }
    if ((Get-HttpStatus 'http://127.0.0.1:17778/healthz') -ne 200) {
        throw 'Desktop local became unhealthy'
    }
    if ((Get-HttpStatus 'https://mcp.maiteneru.com/.well-known/openid-configuration') -ne 200) {
        throw 'Public OAuth became unhealthy'
    }
    if ((Get-HttpStatus 'https://mcp.maiteneru.com/mcp') -ne 401) {
        throw 'Public unauthenticated MCP is not returning HTTP 401'
    }
    try { Invoke-RestMethod -Method Post 'http://127.0.0.1:17991/api/refresh' -TimeoutSec 10 | Out-Null } catch {}
    Record 'success' ('isolated_gateway=' + [IO.Path]::GetFileName($versioned))
    Write-Output 'GATEWAY_ISOLATED_DEPLOYMENT_OK'
} catch {
    $problem = $_.Exception.Message
    if (-not $cutoverStarted) {
        Record 'preflight_failed' $problem
        throw
    }
    Record 'rolling_back' $problem
    $stopFailure = ''
    try {
        Stop-ActiveGateway @($expectedOldExe, $versioned)
    } catch {
        $stopFailure = $_.Exception.Message
    }
    # Restore pointer and restart even when stop fails. Never attempt to
    # overwrite a potentially locked executable.
    $recovery = 'rollback_failed'
    try {
        if ($hadPointer) { Set-ExePointer $oldPointer } else { Set-ExePointer '' }
        Invoke-GatewayStart
        if (Wait-Gateway $expectedOldExe) { $recovery = 'rolled_back' }
    } catch {
        $problem += '; restart=' + $_.Exception.Message
    } finally {
        Record $recovery ($problem + '; stop=' + $stopFailure)
    }
    throw ('Gateway cutover failed: ' + $problem)
}
