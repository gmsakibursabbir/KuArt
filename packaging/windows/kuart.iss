; Inno Setup installer (KuArt-X.Y.Z-x64-setup.exe). Per-user by default, no admin needed.
; Built by release.yml: iscc /DVersion=X.Y.Z /DBinDir=<dir> /DOutDir=<dir> packaging\windows\kuart.iss
#ifndef Version
  #define Version "0.0.0"
#endif

[Setup]
AppId={{6A1C5E2B-9F3D-4B7A-8C21-0D4E5F6A7B8C}
AppName=KuArt
AppVersion={#Version}
AppPublisher=KuArt
AppPublisherURL=https://github.com/gmsakibursabbir/KuArt
DefaultDirName={autopf}\KuArt
DefaultGroupName=KuArt
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir={#OutDir}
OutputBaseFilename=KuArt-{#Version}-x64-setup
UninstallDisplayIcon={app}\design-editor.exe
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ChangesAssociations=yes

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked
Name: "associate"; Description: "Open .krt, .fig, .xd and .psd files with KuArt"; GroupDescription: "File associations:"

[Files]
Source: "{#BinDir}\design-editor.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#BinDir}\design-editor-cli.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\LICENSE-MIT"; DestDir: "{app}"
Source: "..\..\LICENSE-APACHE"; DestDir: "{app}"

[Icons]
Name: "{group}\KuArt"; Filename: "{app}\design-editor.exe"
Name: "{group}\Uninstall KuArt"; Filename: "{uninstallexe}"
Name: "{autodesktop}\KuArt"; Filename: "{app}\design-editor.exe"; Tasks: desktopicon

[Registry]
Root: HKA; Subkey: "Software\Classes\KuArt.Design"; ValueType: string; ValueData: "KuArt design"; Flags: uninsdeletekey; Tasks: associate
Root: HKA; Subkey: "Software\Classes\KuArt.Design\DefaultIcon"; ValueType: string; ValueData: "{app}\design-editor.exe,0"; Tasks: associate
Root: HKA; Subkey: "Software\Classes\KuArt.Design\shell\open\command"; ValueType: string; ValueData: """{app}\design-editor.exe"" ""%1"""; Tasks: associate
Root: HKA; Subkey: "Software\Classes\.krt\OpenWithProgids"; ValueType: string; ValueName: "KuArt.Design"; ValueData: ""; Flags: uninsdeletevalue; Tasks: associate
Root: HKA; Subkey: "Software\Classes\.design\OpenWithProgids"; ValueType: string; ValueName: "KuArt.Design"; ValueData: ""; Flags: uninsdeletevalue; Tasks: associate
Root: HKA; Subkey: "Software\Classes\.fig\OpenWithProgids"; ValueType: string; ValueName: "KuArt.Design"; ValueData: ""; Flags: uninsdeletevalue; Tasks: associate
Root: HKA; Subkey: "Software\Classes\.xd\OpenWithProgids"; ValueType: string; ValueName: "KuArt.Design"; ValueData: ""; Flags: uninsdeletevalue; Tasks: associate
Root: HKA; Subkey: "Software\Classes\.psd\OpenWithProgids"; ValueType: string; ValueName: "KuArt.Design"; ValueData: ""; Flags: uninsdeletevalue; Tasks: associate

[Run]
Filename: "{app}\design-editor.exe"; Description: "{cm:LaunchProgram,KuArt}"; Flags: nowait postinstall skipifsilent
