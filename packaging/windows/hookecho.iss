; Inno Setup script: classic setup.exe for HookEcho.
; CI passes /DAppVersion=X.Y.Z and /DExeDir=path\to\dir-containing-hookecho.exe.

[Setup]
AppId={{c5f4f6f0-52c0-4b6e-9d3e-2c8f6f7d1b11}
AppName=HookEcho
AppVersion={#AppVersion}
AppPublisher=HookEcho project
DefaultDirName={autopf}\HookEcho
DefaultGroupName=HookEcho
DisableProgramGroupPage=yes
OutputBaseFilename=HookEcho-setup-x86_64
Compression=lzma2
SolidCompression=yes
ArchitecturesInstallIn64BitMode=x64compatible
WizardStyle=modern
; Relative to this script's directory.
SetupIconFile=icon.ico
UninstallDisplayIcon={app}\hookecho.exe

[Files]
Source: "{#ExeDir}\hookecho.exe"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\HookEcho"; Filename: "{app}\hookecho.exe"
Name: "{autodesktop}\HookEcho"; Filename: "{app}\hookecho.exe"; Tasks: desktopicon

[Registry]
; Shared hookecho:// links open the app, the way they already do on Android.
Root: HKLM; Subkey: "Software\Classes\hookecho"; ValueType: string; ValueName: ""; ValueData: "URL:HookEcho"; Flags: uninsdeletekey
Root: HKLM; Subkey: "Software\Classes\hookecho"; ValueType: string; ValueName: "URL Protocol"; ValueData: ""
Root: HKLM; Subkey: "Software\Classes\hookecho\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\hookecho.exe,0"
Root: HKLM; Subkey: "Software\Classes\hookecho\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\hookecho.exe"" ""%1"""

[Tasks]
Name: "desktopicon"; Description: "Create a &desktop shortcut"; Flags: unchecked

[Run]
Filename: "{app}\hookecho.exe"; Description: "Launch HookEcho"; Flags: nowait postinstall skipifsilent
