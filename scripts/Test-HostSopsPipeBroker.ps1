[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [ValidatePattern('^iw-sops-test-[a-f0-9]{16,48}$')]
    [string]$PipeName,
    [Parameter(Mandatory)]
    [ValidatePattern('^S-1-5-[0-9-]+$')]
    [string]$ClientSid,
    [ValidateRange(1, 8)]
    [int]$MaxRequests = 1,
    [switch]$SyntheticOnly
)

# TEST-ONLY harness. It does NOT isolate secrets while it runs as the same
# Windows user as the caller. Never start it against production SOPS/age.
$ErrorActionPreference = 'Stop'
if (-not $SyntheticOnly -or $env:HOST_SOPS_SYNTHETIC_TEST -cne '1' -or
    -not $env:SOPS_AGE_KEY_FILE -or
    -not (Test-Path -LiteralPath $env:SOPS_AGE_KEY_FILE -PathType Leaf)) {
    throw 'Synthetic-only broker requires an isolated SOPS/age test context'
}
$actualProfile = [Environment]::GetFolderPath(
    [Environment+SpecialFolder]::UserProfile
)
if (-not $env:USERPROFILE -or
    [IO.Path]::GetFullPath($env:USERPROFILE).TrimEnd('\') -ieq
        [IO.Path]::GetFullPath($actualProfile).TrimEnd('\')) {
    throw 'Synthetic-only broker refuses the real Windows profile'
}

$me = [Security.Principal.WindowsIdentity]::GetCurrent().User
$client = [Security.Principal.SecurityIdentifier]::new($ClientSid)
$acl = [System.IO.Pipes.PipeSecurity]::new()
$acl.SetAccessRuleProtection($true, $false)
$acl.AddAccessRule([System.IO.Pipes.PipeAccessRule]::new(
    $me, [System.IO.Pipes.PipeAccessRights]::FullControl,
    [Security.AccessControl.AccessControlType]::Allow
))
if ($me.Value -cne $client.Value) {
    $acl.AddAccessRule([System.IO.Pipes.PipeAccessRule]::new(
        $client, [System.IO.Pipes.PipeAccessRights]::ReadWrite,
        [Security.AccessControl.AccessControlType]::Allow
    ))
}

$manager = Join-Path $PSScriptRoot 'Invoke-HostSops.ps1'
for ($n = 0; $n -lt $MaxRequests; $n++) {
    $pipe = [System.IO.Pipes.NamedPipeServerStreamAcl]::Create(
        $PipeName, [System.IO.Pipes.PipeDirection]::InOut, 1,
        [System.IO.Pipes.PipeTransmissionMode]::Byte,
        [System.IO.Pipes.PipeOptions]::Asynchronous, 4096, 4096, $acl,
        [System.IO.HandleInheritability]::None,
        ([System.IO.Pipes.PipeAccessRights]0)
    )
    try {
        $pipe.WaitForConnection()
        $deadline = [Threading.CancellationTokenSource]::new(5000)
        $writer = [IO.StreamWriter]::new($pipe, [Text.Encoding]::UTF8, 1024, $true)
        try {
            $line = [Collections.Generic.List[byte]]::new()
            $complete = $false
            $buffer = [byte[]]::new(1)
            for ($i = 0; $i -lt 2048; $i++) {
                $count = $pipe.ReadAsync($buffer, 0, 1, $deadline.Token).GetAwaiter().GetResult()
                if ($count -eq 0) { break }
                if ($buffer[0] -eq 10) { $complete = $true; break }
                if ($buffer[0] -ne 13) { $line.Add($buffer[0]) }
            }
            $response = [ordered]@{ ok = $false; code = 'DENIED' }
            if ($complete -and $line.Length -le 2048) {
                try {
                    $request = [Text.Encoding]::UTF8.GetString($line.ToArray()).TrimStart([char]0xFEFF) | ConvertFrom-Json -ErrorAction Stop
                    $keys = @($request.PSObject.Properties.Name)
                    if ($request.version -ne 1 -or $request.action -cne 'Status' -or
                        $request.provider -cne 'RandomHex' -or
                        $request.key -cnotmatch '^[A-Z][A-Z0-9_]{1,127}$' -or
                        $keys.Count -ne 4 -or
                        @($keys | Where-Object { $_ -cnotin @('version', 'action', 'provider', 'key') }).Count) {
                        throw 'Unsupported broker request'
                    }
                    $result = & pwsh.exe -NoProfile -NonInteractive -File $manager -Action Status -Key ([string]$request.key) -Provider RandomHex 2>$null
                    if ($LASTEXITCODE -ne 0) { throw 'Status provider unavailable' }
                    $parsed = ($result | Out-String) | ConvertFrom-Json -ErrorAction Stop
                    $response = [ordered]@{ ok = $true; present = [bool]$parsed.present }
                } catch {
                    # Never return provider stderr, exception text or secret data.
                    $response = [ordered]@{ ok = $false; code = 'DENIED' }
                }
            }
            $writer.WriteLine(($response | ConvertTo-Json -Compress))
            $writer.Flush()
        } finally {
            $deadline.Dispose()
            $writer.Dispose()
        }
    } finally {
        $pipe.Dispose()
    }
}
