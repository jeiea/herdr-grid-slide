$ErrorActionPreference = 'Stop'
# Herdr can inherit PowerShell 7 module paths; use this process's built-in modules.
# https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.core/about/about_psmodulepath#starting-windows-powershell-from-powershell-7
$env:PSModulePath = "$PSHOME/Modules"
$root = Split-Path $PSScriptRoot -Parent
$temporary = $null
$exitCode = 1

try {
    $architecture = $env:PROCESSOR_ARCHITEW6432
    if (-not $architecture) { $architecture = $env:PROCESSOR_ARCHITECTURE }
    if ($architecture -ne 'AMD64') {
        throw "unsupported architecture: $architecture"
    }

    $manifest = Get-Content -LiteralPath (Join-Path $root 'herdr-plugin.toml') -Raw
    $version = [regex]::Match($manifest, '(?m)^version\s*=\s*"([^"]+)"').Groups[1].Value
    if (-not $version) { throw 'could not read version from herdr-plugin.toml' }

    $asset = "herdr-grid-slide-v$version-x86_64-pc-windows-msvc.exe"
    $baseUrl = $env:HERDR_GRID_SLIDE_RELEASE_BASE_URL
    $curlOptions = @('--fail', '--location', '--silent', '--show-error')
    if (-not $baseUrl) {
        $baseUrl = "https://github.com/jeiea/herdr-grid-slide/releases/download/v$version"
        $curlOptions += @('--proto', '=https', '--tlsv1.2')
    }

    $bin = Join-Path $root 'bin'
    [System.IO.Directory]::CreateDirectory($bin) | Out-Null
    $temporary = Join-Path $bin ('.herdr-grid-slide.' + [guid]::NewGuid().ToString('N'))
    [System.IO.Directory]::CreateDirectory($temporary) | Out-Null
    foreach ($file in @('SHA256SUMS', $asset)) {
        & curl.exe @curlOptions "$baseUrl/$file" --output (Join-Path $temporary $file)
        if ($LASTEXITCODE -ne 0) {
            $exitCode = $LASTEXITCODE
            throw "download failed: $file (curl exit $exitCode)"
        }
    }

    $entries = @(Get-Content -LiteralPath (Join-Path $temporary 'SHA256SUMS') | Where-Object {
        $fields = $_.Trim() -split '\s+'
        $fields.Count -ge 2 -and ($fields[1] -ceq $asset -or $fields[1] -ceq "*$asset")
    })
    if ($entries.Count -ne 1) {
        throw "SHA256SUMS must contain exactly one valid entry for $asset"
    }
    $fields = $entries[0].Trim() -split '\s+'
    if ($fields.Count -ne 2 -or $fields[0] -notmatch '^[0-9a-fA-F]{64}$') {
        throw "SHA256SUMS must contain exactly one valid entry for $asset"
    }
    $download = Join-Path $temporary $asset
    if ((Get-FileHash -LiteralPath $download -Algorithm SHA256).Hash -ne $fields[0]) {
        throw "checksum mismatch for $asset"
    }

    $destination = Join-Path $bin 'herdr-grid-slide.exe'
    if ([System.IO.File]::Exists($destination)) {
        [System.IO.File]::Replace($download, $destination, [NullString]::Value)
    } else {
        [System.IO.File]::Move($download, $destination)
    }
    $exitCode = 0
} catch {
    [Console]::Error.WriteLine($_.Exception.Message)
} finally {
    if ($temporary) { Remove-Item -LiteralPath $temporary -Recurse -Force }
}
exit $exitCode
