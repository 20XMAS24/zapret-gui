; ZAPRET GUI installer (Inno Setup 6). Built by release.ps1:
;   ISCC.exe /DAppVersion=2.0.0 /DStage=<staging dir> /DOutDir=<output dir> /DRepo=owner/name setup.iss
;
; Staging layout (release.ps1 prepares it):
;   ZapretGUI.exe            - always replaced
;   bin\sing-box.exe         - always replaced
;   TgWsProxy_windows.exe    - only on first install (ZapretGUI updates it later)
;   zapret\...               - official zapret release, only on first install:
;                              afterwards ZapretGUI updates zapret itself and user lists must survive

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef Stage
  #define Stage "..\dist\stage"
#endif
#ifndef OutDir
  #define OutDir "..\dist"
#endif
#ifndef Repo
  #define Repo ""
#endif

[Setup]
AppId={{8E3C2F7A-5B14-4C9D-9A61-3F2E7D0B5C11}
AppName=ZAPRET GUI
AppVersion={#AppVersion}
AppVerName=ZAPRET GUI {#AppVersion}
AppPublisher=ZapretGUI
#if Repo != ""
AppPublisherURL=https://github.com/{#Repo}
AppSupportURL=https://github.com/{#Repo}/issues
AppUpdatesURL=https://github.com/{#Repo}/releases
#endif
VersionInfoVersion={#AppVersion}
DefaultDirName={autopf}\ZapretGUI
DefaultGroupName=ZAPRET GUI
DisableProgramGroupPage=yes
PrivilegesRequired=admin
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputDir={#OutDir}
OutputBaseFilename=ZapretGUI-Setup-{#AppVersion}
SetupIconFile=app.ico
UninstallDisplayIcon={app}\ZapretGUI.exe
UninstallDisplayName=ZAPRET GUI
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
; Files of a running ZapretGUI / winws / TgWsProxy would be locked: close them via Restart Manager.
CloseApplications=force
RestartApplications=no

[Languages]
Name: "ru"; MessagesFile: "compiler:Languages\Russian.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"
Name: "autostart"; Description: "Запускать ZAPRET GUI вместе с Windows"; GroupDescription: "Дополнительно:"; Flags: unchecked

[Files]
Source: "{#Stage}\ZapretGUI.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Stage}\bin\sing-box.exe"; DestDir: "{app}\bin"; Flags: ignoreversion
Source: "{#Stage}\TgWsProxy_windows.exe"; DestDir: "{app}"; Flags: onlyifdoesntexist
Source: "{#Stage}\zapret\*"; DestDir: "{app}"; Flags: onlyifdoesntexist recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\ZAPRET GUI"; Filename: "{app}\ZapretGUI.exe"
Name: "{autodesktop}\ZAPRET GUI"; Filename: "{app}\ZapretGUI.exe"; Tasks: desktopicon

[Run]
; Logon task with highest privileges (no UAC prompt at every logon), same as the in-app switch.
Filename: "{sys}\schtasks.exe"; Parameters: "/Create /F /TN ""ZapretGUI"" /SC ONLOGON /RL HIGHEST /TR ""\""{app}\ZapretGUI.exe\"" -autostart"""; Flags: runhidden; Tasks: autostart
; No skipifsilent: after a silent self-update the app is started again automatically.
Filename: "{app}\ZapretGUI.exe"; Description: "Запустить ZAPRET GUI"; Flags: nowait postinstall

[UninstallRun]
; Stops winws / TgWsProxy / sing-box, removes the kill switch rules and the autostart task.
Filename: "{app}\ZapretGUI.exe"; Parameters: "--cleanup"; Flags: runhidden waituntilterminated; RunOnceId: "ZapretGUICleanup"

[UninstallDelete]
Type: filesandordirs; Name: "{app}"
