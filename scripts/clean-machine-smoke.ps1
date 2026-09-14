[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$Repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$Suffix = if ($IsWindows) { '.exe' } else { '' }
$Temp = Join-Path ([IO.Path]::GetTempPath()) ("lomway-clean-smoke-{0}-{1}" -f $PID, [guid]::NewGuid().ToString('N'))
$BuildRoot = Join-Path $Temp 'cargo-target'
$Mock = $null
$Gateway = $null
$PreviousCargoTarget = $env:CARGO_TARGET_DIR

function Wait-HttpOk([string]$Url, [int]$Seconds = 15) {
    $deadline = [DateTime]::UtcNow.AddSeconds($Seconds)
    while ([DateTime]::UtcNow -lt $deadline) {
        try {
            $response = Invoke-WebRequest -UseBasicParsing -Uri $Url -TimeoutSec 2
            if ($response.StatusCode -eq 200) { return }
        } catch {}
        Start-Sleep -Milliseconds 200
    }
    throw "Timed out waiting for $Url"
}

try {
    New-Item -ItemType Directory -Force -Path $Temp | Out-Null
    $env:CARGO_TARGET_DIR = $BuildRoot
    Push-Location $Repo
    try {
        cargo build --locked --release --bin lomway --example public_mock_backend
        if ($LASTEXITCODE -ne 0) { throw 'release artifact build failed' }
    } finally {
        Pop-Location
    }

    $LomwayPath = Join-Path $Temp "lomway$Suffix"
    $MockPath = Join-Path $Temp "public_mock_backend$Suffix"
    $ConfigPath = Join-Path $Temp 'one-backend.toml'
    Copy-Item (Join-Path $BuildRoot "release/lomway$Suffix") $LomwayPath
    Copy-Item (Join-Path $BuildRoot "release/examples/public_mock_backend$Suffix") $MockPath
    Copy-Item (Join-Path $Repo 'test-fixtures/configs/one-backend.toml') $ConfigPath

    $env:LOMWAY_SMOKE_BACKEND_ADDR = '127.0.0.1:18701'
    $Mock = Start-Process -FilePath $MockPath -WorkingDirectory $Temp -PassThru -WindowStyle Hidden
    Start-Sleep -Milliseconds 500
    if ($Mock.HasExited) { throw 'public mock backend exited during startup' }

    & $LomwayPath check --config $ConfigPath | Out-Host
    if ($LASTEXITCODE -ne 0) { throw 'copied release binary failed check with copied public fixture' }

    $Gateway = Start-Process -FilePath $LomwayPath -ArgumentList @('serve', '--config', $ConfigPath) `
        -WorkingDirectory $Temp -PassThru -WindowStyle Hidden
    Wait-HttpOk 'http://127.0.0.1:18791/healthz'
    Wait-HttpOk 'http://127.0.0.1:18791/readyz'
    if ($Gateway.HasExited) { throw 'copied release binary exited before readiness verification' }

    Write-Output 'clean-machine artifact smoke: PASS (copied release binary + copied public fixture; no private deployment backends)'
} finally {
    if ($Gateway -and -not $Gateway.HasExited) { Stop-Process -Id $Gateway.Id -Force -ErrorAction SilentlyContinue }
    if ($Mock -and -not $Mock.HasExited) { Stop-Process -Id $Mock.Id -Force -ErrorAction SilentlyContinue }
    Remove-Item Env:LOMWAY_SMOKE_BACKEND_ADDR -ErrorAction SilentlyContinue
    if ($null -eq $PreviousCargoTarget) {
        Remove-Item Env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue
    } else {
        $env:CARGO_TARGET_DIR = $PreviousCargoTarget
    }
    Remove-Item $Temp -Recurse -Force -ErrorAction SilentlyContinue
}
