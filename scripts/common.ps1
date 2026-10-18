$ErrorActionPreference = 'Stop'

$Script:RepoRoot = Split-Path -Parent $PSScriptRoot
$Script:RuntimeDir = Join-Path $Script:RepoRoot 'runtime'
$Script:LogDir = Join-Path $Script:RepoRoot 'logs'
$Script:PidFile = Join-Path $Script:RuntimeDir 'gateway.pid'
$Script:DefaultExe = Join-Path $Script:RepoRoot 'target\release\lomway.exe'
$Script:BinaryPointer = Join-Path $Script:RuntimeDir 'gateway-binary-path.txt'
$Script:Exe = $Script:DefaultExe
if (Test-Path -LiteralPath $Script:BinaryPointer -PathType Leaf) {
    $selected = (Get-Content -LiteralPath $Script:BinaryPointer -Raw).Trim()
    $approvedDir = [IO.Path]::GetFullPath((Join-Path $Script:RuntimeDir 'bin'))
    if (
        -not [IO.Path]::IsPathRooted($selected) -or
        -not [IO.Path]::GetFullPath($selected).StartsWith(
            ($approvedDir + [IO.Path]::DirectorySeparatorChar),
            [StringComparison]::OrdinalIgnoreCase
        ) -or
        [IO.Path]::GetExtension($selected) -ne '.exe'
    ) {
        throw 'Gateway binary pointer is outside runtime/bin or malformed.'
    }
    $Script:Exe = [IO.Path]::GetFullPath($selected)
}

function Get-GatewayPid {
    if (-not (Test-Path -LiteralPath $Script:PidFile -PathType Leaf)) { return $null }
    $raw = (Get-Content -LiteralPath $Script:PidFile -Raw).Trim()
    if ($raw -notmatch '^\d+$') { return $null }
    return [int]$raw
}

# Image path with PROCESS_QUERY_LIMITED_INFORMATION. Process.Path needs
# PROCESS_QUERY_INFORMATION, which a non-elevated caller does not get for a
# gateway that was started elevated; status/stop then reported "not running"
# (Swibo DEGRADED) while the gateway was serving (2026-10-19).
function Get-ProcessImagePath([int]$ProcessId) {
    if (-not ('LomwayOps.ProcessImage' -as [type])) {
        Add-Type -Namespace LomwayOps -Name ProcessImage -MemberDefinition @'
[DllImport("kernel32.dll", SetLastError=true)] public static extern IntPtr OpenProcess(uint access, bool inherit, int pid);
[DllImport("kernel32.dll", SetLastError=true, CharSet=CharSet.Unicode)] public static extern bool QueryFullProcessImageName(IntPtr h, int flags, System.Text.StringBuilder name, ref int size);
[DllImport("kernel32.dll")] public static extern bool CloseHandle(IntPtr h);
'@
    }
    $handle = [LomwayOps.ProcessImage]::OpenProcess(0x1000, $false, $ProcessId)
    if ($handle -eq [IntPtr]::Zero) { return $null }
    try {
        $buffer = New-Object System.Text.StringBuilder 1024
        $size = 1024
        if ([LomwayOps.ProcessImage]::QueryFullProcessImageName($handle, 0, $buffer, [ref]$size)) { return $buffer.ToString() }
        return $null
    } finally {
        [void][LomwayOps.ProcessImage]::CloseHandle($handle)
    }
}

function Get-GatewayProcess {
    $pidValue = Get-GatewayPid
    if (-not $pidValue) { return $null }
    $process = Get-Process -Id $pidValue -ErrorAction SilentlyContinue
    if (-not $process) { return $null }
    $actualPath = $null
    try {
        $actualPath = $process.Path
    } catch {}
    if (-not $actualPath) { $actualPath = Get-ProcessImagePath $process.Id }
    if (-not $actualPath) { return $null }
    if ([IO.Path]::GetFullPath($actualPath) -ne [IO.Path]::GetFullPath($Script:Exe)) { return $null }
    return $process
}

function Assert-GatewayBinary {
    if (-not (Test-Path -LiteralPath $Script:Exe -PathType Leaf)) {
        throw "Gateway binary is missing. Run scripts\install.ps1 first: $Script:Exe"
    }
}
