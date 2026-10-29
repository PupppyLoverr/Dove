; Early-release installer for Keplorayx (K22).
; packaging\windows\package.ps1 stages the binaries and compiles this:
;   iscc /DStage=<folder with dove.exe and sidecars> /DAppVersion=0.1.0 packaging\dove.iss
; or, for dove.exe alone:
;   iscc /DDoveExe=<path-to-dove.exe> packaging\dove.iss

#ifndef DoveExe
  #define DoveExe "..\target\release\dove.exe"
#endif
#ifndef DoveIcon
  #define DoveIcon "..\apps\dove\assets\keplorayx.ico"
#endif
#ifndef AppVersion
  #define AppVersion "0.1.0"
#endif
; Beside dove.exe ship dove-escrow.exe (the tiny suspended helper each agent
; spawn uses on Windows), DirectML.dll and the remote-control host.
#ifdef Stage
  #define DoveDir Stage + "\"
#else
  #define DoveDir ExtractFilePath(DoveExe)
#endif

[Setup]
AppId={{7E4B9C2A-1D58-4F63-9A70-6C2E8B15D4F1}
AppName=Keplorayx
AppVersion={#AppVersion}
AppPublisher=Keplorayx
DefaultDirName={localappdata}\Programs\Keplorayx
DefaultGroupName=Keplorayx
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
OutputDir=out
OutputBaseFilename=Keplorayx-Setup
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
#else
Source: "{#DoveExe}"; DestDir: "{app}"; Flags: ignoreversion
#endif
Source: "{#DoveDir}dove-escrow.exe"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist
Source: "{#DoveDir}DirectML.dll"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist
Source: "{#DoveDir}dove-mesh-host.exe"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist

[Icons]
Name: "{group}\Keplorayx"; Filename: "{app}\dove.exe"
Name: "{autodesktop}\Keplorayx"; Filename: "{app}\dove.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\dove.exe"; Description: "{cm:LaunchProgram,Dove}"; Flags: nowait postinstall skipifsilent
