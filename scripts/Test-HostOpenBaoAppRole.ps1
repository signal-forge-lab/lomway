# Fixed, local-only OpenBao AppRole E2E consumer for Invoke-HostSops.ps1.
# Do not invoke via arbitrary caller-supplied commands or return secret values.
[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$base = 'http://127.0.0.1:8200/v1'
$store = Join-Path $env:USERPROFILE '.config\sops\secrets\global.sops.json'
$token = $null
$headers = $null
$phase = 'preflight'
$failed = $false

function Read-FixedSecret([string]$name) {
    $selector = '["' + $name + '"]'
    $valueText = (& sops --decrypt --extract $selector $store 2>$null) -join ''
    if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($valueText)) {
        throw 'SOPS_EXTRACT_FAILED'
    }
    # SOPS scalar extraction may emit a raw string rather than JSON quoting.
    # Match the existing Invoke-HostSops.ps1 scalar handling.
    try { return [string]($valueText | ConvertFrom-Json -ErrorAction Stop) }
    catch { return [string]$valueText.Trim('"') }
}

try {
    $phase = 'read-sops'
    $roleId = Read-FixedSecret 'OPENBAO_WINDOWS_AI_RUNTIME_ROLE_ID'
    $secretId = Read-FixedSecret 'OPENBAO_WINDOWS_AI_RUNTIME_SECRET_ID'
    if ([string]::IsNullOrWhiteSpace($roleId) -or
        [string]::IsNullOrWhiteSpace($secretId)) {
        throw 'Missing AppRole credentials'
    }
    Write-Output 'SOPS_CREDENTIALS=PASS'

    $phase = 'approle-login'
    $request = @{ role_id = $roleId; secret_id = $secretId } | ConvertTo-Json -Compress
    $login = Invoke-RestMethod -Uri "$base/auth/approle/login" -Method Post `
        -ContentType 'application/json' -Body $request
    $request = $null
    $token = [string]$login.auth.client_token
    if (-not $token) { throw 'AppRole login returned no token' }
    Write-Output 'APPROLE_LOGIN=PASS'
    if (@($login.auth.policies) -cnotcontains 'read-api-ai-openai') {
        throw 'Expected policy was not assigned'
    }
    Write-Output 'POLICY_ASSIGNED=PASS'

    $headers = @{ 'X-Vault-Token' = $token }
    $phase = 'kv-read'
    $result = Invoke-RestMethod -Uri "$base/kv/data/api/ai/openai" -Headers $headers -Method Get
    $keys = @($result.data.data.PSObject.Properties.Name)
    if ($keys -cnotcontains 'free' -or $keys -cnotcontains 'paid' -or
        [string]::IsNullOrWhiteSpace([string]$result.data.data.free) -or
        [string]::IsNullOrWhiteSpace([string]$result.data.data.paid)) {
        throw 'Expected free/paid fields missing'
    }
    Write-Output 'KV_FREE_AND_PAID_READ=PASS'
    $result = $null

    $phase = 'capabilities'
    $paths = @('kv/data/api/ai/openai','kv/data/api/ai/other-provider')
    $caps = Invoke-RestMethod -Uri "$base/sys/capabilities-self" -Headers $headers `
        -Method Post -ContentType 'application/json' `
        -Body (@{paths=$paths} | ConvertTo-Json -Compress)
    $data = if ($caps.data) { $caps.data } else { $caps }
    $allowed = @($data.'kv/data/api/ai/openai')
    $other = @($data.'kv/data/api/ai/other-provider')
    $mutations = @('create','update','patch','delete','sudo')
    if ($allowed -cnotcontains 'read' -or
        @($mutations | Where-Object { $allowed -ccontains $_ }).Count -ne 0 -or
        $other -cnotcontains 'deny') {
        throw 'Unexpected token capabilities'
    }
    Write-Output 'READ_ONLY_AND_OTHER_DENIED=PASS'
}
catch {
    $failed = $true
    # Never echo web response bodies, exceptions, credentials or secret data.
    Write-Output ('FAILED_PHASE=' + $phase)
    if ($_.Exception.Message -in @('SOPS_EXTRACT_FAILED', 'SOPS_JSON_SCALAR_FAILED')) {
        Write-Output ('FAILED_REASON=' + $_.Exception.Message)
    }
}
finally {
    if ($token) {
        try {
            if (-not $headers) { $headers = @{ 'X-Vault-Token' = $token } }
            Invoke-RestMethod -Uri "$base/auth/token/revoke-self" -Headers $headers `
                -Method Post | Out-Null
            Write-Output 'TEST_TOKEN_REVOKED=PASS'
        } catch {
            Write-Output 'TEST_TOKEN_REVOKED=FAIL'
            $failed = $true
        }
    }
    $token = $null
    $roleId = $null
    $secretId = $null
    $login = $null
    $headers = $null
    $result = $null
}
if ($failed) { Write-Output 'E2E=FAIL'; exit 1 }
Write-Output 'E2E=PASS'
exit 0
