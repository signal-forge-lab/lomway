[CmdletBinding()]
param(
    [ValidateRange(20,300)][int]$TimeoutSeconds = 90,
    [ValidateRange(5,60)][int]$ModelPreflightSeconds = 25
)

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
$preflight = $null
try {
    if (-not (Get-Command codex -ErrorAction SilentlyContinue)) {
        throw 'Codex CLI unavailable'
    }
    # Establish that the actual AI responds BEFORE minting any credential.
    # A logged-in CLI and reachable model endpoint do not guarantee the
    # model can complete a turn; in that case no PAT should be issued at all.
    $preflightStart = [System.Diagnostics.ProcessStartInfo]::new()
    $preflightStart.FileName = (Get-Command codex).Source
    $preflightStart.WorkingDirectory = $repo
    $preflightStart.UseShellExecute = $false
    $preflightStart.RedirectStandardOutput = $true
    $preflightStart.RedirectStandardError = $true
    $preflightStart.CreateNoWindow = $true
    foreach ($arg in @(
        'exec', '--ignore-user-config', '--ignore-rules', '--ephemeral',
        '--skip-git-repo-check', '--sandbox', 'read-only', '--json',
        'Reply exactly MODEL_PREFLIGHT_OK. Do not use tools.'
    )) { [void]$preflightStart.ArgumentList.Add($arg) }
    $preflight = [System.Diagnostics.Process]::Start($preflightStart)
    $preflightStdout = $preflight.StandardOutput.ReadToEndAsync()
    $preflightStderr = $preflight.StandardError.ReadToEndAsync()
    if (-not $preflight.WaitForExit($ModelPreflightSeconds * 1000)) {
        $preflight.Kill($true)
        $preflight.WaitForExit(5000) | Out-Null
        throw 'Codex basic model preflight timed out; no PAT was issued'
    }
    $preflightText = $preflightStdout.GetAwaiter().GetResult()
    $null = $preflightStderr.GetAwaiter().GetResult()
    $preflightConfirmed = $false
    foreach ($line in ($preflightText -split '\r?\n')) {
        try { $event = [string]$line | ConvertFrom-Json -Depth 12 } catch { continue }
        if ($event.type -eq 'item.completed' -and $event.item.type -eq 'agent_message' -and
            ([string]$event.item.text).Trim() -eq 'MODEL_PREFLIGHT_OK') {
            $preflightConfirmed = $true
        }
    }
    if ($preflight.ExitCode -ne 0 -or -not $preflightConfirmed) {
        throw 'Codex basic model preflight failed; no PAT was issued'
    }
    $preflightText = $null
    Write-Output 'CODEX_MODEL_PREFLIGHT_PASS'

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
    if ($preflight -and -not $preflight.HasExited) {
        $preflight.Kill($true)
        $preflight.WaitForExit(5000) | Out-Null
    }
    if ($preflight) { $preflight.Dispose() }
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
