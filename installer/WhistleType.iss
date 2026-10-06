; WhistleType installer (Inno Setup 6).
; Build: scripts\build-release.ps1  (passes /DAppVersion=... and expects dist\app\ to be staged)
;
; Per-user install by default (no administrator rights, no UAC prompt) into %LOCALAPPDATA%\Programs\WhistleType.
; The Whistle model is NOT bundled: the app downloads it once on first run (pinned revision + SHA-256),
; or the user can import it from a file.

#ifndef AppVersion
  #define AppVersion "1.1.0"
#endif
#define AppName "WhistleType"
#define AppExe "WhistleType.exe"

[Setup]
AppId={{6E0D5F2B-8C1A-4B7E-9F3C-57A1D2E9C4B1}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher=apkmasondev
AppPublisherURL=https://github.com/apkmasondev/whistle-type
AppSupportURL=https://github.com/apkmasondev/whistle-type/issues
AppUpdatesURL=https://github.com/apkmasondev/whistle-type/releases
AppComments=Local push-to-talk dictation for Windows (Whistle + Whisper, runs offline)
VersionInfoVersion={#AppVersion}
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.17763
OutputDir=..\dist
OutputBaseFilename=WhistleType-{#AppVersion}-setup-x64
SetupIconFile=..\res\icons\app.ico
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName}
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes
CloseApplicationsFilter=*.exe
RestartApplications=no
LicenseFile=..\dist\app\LICENSE.txt

[Languages]
Name: "en"; MessagesFile: "compiler:Default.isl"
Name: "pl"; MessagesFile: "compiler:Languages\Polish.isl"

[CustomMessages]
en.AutostartTask=Start WhistleType when I sign in to Windows
pl.AutostartTask=Uruchamiaj WhistleType po zalogowaniu do Windows
en.OptionsGroup=Options:
pl.OptionsGroup=Opcje:
en.DeleteUserData=Also delete your WhistleType settings, logs and the downloaded speech model?
pl.DeleteUserData=Usunąć także ustawienia WhistleType, logi i pobrany model mowy?

[Tasks]
Name: "autostart"; Description: "{cm:AutostartTask}"; GroupDescription: "{cm:OptionsGroup}"
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "..\dist\app\WhistleType.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\dist\app\libneedle3.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\dist\app\whisper-cpu\*"; DestDir: "{app}\whisper-cpu"; Flags: ignoreversion
Source: "..\dist\app\THIRD_PARTY_NOTICES.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\dist\app\LICENSE.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\dist\app\LICENSES\*"; DestDir: "{app}\LICENSES"; Flags: ignoreversion recursesubdirs
Source: "..\dist\app\README.txt"; DestDir: "{app}"; Flags: ignoreversion isreadme

[Icons]
Name: "{group}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{group}\Uninstall {#AppName}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Registry]
; Same value the app writes from Settings → "Start with Windows"
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "WhistleType"; ValueData: """{app}\{#AppExe}"" --background"; Tasks: autostart; Flags: uninsdeletevalue

[Run]
Filename: "{app}\{#AppExe}"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall skipifsilent

[UninstallRun]
Filename: "{cmd}"; Parameters: "/C taskkill /IM {#AppExe} /F"; Flags: runhidden; RunOnceId: "StopApp"

[UninstallDelete]
; the Run value is also written by the app itself (Settings), remove it in any case
Type: files; Name: "{app}\*.log"

[Code]
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Data: String;
begin
  if CurUninstallStep = usPostUninstall then
  begin
    RegDeleteValue(HKEY_CURRENT_USER, 'Software\Microsoft\Windows\CurrentVersion\Run', 'WhistleType');
    if not UninstallSilent then
    begin
      if MsgBox(CustomMessage('DeleteUserData') + #13#10#13#10 +
                ExpandConstant('{userappdata}\WhistleType') + #13#10 + ExpandConstant('{localappdata}\WhistleType'),
                mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES then
      begin
        Data := ExpandConstant('{userappdata}\WhistleType');
        DelTree(Data, True, True, True);
        Data := ExpandConstant('{localappdata}\WhistleType');
        DelTree(Data, True, True, True);
      end;
    end;
  end;
end;
