#ifndef AppVersion
  #define AppVersion "0.1.0"
#endif

[Setup]
AppId={{A5C4F9B8-2F13-4F07-8BA2-82FB3EB6B041}
AppName=Tyin-Editor
AppVersion={#AppVersion}
AppPublisher=Tyin-Editor
DefaultDirName={localappdata}\Programs\Tyin-Editor
DefaultGroupName=Tyin-Editor
UninstallDisplayIcon={app}\tyin-app.exe
PrivilegesRequired=lowest
ArchitecturesAllowed=x64
ArchitecturesInstallIn64BitMode=x64
DisableProgramGroupPage=yes
OutputDir=..\..\dist
OutputBaseFilename=Tyin-Editor-Setup-{#AppVersion}-x64
Compression=lzma2
SolidCompression=yes
WizardStyle=modern

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Additional shortcuts:"; Flags: unchecked

[Dirs]
Name: "{userappdata}\Tyin-Editor"; Flags: uninsneveruninstall

[Files]
Source: "..\..\target\release\tyin-app.exe"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Tyin-Editor"; Filename: "{app}\tyin-app.exe"; WorkingDir: "{userappdata}\Tyin-Editor"
Name: "{autodesktop}\Tyin-Editor"; Filename: "{app}\tyin-app.exe"; WorkingDir: "{userappdata}\Tyin-Editor"; Tasks: desktopicon

[Run]
Filename: "{app}\tyin-app.exe"; Description: "Launch Tyin-Editor"; Flags: postinstall nowait skipifsilent
