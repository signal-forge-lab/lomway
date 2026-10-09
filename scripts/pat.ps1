[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateSet('issue', 'list', 'revoke')]
    [string]$Action,
    [string]$Label,
    [ValidateRange(1, 90)]
    [int]$Days = 30,
    [string]$Tools = 'lomway_search_tools,lomway_describe_tool',
    [switch]$AllTools,
    [string]$Id
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$cli = Join-Path $root 'sidecar\dist\src\pat-cli.js'
if (-not (Test-Path -LiteralPath $cli)) {
    throw 'PAT CLI is not built. Run npm run build in sidecar first.'
}

switch ($Action) {
    'issue' {
        if ([string]::IsNullOrWhiteSpace($Label)) { throw '-Label is required.' }
        if ($AllTools -and $PSBoundParameters.ContainsKey('Tools')) {
            throw '-AllTools and -Tools cannot be used together.'
        }
        # Capture the credential inside this process, never echo to the transcript.
        if ($AllTools) {
            $raw = & node.exe $cli issue --label $Label --days $Days --all-tools
        } else {
            $raw = & node.exe $cli issue --label $Label --days $Days --tools $Tools
        }
        if ($LASTEXITCODE -ne 0) { throw 'PAT issue failed.' }
        $item = $raw | ConvertFrom-Json -ErrorAction Stop
        Set-Clipboard -Value ([string]$item.token)
        Write-Output ("ISSUED id={0} label={1} expires={2} all_tools={3} copied_to_clipboard=true" -f $item.id, $item.label, ([DateTimeOffset]::FromUnixTimeSeconds([long]$item.expiresAt).ToLocalTime().ToString('o')), $AllTools.IsPresent)
        $item = $null
        $raw = $null
    }
    'list' {
        & node.exe $cli list
        if ($LASTEXITCODE -ne 0) { throw 'PAT list failed.' }
    }
    'revoke' {
        if ([string]::IsNullOrWhiteSpace($Id)) { throw '-Id is required.' }
        & node.exe $cli revoke --id $Id
        if ($LASTEXITCODE -ne 0) { throw 'PAT revoke failed.' }
    }
}
