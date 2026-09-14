[CmdletBinding()]
param()

. (Join-Path $PSScriptRoot 'common.ps1')
$process = Get-GatewayProcess
if (-not $process) {
    Remove-Item -LiteralPath $Script:PidFile -Force -ErrorAction SilentlyContinue
    Write-Output 'NOT_RUNNING'
    exit 0
}

try {
    $actualPath = $process.Path
} catch {
    $actualPath = $null
}
if ($actualPath -and ([IO.Path]::GetFullPath($actualPath) -ne [IO.Path]::GetFullPath($Script:Exe))) {
    throw "Refusing to stop PID $($process.Id): executable does not match lomway.exe"
}

Stop-Process -Id $process.Id -ErrorAction Stop
$process.WaitForExit(5000) | Out-Null
Remove-Item -LiteralPath $Script:PidFile -Force -ErrorAction SilentlyContinue
Write-Output 'STOPPED'
