; HyperOx-Setup-x64.exe: a per-user installer (no admin) with the same layout as
; distribution/install.ps1: %LOCALAPPDATA%\Programs\h0x-ade, on the user PATH,
; HyperOx in the Start menu. The core is not bundled: the app offers to install
; it on first run (src/setup.rs). Built by CI: iscc /DAppVersion=.. /DSourceDir=.. h0x-ade.iss
#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef SourceDir
  #define SourceDir "..\..\dist\h0x-ade"
#endif

[Setup]
AppId=h0x-ade
AppName=HyperOx
AppVersion={#AppVersion}
AppPublisher=HyperOx
DefaultDirName={localappdata}\Programs\h0x-ade
DefaultGroupName=HyperOx
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\..\dist
OutputBaseFilename=HyperOx-Setup-x64
SetupIconFile=..\..\assets\icons\h0x-ade.ico
UninstallDisplayIcon={app}\h0x-ade.exe
LicenseFile=..\..\LICENSE
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
; PATH is edited in code (raw user value, so %VARIABLES% stay). An open HyperOx is detected (Restart Manager) and the person is asked to close it.
CloseApplications=yes
RestartApplications=no
ChangesEnvironment=yes

[Files]
Source: "{#SourceDir}\h0x-ade.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\h0x-ade.svg"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\THIRD-PARTY-NOTICES"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{userprograms}\HyperOx"; Filename: "{app}\h0x-ade.exe"; WorkingDir: "{%USERPROFILE}"; Comment: "Terminals and coding agents that keep running"

[Run]
Filename: "{app}\h0x-ade.exe"; Description: "Launch HyperOx"; Flags: nowait postinstall skipifsilent

[Code]
const
  EnvKey = 'Environment';

function Listed(Raw, Dir: string): Boolean;
begin
  Result := Pos(';' + Lowercase(Dir) + ';', ';' + Lowercase(Raw) + ';') > 0;
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
  Raw, Dir: string;
begin
  if CurStep <> ssPostInstall then Exit;
  Dir := ExpandConstant('{app}');
  if not RegQueryStringValue(HKCU, EnvKey, 'Path', Raw) then Raw := '';
  if Listed(Raw, Dir) or Listed(Raw, Dir + '\') then Exit;
  if (Raw <> '') and (Raw[Length(Raw)] <> ';') then Raw := Raw + ';';
  Raw := Raw + Dir;
  RegWriteExpandStringValue(HKCU, EnvKey, 'Path', Raw);
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Raw, Dir: string;
  P: Integer;
begin
  if CurUninstallStep <> usPostUninstall then Exit;
  Dir := ExpandConstant('{app}');
  if not RegQueryStringValue(HKCU, EnvKey, 'Path', Raw) then Exit;
  Raw := ';' + Raw + ';';
  P := Pos(';' + Dir + ';', Raw);
  if P = 0 then Exit;
  Delete(Raw, P, Length(Dir) + 1);
  Raw := Copy(Raw, 2, Length(Raw) - 2);
  RegWriteExpandStringValue(HKCU, EnvKey, 'Path', Raw);
end;
