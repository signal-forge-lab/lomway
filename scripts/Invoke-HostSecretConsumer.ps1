[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [ValidateSet('Status', 'Plan')]
    [string]$Action,
    [Parameter(Mandatory)]
    [ValidateSet('MacVncLogin')]
    [string]$Consumer
)

# Read-only host-secret consumer boundary. No arbitrary key, executable, host,
# environment variable name or user-supplied command can enter this interface.
# Secret extraction and credential delivery are deliberately NOT implemented.
# This file is safe to expose through coding-tools as a read-only preflight.
$ErrorActionPreference = 'Stop'
$profile = @{
    MacVncLogin = @{
        key = 'MAC_MINI_ADMIN_PASSWORD'
        executable = 'C:\Program Files\TigerVNC\vncviewer.exe'
        remote_address = '100.100.47.83'
        remote_port = 5900
        title = 'VNC authentication'
    }
}[$Consumer]

try {
    $store = Join-Path $env:USERPROFILE '.config\sops\secrets\global.sops.json'
    if (-not (Test-Path -LiteralPath $store -PathType Leaf)) {
        throw 'store-not-found'
    }
    # Top-level keys in the encrypted SOPS JSON are not secret values.
    $document = Get-Content -LiteralPath $store -Raw | ConvertFrom-Json
    $present = $null -ne $document.PSObject.Properties[$profile.key]
    $document = $null
    if ($Action -eq 'Status') {
        [pscustomobject]@{
            consumer = $Consumer
            key = $profile.key
            present = $present
            secret_exposed = $false
            credential_delivery = 'not-implemented'
        } | ConvertTo-Json -Compress
        return
    }
    $viewer = @()
    if ($present) {
        $viewer = @(Get-Process -Name vncviewer -ErrorAction SilentlyContinue |
            Where-Object {
                $_.Path -ceq $profile.executable -and
                $_.MainWindowTitle -ceq $profile.title
            })
    }
    $ready = $false
    if ($viewer.Count -eq 1) {
        $connection = @(Get-NetTCPConnection -OwningProcess $viewer[0].Id -State Established -ErrorAction SilentlyContinue |
            Where-Object {
                $_.RemoteAddress -ceq $profile.remote_address -and
                $_.RemotePort -eq $profile.remote_port
            })
        $ready = $connection.Count -eq 1
    }
    [pscustomobject]@{
        consumer = $Consumer
        key = $profile.key
        present = $present
        viewer_ready = $ready
        destination = '100.100.47.83:5900'
        secret_exposed = $false
        credential_delivery = 'not-implemented'
    } | ConvertTo-Json -Compress
} catch {
    # Never return file contents or exception text to an external caller.
    [Console]::Error.WriteLine('HOST_SECRET_CONSUMER_PREFLIGHT_FAILED')
    exit 1
}
