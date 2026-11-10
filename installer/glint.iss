; Instalador do Glint (Inno Setup 6). Gerado pelo scripts\release.ps1:
;   ISCC /DAppVersion=1.0.0-rc.3 /DSrcDir=..\dist\Glint installer\glint.iss
; Instala por usuário (sem administrador) em %LOCALAPPDATA%\Programs\Glint.
; Clipes e configurações ficam fora da pasta do programa e sobrevivem a desinstalar/atualizar.
#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef SrcDir
  #define SrcDir "..\dist\Glint"
#endif
#define AppName "Glint"
#define AppExe "glint.exe"
#define RecExe "glint-recorder.exe"

[Setup]
AppId={{6A1C4E2B-3F7D-4B8A-9C21-5E0D7A9B1F42}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher={#AppName}
DefaultDirName={localappdata}\Programs\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
DisableDirPage=yes
PrivilegesRequired=lowest
OutputDir=..\dist
OutputBaseFilename=Glint-Setup-{#AppVersion}
SetupIconFile=..\assets\brand\glint.ico
UninstallDisplayIcon={app}\{#AppExe}
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "pt"; MessagesFile: "compiler:Languages\BrazilianPortuguese.isl"

[Tasks]
Name: "desktopicon"; Description: "Criar atalho na Área de Trabalho"; Flags: unchecked

[Files]
Source: "{#SrcDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Run]
; O gravador se registra para iniciar com o Windows (config start_with_os, padrão ligado).
Filename: "{app}\{#RecExe}"; Flags: nowait runasoriginaluser
Filename: "{app}\{#AppExe}"; Description: "Abrir o {#AppName}"; Flags: nowait postinstall runasoriginaluser

[UninstallRun]
Filename: "{app}\{#RecExe}"; Parameters: "--quit"; Flags: runhidden waituntilterminated; RunOnceId: "QuitRecorder"

[Registry]
; Desinstalar tira o "iniciar com o Windows" que o gravador criou.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: none; ValueName: "{#AppName}"; Flags: uninsdeletevalue dontcreatekey
; Links glint:// (botão "Abrir no Glint" da página de convite): o app recebe o link como argumento
; e, se já estiver aberto, passa para a janela aberta.
Root: HKCU; Subkey: "Software\Classes\glint"; ValueType: string; ValueName: ""; ValueData: "URL:{#AppName}"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\glint"; ValueType: string; ValueName: "URL Protocol"; ValueData: ""
Root: HKCU; Subkey: "Software\Classes\glint\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"",0"
Root: HKCU; Subkey: "Software\Classes\glint\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"" ""%1"""

[Code]
// Antes de copiar: fecha o gravador pelo IPC (ele segura as DLLs) e o app.
function PrepareToInstall(var NeedsRestart: Boolean): String;
var Code: Integer;
begin
  if FileExists(ExpandConstant('{app}\{#RecExe}')) then
    Exec(ExpandConstant('{app}\{#RecExe}'), '--quit', '', SW_HIDE, ewWaitUntilTerminated, Code);
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM {#AppExe}', '', SW_HIDE, ewWaitUntilTerminated, Code);
  Sleep(800);
  Result := '';
end;
