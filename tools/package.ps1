# Erzeugt die Pakete zur Weitergabe in dist\:
#   VectorLume-Setup-<Version>.exe   Setup fuer den aktuellen Benutzer (ohne Adminrechte, mit Deinstallation)
#   VectorLume-<Version>-portable.zip  nur Programm und Lizenzliste, ohne Installation
#   powershell -ExecutionPolicy Bypass -File tools\package.ps1            (bauen + paketieren)
#   powershell -ExecutionPolicy Bypass -File tools\package.ps1 -NoBuild   (vorhandenen Release-Build verwenden)
# Benoetigt Inno Setup 6 (winget install JRSoftware.InnoSetup) und cargo-about (cargo install cargo-about --locked).
param([switch]$NoBuild)
$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
$dist = Join-Path $root 'dist'
$stage = Join-Path $dist 'stage'

$match = Select-String -Path (Join-Path $root 'Cargo.toml') -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1
if (-not $match) { throw 'Version in Cargo.toml nicht gefunden.' }
$version = $match.Matches[0].Groups[1].Value

$iscc = @(
    (Get-Command ISCC.exe -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Source),
    (Join-Path $env:LOCALAPPDATA 'Programs\Inno Setup 6\ISCC.exe'),
    (Join-Path ${env:ProgramFiles(x86)} 'Inno Setup 6\ISCC.exe'),
    (Join-Path $env:ProgramFiles 'Inno Setup 6\ISCC.exe')
) | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
if (-not $iscc) { throw 'Inno Setup 6 nicht gefunden (winget install JRSoftware.InnoSetup).' }

Push-Location $root
try {
    if (-not $NoBuild) {
        cargo build --release
        if ($LASTEXITCODE -ne 0) { throw 'Build fehlgeschlagen.' }
    }
    if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
    New-Item -ItemType Directory -Force $stage | Out-Null
    Copy-Item 'target\release\vectorlume.exe' $stage

    # Das Programmsymbol erzeugt app\build.rs beim Bauen
    $ico = Get-ChildItem 'target\release\build' -Filter 'vectorlume.ico' -Recurse |
        Sort-Object LastWriteTime -Descending | Select-Object -First 1
    if (-not $ico) { throw 'Programmsymbol nicht gefunden - bitte ohne -NoBuild ausfuehren.' }
    Copy-Item $ico.FullName (Join-Path $stage 'vectorlume.ico')

    # Lizenztexte aller mitgelieferten Bibliotheken (MIT/Apache usw. verlangen sie bei der Weitergabe)
    $licenses = Join-Path $stage 'THIRD-PARTY-LICENSES.html'
    cargo about generate --config 'tools\about.toml' --output-file $licenses --fail 'tools\about.hbs'
    if ($LASTEXITCODE -ne 0) { throw 'Lizenzliste fehlgeschlagen.' }

    & $iscc /Qp "/DAppVersion=$version" "/DStage=$stage" 'tools\setup.iss'
    if ($LASTEXITCODE -ne 0) { throw 'Setup fehlgeschlagen.' }

    $zip = Join-Path $dist "VectorLume-$version-portable.zip"
    if (Test-Path $zip) { Remove-Item $zip }
    Compress-Archive -Path (Join-Path $stage 'vectorlume.exe'), $licenses -DestinationPath $zip
} finally {
    Pop-Location
}
Write-Host "Setup:    $(Join-Path $dist "VectorLume-Setup-$version.exe")"
Write-Host "Portabel: $zip"
