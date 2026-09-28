#!/usr/bin/env -S powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File
$ErrorActionPreference = 'Stop'
$env:PSModulePath = "$PSHOME/Modules"
$root = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$staged = Join-Path $root ('bin/.herdr-grid-slide.' + [guid]::NewGuid().ToString('N'))

Push-Location -LiteralPath $root
try {
    & cargo build --locked --release
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    [System.IO.Directory]::CreateDirectory((Join-Path $root 'bin')) | Out-Null
    [System.IO.File]::Copy((Join-Path $root 'target/release/herdr-grid-slide.exe'), $staged)
    $destination = Join-Path $root 'bin/herdr-grid-slide.exe'
    if ([System.IO.File]::Exists($destination)) {
        [System.IO.File]::Replace($staged, $destination, [NullString]::Value)
    } else {
        [System.IO.File]::Move($staged, $destination)
    }

    & herdr plugin link .
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    & herdr server reload-config
    exit $LASTEXITCODE
} finally {
    if (Test-Path -LiteralPath $staged) { Remove-Item -LiteralPath $staged -Force }
    Pop-Location
}
