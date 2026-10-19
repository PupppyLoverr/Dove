; Early-release installer for Dove.
; packaging\windows\package.ps1 stages the binaries and compiles this:
;   iscc /DStage=<folder with dove.exe and sidecars> /DAppVersion=0.1.0 packaging\dove.iss
; or, for dove.exe alone:
;   iscc /DDoveExe=<path-to-dove.exe> packaging\dove.iss

#ifndef DoveExe
  #define DoveExe "..\target\release\dove.exe"
#endif
#ifndef DoveIcon
  #define DoveIcon "..\apps\dove\assets\dove.ico"
#endif
#ifndef AppVersion
  #define AppVersion "0.1.0"
#endif

[Setup]
AppId={{7E4B9C2A-1D58-4F63-9A70-6C2E8B15D4F1}
AppName=Dove
AppVersion={#AppVersion}
AppPublisher=Dove
DefaultDirName={localappdata}\Programs\Dove
DefaultGroupName=Dove
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
OutputDir=out
OutputBaseFilename=Dove-Setup
SetupIconFile={#DoveIcon}
UninstallDisplayIcon={app}\dove.exe
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
; An open Dove holds dove.exe; ask to close it rather than fail the copy.
CloseApplications=yes

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
#ifdef Stage
Source: "{#Stage}\dove.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Stage}\dove-escrow.exe"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist
Source: "{#Stage}\dove-mesh-host.exe"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist
#else
Source: "{#DoveExe}"; DestDir: "{app}"; Flags: ignoreversion
#endif

[Icons]
Name: "{group}\Dove"; Filename: "{app}\dove.exe"
Name: "{autodesktop}\Dove"; Filename: "{app}\dove.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\dove.exe"; Description: "{cm:LaunchProgram,Dove}"; Flags: nowait postinstall skipifsilent
