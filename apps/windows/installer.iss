#ifndef AppVersion
  #error AppVersion is required
#endif
#ifndef SourceDir
  #error SourceDir is required
#endif
#ifndef OutputDir
  #error OutputDir is required
#endif

[Setup]
AppId={{263F7D9C-62D7-4648-93B8-71B2DB01D376}
AppName=FileNode
AppVersion={#AppVersion}
AppPublisher=FileNode
AppPublisherURL=https://github.com/yannickdreher/FileNode
AppSupportURL=https://github.com/yannickdreher/FileNode/issues
DefaultDirName={localappdata}\Programs\FileNode
DefaultGroupName=FileNode
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.19041
OutputDir={#OutputDir}
OutputBaseFilename=filenode-{#AppVersion}-windows-x86_64-setup
SetupIconFile={#SourceDir}\Assets\filenode.ico
UninstallDisplayIcon={app}\filenode-ui.exe
Compression=lzma2
SolidCompression=yes
WizardStyle=modern dynamic
DisableProgramGroupPage=yes
CloseApplications=yes
; The desktop host cooperatively stops its own Rust child before exiting.
; Do not independently send console shutdown to the worker during that cleanup.
CloseApplicationsFilter=filenode-ui.exe
RestartApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "german"; MessagesFile: "compiler:Languages\German.isl"

[Files]
Source: "{#SourceDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\FileNode"; Filename: "{app}\filenode-ui.exe"; WorkingDir: "{app}"

[Run]
Filename: "{app}\filenode-ui.exe"; Description: "{cm:LaunchProgram,FileNode}"; Flags: nowait postinstall skipifsilent

[Code]
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Command: String;
  InstalledExecutable: String;
begin
  if CurUninstallStep = usUninstall then
  begin
    InstalledExecutable := '"' + ExpandConstant('{app}\filenode-ui.exe') + '"';
    if RegQueryStringValue(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Run', 'FileNode', Command) then
      if Pos(Lowercase(InstalledExecutable), Lowercase(Command)) = 1 then
        RegDeleteValue(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Run', 'FileNode');
  end;
end;
