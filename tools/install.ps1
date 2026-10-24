# Installiert VectorLume fuer den aktuellen Benutzer (ohne Adminrechte), legt Verknuepfungen auf dem
# Desktop und im Startmenue an und meldet die Dateitypen an: .vlume oeffnet per Doppelklick mit VectorLume,
# fuer LightBurn-Projekte (.lbrn2) erscheint VectorLume nur unter "Oeffnen mit" (bestehende Zuordnung bleibt).
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

$classes = 'HKCU:\Software\Classes'

# Schluessel nur anlegen, wenn er fehlt - New-Item -Force wuerde einen vorhandenen Schluessel leeren.
function Set-KeyDefault([string]$key, [string]$value) {
    if (-not (Test-Path $key)) { New-Item -Path $key -Force | Out-Null }
    Set-Item -Path $key -Value $value
}

function Update-ShellAssociations {
    Add-Type -Namespace VectorLume -Name Shell -MemberDefinition @'
[System.Runtime.InteropServices.DllImport("shell32.dll")]
public static extern void SHChangeNotify(int eventId, uint flags, System.IntPtr item1, System.IntPtr item2);
'@
    [VectorLume.Shell]::SHChangeNotify(0x08000000, 0, [IntPtr]::Zero, [IntPtr]::Zero)
}

function Register-FileTypes {
    $open = "`"$exe`" `"%1`""
    Set-KeyDefault "$classes\VectorLume.Project" 'VectorLume-Projekt'
    Set-KeyDefault "$classes\VectorLume.Project\DefaultIcon" "`"$exe`",0"
    Set-KeyDefault "$classes\VectorLume.Project\shell\open\command" $open
    Set-KeyDefault "$classes\.vlume" 'VectorLume.Project'
    Set-KeyDefault "$classes\VectorLume.LightBurn" 'LightBurn-Projekt'
    Set-KeyDefault "$classes\VectorLume.LightBurn\DefaultIcon" "`"$exe`",0"
    Set-KeyDefault "$classes\VectorLume.LightBurn\shell\open\command" $open
    $with = "$classes\.lbrn2\OpenWithProgids"
    if (-not (Test-Path $with)) { New-Item -Path $with -Force | Out-Null }
    New-ItemProperty -Path $with -Name 'VectorLume.LightBurn' -Value '' -PropertyType String -Force | Out-Null
    Update-ShellAssociations
}

function Unregister-FileTypes {
    foreach ($key in "$classes\VectorLume.Project", "$classes\VectorLume.LightBurn") {
        if (Test-Path $key) { Remove-Item -Path $key -Recurse }
    }
    if ((Test-Path "$classes\.vlume") -and (Get-Item "$classes\.vlume").GetValue('') -eq 'VectorLume.Project') {
        Remove-Item -Path "$classes\.vlume" -Recurse
    }
    $with = "$classes\.lbrn2\OpenWithProgids"
    if (Test-Path $with) { Remove-ItemProperty -Path $with -Name 'VectorLume.LightBurn' -ErrorAction SilentlyContinue }
    Update-ShellAssociations
}

if (Get-Process vectorlume -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $exe }) {
    throw 'VectorLume laeuft noch - bitte zuerst schliessen.'
}

if ($Uninstall) {
    Unregister-FileTypes
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
Register-FileTypes
Write-Host "VectorLume installiert: $exe"
Write-Host 'Verknuepfungen: Desktop und Startmenue. Dateitypen: .vlume, .lbrn2 (unter "Oeffnen mit").'
