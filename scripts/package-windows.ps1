param(
    [ValidateSet('x86_64-pc-windows-msvc')]
    [string]$Target = 'x86_64-pc-windows-msvc',
    [switch]$SkipBuild
)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
Push-Location $projectRoot
$stage = $null
try {
    if (-not $SkipBuild) {
        & cargo build --release --locked --target $Target
        if ($LASTEXITCODE -ne 0) { throw 'Release build failed.' }
    }
    $binary = & python scripts/release.py binary-path --target $Target
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $binary)) { throw 'Release executable is missing.' }
    $asset = & python scripts/release.py asset-name --target $Target
    if ($LASTEXITCODE -ne 0) { throw 'Could not determine package name.' }
    $dist = Join-Path $projectRoot 'dist'
    $releaseDir = Join-Path $dist 'release'
    New-Item -ItemType Directory -Force -Path $releaseDir | Out-Null
    $stage = Join-Path $dist ('stage-' + [guid]::NewGuid().ToString('N'))
    & python scripts/release.py stage --target $Target --directory $stage
    if ($LASTEXITCODE -ne 0) { throw 'License and SBOM staging failed.' }
    Copy-Item -LiteralPath $binary -Destination (Join-Path $stage 'Vyber.exe')
    Compress-Archive -Path (Join-Path $stage '*') -DestinationPath (Join-Path $releaseDir $asset) -Force
    & python scripts/release.py record --target $Target
    if ($LASTEXITCODE -ne 0) { throw 'Package metadata failed.' }
    Write-Host "Packaged: dist/release/$asset"
} finally {
    if ($stage -and (Test-Path -LiteralPath $stage)) {
        $resolvedStage = [IO.Path]::GetFullPath($stage)
        $allowedRoot = [IO.Path]::GetFullPath((Join-Path $projectRoot 'dist')) + [IO.Path]::DirectorySeparatorChar
        if (-not $resolvedStage.StartsWith($allowedRoot, [StringComparison]::OrdinalIgnoreCase)) { throw 'Unsafe package cleanup path.' }
        Remove-Item -LiteralPath $resolvedStage -Recurse -Force
    }
    Pop-Location
}
