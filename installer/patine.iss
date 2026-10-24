; Windows installer of Patine (Inno Setup 6). Saved as UTF-8 with a BOM, which Inno Setup
; needs to read accented text.
;
; Built by .github/workflows/release.yml:
;   iscc /DAppVersion=0.1.0 /DSourceDir=<staging folder> /DOutputDir=<folder> installer\patine.iss
;
; The staging folder holds what the installer copies: patine_app.exe, patine_cli.exe,
; assets\environments\, the licenses and README.md.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef SourceDir
  #define SourceDir "..\dist\Patine"
#endif
#ifndef OutputDir
  #define OutputDir "..\dist"
#endif

#define AppName "Patine"
#define AppExe "patine_app.exe"
#define AppUrl "https://github.com/Farontir/platine"

[Setup]
; Stable identifier: upgrades replace the installed version.
AppId={{D1DAC1DB-1240-47F4-935F-A71163436C31}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher=Delbarre Jérémy
AppPublisherURL={#AppUrl}
AppSupportURL={#AppUrl}/issues
AppUpdatesURL={#AppUrl}/releases
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
; Per-user install by default (no administrator rights); for all users on request.
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputDir={#OutputDir}
OutputBaseFilename=Patine-{#AppVersion}-windows-x64-setup
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ChangesAssociations=yes
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName} {#AppVersion}

[Languages]
Name: "french"; MessagesFile: "compiler:Languages\French.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[CustomMessages]
french.AssociateProjects=Ouvrir les projets .patine avec Patine
english.AssociateProjects=Open .patine projects with Patine
french.ProjectFile=Projet Patine
english.ProjectFile=Patine project

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked
Name: "association"; Description: "{cm:AssociateProjects}"; GroupDescription: "{cm:AdditionalIcons}"

[Files]
Source: "{#SourceDir}\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\patine_cli.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\assets\*"; DestDir: "{app}\assets"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#SourceDir}\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\LICENSE-MIT"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\LICENSE-APACHE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\THIRD-PARTY-LICENSES.txt"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Registry]
Root: HKA; Subkey: "Software\Classes\.patine"; ValueType: string; ValueName: ""; ValueData: "Patine.Project"; Flags: uninsdeletevalue uninsdeletekeyifempty; Tasks: association
Root: HKA; Subkey: "Software\Classes\Patine.Project"; ValueType: string; ValueName: ""; ValueData: "{cm:ProjectFile}"; Flags: uninsdeletekey; Tasks: association
Root: HKA; Subkey: "Software\Classes\Patine.Project\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\{#AppExe},0"; Tasks: association
Root: HKA; Subkey: "Software\Classes\Patine.Project\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"" ""%1"""; Tasks: association

[Run]
Filename: "{app}\{#AppExe}"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall skipifsilent
