[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [ValidatePattern('^iw-sops-test-[a-f0-9]{16,48}$')]
    [string]$PipeName,
    [Parameter(Mandatory)]
    [string]$RequestJson
)

# Synthetic named-pipe protocol exerciser, without secret-value parameters.
$ErrorActionPreference = 'Stop'
$pipe = [IO.Pipes.NamedPipeClientStream]::new(
    '.', $PipeName, [IO.Pipes.PipeDirection]::InOut,
    [IO.Pipes.PipeOptions]::None
)
try {
    $pipe.Connect(5000)
    $writer = [IO.StreamWriter]::new($pipe, [Text.Encoding]::UTF8, 1024, $true)
    $reader = [IO.StreamReader]::new($pipe, [Text.Encoding]::UTF8, $false, 1024, $true)
    try {
        $writer.WriteLine($RequestJson)
        $writer.Flush()
        $response = $reader.ReadLine()
        if (-not $response -or $response.Length -gt 1024) {
            throw 'Malformed synthetic broker response'
        }
        $response
    } finally {
        $writer.Dispose()
        $reader.Dispose()
    }
} finally {
    $pipe.Dispose()
}
