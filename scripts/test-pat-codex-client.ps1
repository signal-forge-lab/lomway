[CmdletBinding()]
param([ValidateRange(20,300)][int]$TimeoutSeconds = 90)

# End-to-end external AI test with disposable single-tool PAT.
# Credentials exist only in a child-process environment variable.
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$cli = Join-Path $repo 'sidecar\dist\src\pat-cli.js'
$token = $null
$tokenId = $null
$raw = $null
$allowed = 'praxiom_praxiom_status'
$child = $null
try {
    if (-not (Get-Command codex -ErrorAction SilentlyContinue)) {
        throw 'Codex CLI unavailable'
    }
    $issued = & node.exe $cli issue --label 'disposable-codex-mcp-probe' --days 1 --tools $allowed
    if ($LASTEXITCODE -ne 0) { throw 'PAT issuance failed' }
    $parsed = $issued | ConvertFrom-Json
    $token = [string]$parsed.token
    $tokenId = [string]$parsed.id
    $issued = $null
    $parsed = $null
    if (-not $token -or -not $tokenId) { throw 'PAT issuance malformed' }

    $configuration = 'mcp_servers.lomway_pat_probe={url="https://mcp.maiteneru.com/mcp",bearer_token_env_var="LOMWAY_PAT_PROBE_TOKEN",enabled=true,startup_timeout_sec=20,tool_timeout_sec=25}'
    $prompt = 'Use the MCP tool lomway_pat_probe.praxiom_praxiom_status exactly once. Do not run a shell or inspect files. If the tool succeeds, reply only MCP_PROBE_OK; otherwise MCP_PROBE_FAILED. Never request or output credentials.'
    $arguments = @(
        'exec', '--ignore-user-config', '--ignore-rules', '--ephemeral',
        '--skip-git-repo-check', '--sandbox', 'read-only', '--json',
        '-c', $configuration, $prompt
    )
    $start = [System.Diagnostics.ProcessStartInfo]::new()
    $start.FileName = (Get-Command codex).Source
    $start.WorkingDirectory = $repo
    $start.UseShellExecute = $false
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $start.CreateNoWindow = $true
    $start.Environment['LOMWAY_PAT_PROBE_TOKEN'] = $token
    foreach ($argument in $arguments) {
        $null = $start.ArgumentList.Add([string]$argument)
    }
    $child = [System.Diagnostics.Process]::Start($start)
    # Drain output concurrently so a full pipe cannot deadlock the child.
    $stdout = $child.StandardOutput.ReadToEndAsync()
    $stderr = $child.StandardError.ReadToEndAsync()
    if (-not $child.WaitForExit($TimeoutSeconds * 1000)) {
        $child.Kill($true)
        $child.WaitForExit(5000) | Out-Null
        throw 'Codex model did not respond within the bounded test deadline'
    }
    $exitCode = $child.ExitCode
    $raw = @($stdout.GetAwaiter().GetResult() -split '\r?\n')
    # stderr is discarded: it can include untrusted content and MCP details.
    $null = $stderr.GetAwaiter().GetResult()

    $toolCalls = @()
    $confirmed = $false
    foreach ($line in $raw) {
        try { $event = ([string]$line | ConvertFrom-Json -Depth 30) } catch { continue }
        if ($event.type -in @('item.started','item.completed') -and
            $event.item.type -eq 'mcp_tool_call' -and
            $event.item.server -eq 'lomway_pat_probe') {
            $toolCalls += $event.item
        }
        if ($event.type -eq 'item.completed' -and $event.item.type -eq 'agent_message' -and
            [string]$event.item.text -match 'MCP_PROBE_OK') {
            $confirmed = $true
        }
    }
    $completed = @($toolCalls | Where-Object {
        $_.tool -eq $allowed -and $_.status -eq 'completed' -and !$_.error
    })
    if ($exitCode -ne 0 -or -not $confirmed -or $completed.Count -lt 1) {
        Write-Output ('CODEX_MCP_PROBE_FAILED exit=' + $exitCode +
            ' calls=' + $toolCalls.Count + ' allowed_completed=' + $completed.Count)
        throw 'Independent AI failed MCP verification'
    }
    Write-Output 'CODEX_REAL_AI_MCP_TOOL_CALL_PASS'
} finally {
    if ($child -and -not $child.HasExited) {
        $child.Kill($true)
        $child.WaitForExit(5000) | Out-Null
    }
    if ($child) { $child.Dispose() }
    $token = $null
    $raw = $null
    if ($tokenId) {
        $null = & node.exe $cli revoke --id $tokenId
        if ($LASTEXITCODE -ne 0) { throw 'CRITICAL: temporary PAT revocation failed' }
        Write-Output 'CODEX_DISPOSABLE_PAT_REVOKED'
    }
}
