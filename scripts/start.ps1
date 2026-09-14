[CmdletBinding()]
param(
    [string]$Config = 'config\proxy.local.toml'
)

. (Join-Path $PSScriptRoot 'common.ps1')
Assert-GatewayBinary

$existing = Get-GatewayProcess
if ($existing) {
    Write-Output ("ALREADY_RUNNING pid={0}" -f $existing.Id)
    exit 0
}

$configPath = if ([IO.Path]::IsPathRooted($Config)) { $Config } else { Join-Path $Script:RepoRoot $Config }
if (-not (Test-Path -LiteralPath $configPath -PathType Leaf)) {
    throw "Local gateway config not found: $configPath"
}

New-Item -ItemType Directory -Force -Path $Script:RuntimeDir, $Script:LogDir | Out-Null
$stdout = Join-Path $Script:LogDir 'gateway.stdout.log'
$stderr = Join-Path $Script:LogDir 'gateway.stderr.log'
$process = Start-Process -FilePath $Script:Exe `
    -ArgumentList @('serve', '--config', ('"{0}"' -f $configPath)) `
    -WorkingDirectory $Script:RepoRoot `
    -WindowStyle Hidden `
    -RedirectStandardOutput $stdout `
    -RedirectStandardError $stderr `
    -PassThru
Set-Content -LiteralPath $Script:PidFile -Value $process.Id -NoNewline

for ($i = 0; $i -lt 40; $i++) {
    Start-Sleep -Milliseconds 250
    if ($process.HasExited) {
        Remove-Item -LiteralPath $Script:PidFile -Force -ErrorAction SilentlyContinue
        throw "Gateway exited during startup with code $($process.ExitCode). See logs\gateway.stderr.log."
    }
    try {
        $response = Invoke-WebRequest -UseBasicParsing -Uri 'http://127.0.0.1:17777/healthz' -TimeoutSec 1
        if ($response.StatusCode -eq 200) {
            Write-Output ("STARTED pid={0}" -f $process.Id)
            exit 0
        }
    } catch {}
}

throw 'Gateway did not become ready within 10 seconds. See logs\gateway.stderr.log.'
