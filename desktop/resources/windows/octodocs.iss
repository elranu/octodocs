; OctoDocs — Inno Setup installer script
; Compile with: ISCC.exe /dAppVersion=0.1.5 /dBinaryPath=..\..\target\release\octodocs-app.exe octodocs.iss
;
; Variables injected by CI (all required):
;   AppVersion    e.g. 0.1.5   (without the leading "v")
;   BinaryPath    path to the compiled octodocs-app.exe

#ifndef AppVersion
  #define AppVersion "dev"
#endif
#ifndef BinaryPath
  #define BinaryPath "..\..\target\release\octodocs-app.exe"
#endif

#define AppName        "OctoDocs"
#define AppPublisher   "elranu"
#define AppURL         "https://github.com/elranu/octodocs"
#define AppExeName     "OctoDocs.exe"
#define AppMutex       "OctoDocs-Instance-Mutex"

[Setup]
AppId={{B3A2F741-1234-4C8D-9E10-ABCDEF012345}
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher={#AppPublisher}
AppPublisherURL={#AppURL}
AppSupportURL={#AppURL}/issues
AppUpdatesURL={#AppURL}/releases
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
AllowNoIcons=yes
PrivilegesRequired=lowest
OutputDir=output
OutputBaseFilename=OctoDocs-Setup-x86_64
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
DisableProgramGroupPage=yes
DisableReadyPage=no
CloseApplications=force
ChangesEnvironment=true
ChangesAssociations=true
AppMutex={#AppMutex}
VersionInfoVersion={#AppVersion}
VersionInfoProductName={#AppName}

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon";    Description: "Create a &desktop shortcut";       GroupDescription: "Additional icons:"; Flags: unchecked
Name: "addtopath";      Description: "Add OctoDocs to &PATH";            GroupDescription: "Other:"
Name: "assocmd";        Description: "Associate .md and .markdown files"; GroupDescription: "Other:"

[Files]
Source: "{#BinaryPath}"; DestDir: "{app}"; DestName: "{#AppExeName}"; Flags: ignoreversion
; Bundle the SVG icon assets next to the executable so the running app can find them
Source: "..\..\crates\octodocs-app\assets\icons\*"; DestDir: "{app}\assets\icons"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{group}\{#AppName}";              Filename: "{app}\{#AppExeName}"
Name: "{group}\Uninstall {#AppName}";    Filename: "{uninstallexe}"
Name: "{autodesktop}\{#AppName}";        Filename: "{app}\{#AppExeName}"; Tasks: desktopicon

[Registry]
; PATH
Root: HKCU; Subkey: "Environment"; ValueType: expandsz; ValueName: "Path"; \
    ValueData: "{app};{olddata}"; Tasks: addtopath; Check: PathNotSet('{app}')

; .md association
Root: HKCU; Subkey: "Software\Classes\.md\OpenWithProgids"; ValueName: "OctoDocs.md"; \
    ValueType: string; ValueData: ""; Flags: uninsdeletevalue; Tasks: assocmd
Root: HKCU; Subkey: "Software\Classes\OctoDocs.md"; ValueType: string; ValueData: "Markdown Document"; \
    Flags: uninsdeletekey; Tasks: assocmd
Root: HKCU; Subkey: "Software\Classes\OctoDocs.md\shell\open\command"; ValueType: string; \
    ValueData: """{app}\{#AppExeName}"" ""%1"""; Tasks: assocmd

; .markdown association
Root: HKCU; Subkey: "Software\Classes\.markdown\OpenWithProgids"; ValueName: "OctoDocs.markdown"; \
    ValueType: string; ValueData: ""; Flags: uninsdeletevalue; Tasks: assocmd
Root: HKCU; Subkey: "Software\Classes\OctoDocs.markdown"; ValueType: string; ValueData: "Markdown Document"; \
    Flags: uninsdeletekey; Tasks: assocmd
Root: HKCU; Subkey: "Software\Classes\OctoDocs.markdown\shell\open\command"; ValueType: string; \
    ValueData: """{app}\{#AppExeName}"" ""%1"""; Tasks: assocmd

[Run]
Filename: "{app}\{#AppExeName}"; Description: "Launch {#AppName}"; \
    Flags: nowait postinstall skipifsilent

[Code]
// Check whether {app} is already in the user PATH
function PathNotSet(const Path: string): Boolean;
var
  CurrentPath: string;
begin
  CurrentPath := '';
  RegQueryStringValue(HKCU, 'Environment', 'Path', CurrentPath);
  Result := Pos(Lowercase(Path), Lowercase(CurrentPath)) = 0;
end;
