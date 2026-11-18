#requires -Version 7.0
param(
    [string]$OutputPath = (Join-Path $PSScriptRoot 'release/Pigeon-Windows-x64.exe')
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if (-not $IsWindows) { throw 'Run this build on Windows x64.' }
$destination = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($OutputPath)
$target = if ($env:CARGO_TARGET_DIR) {
    $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($env:CARGO_TARGET_DIR)
} elseif ($env:DEVTOOLS_HOME) {
    Join-Path $env:DEVTOOLS_HOME 'build-cache/pigeon-standalone'
} else {
    Join-Path $PSScriptRoot 'target'
}
$assets = Join-Path $PSScriptRoot 'assets-2d'
$manifest = Join-Path $assets 'animation-validation.json'
if (-not (Test-Path -LiteralPath $manifest -PathType Leaf)) {
    throw 'Generate the animation frames first with python tools/animate_2d.py.'
}
$clips = (Get-Content -LiteralPath $manifest -Raw | ConvertFrom-Json -AsHashtable).clips
$metadata = & cargo metadata --manifest-path (Join-Path $PSScriptRoot 'Cargo.toml') --locked --no-deps --format-version 1 | ConvertFrom-Json
if ($LASTEXITCODE -ne 0) { throw 'Could not read the package version.' }
$versionBytes = [Text.Encoding]::ASCII.GetBytes(($metadata.packages | Where-Object name -eq 'pigeon').version)
& cargo build --manifest-path (Join-Path $PSScriptRoot 'Cargo.toml') --release --locked --features standalone --target-dir $target
if ($LASTEXITCODE -ne 0) { throw 'Release build failed.' }
$binary = Join-Path $target 'release/pigeon.exe'
if ([IO.Path]::GetFullPath($binary) -eq $destination) { throw 'Choose a destination outside the Cargo executable.' }

$directory = [IO.Path]::GetDirectoryName($destination)
[IO.Directory]::CreateDirectory($directory) | Out-Null
$stage = Join-Path $directory ('.pigeon-' + [guid]::NewGuid().ToString('N') + '.exe')
try {
    [IO.File]::Copy($binary, $stage)
    $stream = [IO.File]::Open($stage, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
    try {
        $reader = [IO.BinaryReader]::new($stream, [Text.Encoding]::UTF8, $true)
        try {
            $stream.Position = 0x3c
            $stream.Position = $reader.ReadUInt32()
            if ($reader.ReadUInt32() -ne 0x4550 -or $reader.ReadUInt16() -ne 0x8664) {
                throw 'Expected a Windows x64 executable.'
            }
        } finally { $reader.Dispose() }
        $start = $stream.Seek(0, [IO.SeekOrigin]::End)
        $offsets = [Collections.Generic.List[uint32]]::new()
        $offsets.Add(0)
        foreach ($clip in 'walk', 'stalk', 'settle', 'turn', 'look', 'peck') {
            $count = [int]$clips[$clip].frames
            if ($count -le 0) { throw "Missing animation: $clip" }
            for ($i = 0; $i -lt $count; $i++) {
                $path = Join-Path $assets ('{0}/{1:D4}.png' -f $clip, $i)
                $frame = [IO.File]::OpenRead($path)
                try {
                    if ($frame.Length -lt 1 -or $frame.Length -gt 1MB) {
                        throw "Invalid frame size: $path"
                    }
                    $frame.CopyTo($stream)
                } finally { $frame.Dispose() }
                $offsets.Add([uint32]($stream.Position - $start))
            }
        }
        $writer = [IO.BinaryWriter]::new($stream, [Text.Encoding]::UTF8, $true)
        try {
            foreach ($offset in $offsets) { $writer.Write($offset) }
            $writer.Write($versionBytes)
            $writer.Write([uint32]$versionBytes.Length)
            $writer.Write([uint32]($offsets.Count - 1))
            $writer.Write([uint64]$start)
            $writer.Write([Text.Encoding]::ASCII.GetBytes('PIGNAPK2'))
        } finally { $writer.Dispose() }
    } finally { $stream.Dispose() }
    $verify = Start-Process -FilePath $stage -ArgumentList '--verify-assets' -WorkingDirectory $directory -WindowStyle Hidden -Wait -PassThru
    if ($verify.ExitCode -ne 0) { throw "Embedded animation verification failed (exit $($verify.ExitCode))." }
    [IO.File]::Move($stage, $destination, $true)
    Write-Host "Verified $($offsets.Count - 1) frames in $destination ($((Get-Item -LiteralPath $destination).Length) bytes)."
} finally {
    if ([IO.File]::Exists($stage)) { [IO.File]::Delete($stage) }
}
