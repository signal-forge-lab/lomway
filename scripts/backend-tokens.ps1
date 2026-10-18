# Backend bearer tokens referenced by a gateway configuration.
#
# A public-schema configuration names each backend token by environment
# variable (`bearer_token_env = "LOMWAY_BACKEND_..."`); the gateway reads the
# value from its own environment and fails closed when it is missing. These
# helpers fill in variables that are not already set from the optional SOPS
# integration, so every launcher (start.ps1, the isolated deployment, a
# supervisor) starts the gateway the same way. Values are never printed,
# written to disk, or placed on a command line.

$Script:LomwayBackendTokenRoot = Split-Path -Parent $PSScriptRoot
$Script:LomwayBackendTokenPattern = '^LOMWAY_BACKEND_[A-Z0-9_]+$'

function Get-LomwayBackendTokenNames {
    param([Parameter(Mandatory = $true)][string]$ConfigPath)
    $text = Get-Content -LiteralPath $ConfigPath -Raw
    $names = foreach ($match in [regex]::Matches(
        $text,
        '(?m)^[ \t]*bearer_token_env[ \t]*=[ \t]*["'']([^"''\r\n]*)["'']'
    )) {
        $name = $match.Groups[1].Value
        # Only gateway backend tokens are ever imported, so a configuration
        # cannot pull an unrelated secret into the gateway environment.
        if ($name -cmatch $Script:LomwayBackendTokenPattern) { $name }
    }
    return @($names | Select-Object -Unique)
}

function Import-LomwayBackendTokens {
    param([Parameter(Mandatory = $true)][string]$ConfigPath)
    $missing = @(Get-LomwayBackendTokenNames -ConfigPath $ConfigPath | Where-Object {
        [string]::IsNullOrEmpty([Environment]::GetEnvironmentVariable($_, 'Process'))
    })
    if ($missing.Count -eq 0) { return @() }
    if (-not (Get-Command sops -ErrorAction SilentlyContinue)) {
        # The gateway reports the missing variable by name and refuses to start.
        return @()
    }
    $import = Join-Path $Script:LomwayBackendTokenRoot 'integrations\sops\Import-SopsSecrets.ps1'
    try {
        & $import -Names $missing | Out-Null
    } catch {
        Write-Warning ('Backend token import from SOPS failed: ' + $_.Exception.Message)
    }
    return @($missing | Where-Object {
        -not [string]::IsNullOrEmpty([Environment]::GetEnvironmentVariable($_, 'Process'))
    })
}

function Clear-LomwayBackendTokens {
    param([string[]]$Names)
    foreach ($name in @($Names)) {
        if ($name -cmatch $Script:LomwayBackendTokenPattern) {
            Remove-Item -LiteralPath ('Env:' + $name) -ErrorAction SilentlyContinue
        }
    }
}
