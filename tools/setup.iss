; VectorLume – Setup für den aktuellen Benutzer (ohne Adminrechte), erzeugt mit Inno Setup 6.
; Nicht direkt übersetzen, sondern über tools\package.ps1 (setzt AppVersion und Stage).
; Gleicher Zielordner und gleiche Dateitypen wie tools\install.ps1, eine vorhandene Installation wird ersetzt.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef Stage
  #define Stage "..\dist\stage"
#endif

[Setup]
AppId={{8C1F4E2A-5B7D-4E9A-9F3C-2D6A1B0E7C45}
AppName=VectorLume
AppVersion={#AppVersion}
AppVerName=VectorLume {#AppVersion}
AppPublisher=VectorLume
VersionInfoVersion={#AppVersion}
VersionInfoDescription=VectorLume Setup
DefaultDirName={localappdata}\Programs\VectorLume
DisableDirPage=auto
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
OutputDir=..\dist
OutputBaseFilename=VectorLume-Setup-{#AppVersion}
SetupIconFile={#Stage}\vectorlume.ico
UninstallDisplayIcon={app}\vectorlume.exe
UninstallDisplayName=VectorLume
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
ChangesAssociations=yes
CloseApplications=yes
ShowLanguageDialog=auto

[Languages]
Name: "de"; MessagesFile: "compiler:Languages\German.isl"
Name: "en"; MessagesFile: "compiler:Default.isl"

[CustomMessages]
de.OpenWithLightBurn=VectorLume unter „Öffnen mit“ für LightBurn-Projekte (.lbrn2) anbieten
en.OpenWithLightBurn=Offer VectorLume under "Open with" for LightBurn projects (.lbrn2)
de.FileTypes=Dateitypen:
en.FileTypes=File types:
de.ProjectType=VectorLume-Projekt
en.ProjectType=VectorLume project
de.LightBurnType=LightBurn-Projekt
en.LightBurnType=LightBurn project

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"
Name: "lbrn2"; Description: "{cm:OpenWithLightBurn}"; GroupDescription: "{cm:FileTypes}"

[Files]
Source: "{#Stage}\vectorlume.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Stage}\THIRD-PARTY-LICENSES.html"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\VectorLume"; Filename: "{app}\vectorlume.exe"; WorkingDir: "{app}"; Comment: "VectorLume"
Name: "{autodesktop}\VectorLume"; Filename: "{app}\vectorlume.exe"; WorkingDir: "{app}"; Comment: "VectorLume"; Tasks: desktopicon

[Registry]
; .vlume öffnet per Doppelklick mit VectorLume
Root: HKA; Subkey: "Software\Classes\.vlume"; ValueType: string; ValueName: ""; ValueData: "VectorLume.Project"; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Classes\VectorLume.Project"; ValueType: string; ValueName: ""; ValueData: "{cm:ProjectType}"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\VectorLume.Project\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: """{app}\vectorlume.exe"",0"
Root: HKA; Subkey: "Software\Classes\VectorLume.Project\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\vectorlume.exe"" ""%1"""
; .lbrn2 nur unter „Öffnen mit“ – die Zuordnung zu LightBurn bleibt unverändert
Root: HKA; Subkey: "Software\Classes\VectorLume.LightBurn"; ValueType: string; ValueName: ""; ValueData: "{cm:LightBurnType}"; Flags: uninsdeletekey; Tasks: lbrn2
Root: HKA; Subkey: "Software\Classes\VectorLume.LightBurn\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: """{app}\vectorlume.exe"",0"; Tasks: lbrn2
Root: HKA; Subkey: "Software\Classes\VectorLume.LightBurn\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\vectorlume.exe"" ""%1"""; Tasks: lbrn2
Root: HKA; Subkey: "Software\Classes\.lbrn2\OpenWithProgids"; ValueType: string; ValueName: "VectorLume.LightBurn"; ValueData: ""; Flags: uninsdeletevalue; Tasks: lbrn2

[Run]
Filename: "{app}\vectorlume.exe"; Description: "{cm:LaunchProgram,VectorLume}"; Flags: nowait postinstall skipifsilent
