param([switch]$Release)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
Push-Location $projectRoot
try {
    $cargoArgs = @('build', '--locked', '--jobs', '2')
    if ($Release) { $cargoArgs += '--release' }
    & cargo @cargoArgs
    if ($LASTEXITCODE -ne 0) { throw 'Vyber build failed.' }
    $profile = if ($Release) { 'release' } else { 'debug' }
    New-Item -ItemType Directory -Path (Join-Path $projectRoot 'dist') -Force | Out-Null
    Copy-Item -LiteralPath (Join-Path $projectRoot "target\$profile\vyber.exe") -Destination (Join-Path $projectRoot 'dist\Vyber.exe') -Force
    Copy-Item -LiteralPath (Join-Path $projectRoot 'assets\vyber.ico') -Destination (Join-Path $projectRoot 'dist\vyber.ico') -Force
    $shortcut = (New-Object -ComObject WScript.Shell).CreateShortcut((Join-Path $projectRoot 'Vyber.lnk'))
    $shortcut.TargetPath = Join-Path $projectRoot 'dist\Vyber.exe'
    $shortcut.WorkingDirectory = $projectRoot
    $shortcut.IconLocation = Join-Path $projectRoot 'dist\vyber.ico'
    $shortcut.Description = 'Vyber native terminal workspace'
    $shortcut.Save()
    Write-Host 'Ready: dist\Vyber.exe or Vyber.lnk'
} finally { Pop-Location }
