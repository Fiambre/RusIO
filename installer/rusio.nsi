; RusIO installer (per-user, no administrator rights needed).
; Build: makensis /DVERSION=0.1.0 installer\rusio.nsi   (output goes to dist\)
Unicode true
!include "MUI2.nsh"

!ifndef VERSION
  !define VERSION "0.0.0"
!endif

Name "RusIO"
OutFile "..\dist\RusIO-Setup-${VERSION}.exe"
InstallDir "$LOCALAPPDATA\Programs\RusIO"
InstallDirRegKey HKCU "Software\RusIO" "InstallDir"
RequestExecutionLevel user
SetCompressor /SOLID lzma

!define MUI_ICON "..\crates\rusio-app\assets\icon.ico"
!define MUI_UNICON "..\crates\rusio-app\assets\icon.ico"
!define MUI_FINISHPAGE_RUN "$INSTDIR\rusio.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Abrir RusIO"

!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "Spanish"

!define UNKEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\RusIO"

Section "Install"
  SetOutPath "$INSTDIR"
  File "..\target\release\rusio.exe"
  WriteUninstaller "$INSTDIR\Uninstall.exe"

  CreateShortcut "$SMPROGRAMS\RusIO.lnk" "$INSTDIR\rusio.exe"

  WriteRegStr HKCU "Software\RusIO" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "${UNKEY}" "DisplayName" "RusIO"
  WriteRegStr HKCU "${UNKEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNKEY}" "Publisher" "Fiambre"
  WriteRegStr HKCU "${UNKEY}" "DisplayIcon" "$INSTDIR\rusio.exe"
  WriteRegStr HKCU "${UNKEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNKEY}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegDWORD HKCU "${UNKEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNKEY}" "NoRepair" 1
SectionEnd

; Connections (connections.json) and saved passwords are left untouched on purpose.
Section "Uninstall"
  Delete "$INSTDIR\rusio.exe"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\RusIO.lnk"
  DeleteRegKey HKCU "${UNKEY}"
  DeleteRegKey HKCU "Software\RusIO"
SectionEnd
