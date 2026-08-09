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
; Relative to this script's directory.
SetupIconFile=icon.ico
UninstallDisplayIcon={app}\hookecho.exe

[Files]
Source: "{#ExeDir}\hookecho.exe"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\Hook Echo-WX"; Filename: "{app}\hookecho.exe"
Name: "{autodesktop}\Hook Echo-WX"; Filename: "{app}\hookecho.exe"; Tasks: desktopicon

[Registry]
; Shared hookecho:// links open the app, the way they already do on Android.
Root: HKLM; Subkey: "Software\Classes\hookecho"; ValueType: string; ValueName: ""; ValueData: "URL:Hook Echo-WX"; Flags: uninsdeletekey
Root: HKLM; Subkey: "Software\Classes\hookecho"; ValueType: string; ValueName: "URL Protocol"; ValueData: ""
Root: HKLM; Subkey: "Software\Classes\hookecho\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\hookecho.exe,0"
Root: HKLM; Subkey: "Software\Classes\hookecho\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\hookecho.exe"" ""%1"""

[Tasks]
Name: "desktopicon"; Description: "Create a &desktop shortcut"; Flags: unchecked

[Run]
Filename: "{app}\hookecho.exe"; Description: "Launch Hook Echo-WX"; Flags: nowait postinstall skipifsilent
