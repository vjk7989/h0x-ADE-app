; HyperOx-Setup-x64.exe: a per-user installer (no admin) with the same layout as
; distribution/install.ps1: %LOCALAPPDATA%\Programs\h0x-ade, on the user PATH,
; HyperOx in the Start menu. It also carries h0x-core (the release's pinned
; version, signature-checked by CI) into %LOCALAPPDATA%\Programs\h0x-core, on the
; PATH, as install.ps1 does, so the app works at once and offline. A core that is
; already there or on the PATH is left alone (the app updates it, running
; terminals kept), so a running core is never touched.
; Built by CI: iscc /DAppVersion=.. /DSourceDir=.. /DCoreDir=.. h0x-ade.iss
#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef SourceDir
  #define SourceDir "..\..\dist\h0x-ade"
#endif
#ifndef CoreDir
  #define CoreDir "..\..\dist\h0x-core"
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
; An open HyperOx is detected (Restart Manager) and the person is asked to close it.
CloseApplications=yes
RestartApplications=no
ChangesEnvironment=yes

[Files]
Source: "{#SourceDir}\h0x-ade.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\h0x-ade.svg"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\THIRD-PARTY-NOTICES"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#CoreDir}\h0x-core.exe"; DestDir: "{localappdata}\Programs\h0x-core"; Flags: ignoreversion; Check: InstallCore
Source: "{#CoreDir}\conpty\*"; DestDir: "{localappdata}\Programs\h0x-core\conpty"; Flags: ignoreversion recursesubdirs; Check: InstallCore
Source: "{#CoreDir}\THIRD-PARTY-NOTICES\*"; DestDir: "{localappdata}\Programs\h0x-core\THIRD-PARTY-NOTICES"; Flags: ignoreversion recursesubdirs skipifsourcedoesntexist; Check: InstallCore

[Icons]
Name: "{userprograms}\HyperOx"; Filename: "{app}\h0x-ade.exe"; WorkingDir: "{%USERPROFILE}"; Comment: "Terminals and coding agents that keep running"

[Run]
Filename: "{app}\h0x-ade.exe"; Description: "Launch HyperOx"; Flags: nowait postinstall skipifsilent

[Code]
const
  EnvKey = 'Environment';

var
  NewCore: Boolean;

function CoreDirPath: string;
begin
  Result := ExpandConstant('{localappdata}\Programs\h0x-core');
end;

// Decided once, before any file is copied (the first copy would change the answer).
function InitializeSetup: Boolean;
begin
  NewCore := not FileExists(CoreDirPath + '\h0x-core.exe') and (FileSearch('h0x-core.exe', GetEnv('PATH')) = '');
  Result := True;
end;

function InstallCore: Boolean;
begin
  Result := NewCore;
end;

function Listed(Raw, Dir: string): Boolean;
begin
  Result := (Pos(';' + Lowercase(Dir) + ';', ';' + Lowercase(Raw) + ';') > 0)
    or (Pos(';' + Lowercase(Dir) + '\;', ';' + Lowercase(Raw) + ';') > 0);
end;

// The raw user PATH, so %VARIABLES% in it stay unexpanded.
procedure AddToPath(Dir: string);
var
  Raw: string;
begin
  if not RegQueryStringValue(HKCU, EnvKey, 'Path', Raw) then Raw := '';
  if Listed(Raw, Dir) then Exit;
  if (Raw <> '') and (Raw[Length(Raw)] <> ';') then Raw := Raw + ';';
  RegWriteExpandStringValue(HKCU, EnvKey, 'Path', Raw + Dir);
end;

procedure RemoveFromPath(Dir: string);
var
  Raw: string;
  P: Integer;
begin
  if not RegQueryStringValue(HKCU, EnvKey, 'Path', Raw) then Exit;
  Raw := ';' + Raw + ';';
  P := Pos(';' + Dir + ';', Raw);
  if P = 0 then Exit;
  Delete(Raw, P, Length(Dir) + 1);
  RegWriteExpandStringValue(HKCU, EnvKey, 'Path', Copy(Raw, 2, Length(Raw) - 2));
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep <> ssPostInstall then Exit;
  AddToPath(ExpandConstant('{app}'));
  if NewCore then AddToPath(CoreDirPath);
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep <> usPostUninstall then Exit;
  RemoveFromPath(ExpandConstant('{app}'));
  // Only if the uninstall took the core away (one that was already there stays).
  if not FileExists(CoreDirPath + '\h0x-core.exe') then RemoveFromPath(CoreDirPath);
end;
