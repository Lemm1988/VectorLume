# Installiert VectorLume fuer den aktuellen Benutzer (ohne Adminrechte) und legt
# Verknuepfungen auf dem Desktop und im Startmenue an.
#   powershell -ExecutionPolicy Bypass -File tools\install.ps1             (bauen + installieren)
#   powershell -ExecutionPolicy Bypass -File tools\install.ps1 -NoBuild    (nur installieren)
#   powershell -ExecutionPolicy Bypass -File tools\install.ps1 -Uninstall  (entfernen)
param([switch]$NoBuild, [switch]$Uninstall)
$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
$dest = Join-Path $env:LOCALAPPDATA 'Programs\VectorLume'
$exe = Join-Path $dest 'vectorlume.exe'
$links = @(
    (Join-Path ([Environment]::GetFolderPath('Desktop')) 'VectorLume.lnk'),
    (Join-Path ([Environment]::GetFolderPath('Programs')) 'VectorLume.lnk')
)

if (Get-Process vectorlume -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $exe }) {
    throw 'VectorLume laeuft noch - bitte zuerst schliessen.'
}

if ($Uninstall) {
    $links | Where-Object { Test-Path $_ } | Remove-Item
    if (Test-Path $dest) { Remove-Item -Recurse $dest }
    Write-Host 'VectorLume wurde entfernt (Einstellungen in %APPDATA%\VectorLume bleiben erhalten).'
    return
}

if (-not $NoBuild) {
    Push-Location $root
    try {
        cargo build --release
        if ($LASTEXITCODE -ne 0) { throw 'Build fehlgeschlagen.' }
    } finally { Pop-Location }
}

New-Item -ItemType Directory -Force $dest | Out-Null
Copy-Item (Join-Path $root 'target\release\vectorlume.exe') $exe -Force

$shell = New-Object -ComObject WScript.Shell
foreach ($path in $links) {
    $lnk = $shell.CreateShortcut($path)
    $lnk.TargetPath = $exe
    $lnk.WorkingDirectory = $dest
    $lnk.IconLocation = "$exe,0"
    $lnk.Description = 'VectorLume - Laser-Software'
    $lnk.Save()
}
Write-Host "VectorLume installiert: $exe"
Write-Host 'Verknuepfungen: Desktop und Startmenue.'
