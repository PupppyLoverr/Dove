; Early-release installer for Dove.
; Compile after a release build:
;   iscc /DDoveExe=<path-to-dove.exe> packaging\dove.iss

#ifndef DoveExe
  #define DoveExe "..\target\release\dove.exe"
#endif
#ifndef DoveIcon
  #define DoveIcon "..\apps\dove\assets\dove.ico"
#endif

[Setup]
AppId={{7E4B9C2A-1D58-4F63-9A70-6C2E8B15D4F1}
AppName=Dove
AppVersion=0.1.0
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

[Files]
Source: "{#DoveExe}"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\Dove"; Filename: "{app}\dove.exe"
