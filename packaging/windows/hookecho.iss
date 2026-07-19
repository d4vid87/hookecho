; Inno Setup script: classic setup.exe for Hook Echo-WX.
; CI passes /DAppVersion=X.Y.Z and /DExeDir=path\to\dir-containing-hookecho.exe.

[Setup]
AppId={{c5f4f6f0-52c0-4b6e-9d3e-2c8f6f7d1b11}
AppName=Hook Echo-WX
AppVersion={#AppVersion}
AppPublisher=Hook Echo-WX project
DefaultDirName={autopf}\Hook Echo-WX
DefaultGroupName=Hook Echo-WX
DisableProgramGroupPage=yes
OutputBaseFilename=Hook_Echo-WX-setup-x86_64
Compression=lzma2
SolidCompression=yes
ArchitecturesInstallIn64BitMode=x64compatible
WizardStyle=modern

[Files]
Source: "{#ExeDir}\hookecho.exe"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\Hook Echo-WX"; Filename: "{app}\hookecho.exe"
Name: "{autodesktop}\Hook Echo-WX"; Filename: "{app}\hookecho.exe"; Tasks: desktopicon

[Tasks]
Name: "desktopicon"; Description: "Create a &desktop shortcut"; Flags: unchecked

[Run]
Filename: "{app}\hookecho.exe"; Description: "Launch Hook Echo-WX"; Flags: nowait postinstall skipifsilent
