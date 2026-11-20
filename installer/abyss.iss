; abyss installer (Inno Setup 6) - built by installer\build.ps1
; Per-user install, no admin prompt. Replaces the old Electron abyss and keeps your settings.

#ifndef AppVersion
  #define AppVersion "0.2.0"
#endif

[Setup]
AppId={{6F1C3B2A-8D4E-4B7A-9C51-0A2B55A2026E}
AppName=abyss
AppVersion={#AppVersion}
AppVerName=abyss {#AppVersion}
AppPublisher=abyss
AppPublisherURL=https://github.com/yumekuse/Abyss-Music
AppSupportURL=https://github.com/yumekuse/Abyss-Music/issues
AppUpdatesURL=https://github.com/yumekuse/Abyss-Music/releases
UsePreviousTasks=yes
DefaultDirName={localappdata}\Programs\abyss
DefaultGroupName=abyss
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\dist
OutputBaseFilename=abyss-setup-{#AppVersion}
SetupIconFile=abyss.ico
UninstallDisplayIcon={app}\abyss.exe
UninstallDisplayName=abyss
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
CloseApplications=force
RestartApplications=no
MinVersion=10.0

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Shortcuts:"
Name: "removeold"; Description: "Remove the old abyss (Electron version) — your settings, likes and queue are kept"; GroupDescription: "Old version:"; Check: OldInstalled

[Files]
Source: "..\target\release\abyss.exe"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\abyss"; Filename: "{app}\abyss.exe"
Name: "{autodesktop}\abyss"; Filename: "{app}\abyss.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\abyss.exe"; Description: "Open abyss"; Flags: nowait postinstall skipifsilent
; auto-update runs this installer silently: open abyss again afterwards
Filename: "{app}\abyss.exe"; Flags: nowait skipifnotsilent; Check: IsAppUpdate

[UninstallRun]
Filename: "{sys}\taskkill.exe"; Parameters: "/F /IM abyss.exe"; Flags: runhidden; RunOnceId: "CloseAbyss"

[UninstallDelete]
; only caches (fonts, saved pages, embedded browser data). Settings in %APPDATA%\abyss stay.
Type: filesandordirs; Name: "{localappdata}\abyss"

[Code]
const
  UninstKey = 'Software\Microsoft\Windows\CurrentVersion\Uninstall';
  WebView2Key = 'SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}';

var
  OldUninstallers: TArrayOfString;

{ every uninstall entry left by the old Electron build (its uninstaller is "Uninstall abyss.exe") }
procedure FindOld();
var
  Keys: TArrayOfString;
  I, N: Integer;
  Name, Cmd: String;
begin
  SetArrayLength(OldUninstallers, 0);
  if not RegGetSubkeyNames(HKCU, UninstKey, Keys) then exit;
  for I := 0 to GetArrayLength(Keys) - 1 do
  begin
    if RegQueryStringValue(HKCU, UninstKey + '\' + Keys[I], 'DisplayName', Name) and
       (Pos('abyss', Lowercase(Name)) = 1) and
       RegQueryStringValue(HKCU, UninstKey + '\' + Keys[I], 'QuietUninstallString', Cmd) and
       (Pos('Uninstall abyss.exe', Cmd) > 0) then
    begin
      N := GetArrayLength(OldUninstallers);
      SetArrayLength(OldUninstallers, N + 1);
      OldUninstallers[N] := Cmd;
    end;
  end;
end;

{ /UPDATE=1 is passed by the app's own updater }
function IsAppUpdate(): Boolean;
begin
  Result := ExpandConstant('{param:UPDATE|0}') = '1';
end;

function OldInstalled(): Boolean;
begin
  Result := GetArrayLength(OldUninstallers) > 0;
end;

function InitializeSetup(): Boolean;
begin
  FindOld();
  Result := True;
end;

function HasWebView2(): Boolean;
var
  V: String;
begin
  Result := (RegQueryStringValue(HKLM, WebView2Key, 'pv', V) and (V <> '') and (V <> '0.0.0.0')) or
            (RegQueryStringValue(HKCU, 'Software\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}', 'pv', V) and (V <> '') and (V <> '0.0.0.0'));
end;

procedure RemoveOld();
var
  I, P, Code: Integer;
  Cmd, Exe, Params: String;
begin
  for I := 0 to GetArrayLength(OldUninstallers) - 1 do
  begin
    Cmd := OldUninstallers[I];
    { "C:\...\Uninstall abyss.exe" /currentuser /S }
    P := Pos('" ', Cmd);
    if (Copy(Cmd, 1, 1) = '"') and (P > 0) then
    begin
      Exe := Copy(Cmd, 2, P - 2);
      Params := Copy(Cmd, P + 2, Length(Cmd));
    end else begin
      Exe := RemoveQuotes(Cmd);
      Params := '/S';
    end;
    if FileExists(Exe) then
    begin
      WizardForm.StatusLabel.Caption := 'Removing the old abyss...';
      { _?= makes the NSIS uninstaller run in place and finish before we continue }
      Exec(Exe, Params + ' _?=' + ExtractFileDir(Exe), '', SW_HIDE, ewWaitUntilTerminated, Code);
      DeleteFile(Exe);
    end;
  end;
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
  Code: Integer;
  Boot: String;
begin
  if CurStep = ssInstall then
  begin
    Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM abyss.exe', '', SW_HIDE, ewWaitUntilTerminated, Code);
    if WizardIsTaskSelected('removeold') then RemoveOld();
  end;
  if (CurStep = ssPostInstall) and not HasWebView2() then
  begin
    WizardForm.StatusLabel.Caption := 'Installing Microsoft Edge WebView2 (needed to draw abyss)...';
    try
      DownloadTemporaryFile('https://go.microsoft.com/fwlink/p/?LinkId=2124703', 'MicrosoftEdgeWebview2Setup.exe', '', nil);
      Boot := ExpandConstant('{tmp}\MicrosoftEdgeWebview2Setup.exe');
      Exec(Boot, '/silent /install', '', SW_HIDE, ewWaitUntilTerminated, Code);
    except
      MsgBox('abyss needs Microsoft Edge WebView2, which could not be downloaded right now.' + #13#10 +
             'Get it from https://developer.microsoft.com/microsoft-edge/webview2/ and open abyss again.', mbInformation, MB_OK);
    end;
  end;
end;
